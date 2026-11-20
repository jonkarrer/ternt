//! A run: request the model, run the tools it asks for, again, until it answers.

use std::sync::Arc;

use anyhow::{Result, bail};
use context::{Block, Message, Role};
use engine::{Next, Policy, Runtime, Task, TaskId, TaskOptions, Tx};
use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolCallInput, ToolOutput};
use crate::{Mind, Model, Settings, Settled, Work, apply, inbox, on_copy};

pub(crate) const NAME: &str = "agent.generation";

pub struct Generation<M> {
    model: Arc<M>,
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
}

impl<M> Generation<M> {
    pub fn new(model: Arc<M>, mind: Option<Arc<dyn Mind>>, settings: Settings) -> Self {
        Self {
            model,
            mind,
            settings,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum State {
    /// Place steers, start the step.
    Prepare { round: u32 },
    /// Let Experience recall, retain, refine before the request.
    Consider { round: u32 },
    /// Ask the model. `attempt` counts failures, crashes included.
    Request { round: u32, attempt: u32 },
    /// Waiting on the round's tool calls, in the order the model made them.
    Tools {
        round: u32,
        tasks: Vec<TaskId>,
        calls: Vec<String>,
    },
    /// Answered: settle the run's inputs.
    Finish { answer: String },
}

impl<M: Model> Task for Generation<M> {
    type Input = ();
    type State = State;
    type Output = String;
    const NAME: &'static str = NAME;

    fn initial(&self, _: &()) -> State {
        State::Prepare { round: 1 }
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let thread = rt.thread_id().to_string();
        match rt.state() {
            &State::Prepare { round } => rt.commit(|tx| {
                let mut inbox = inbox::load(tx, &thread)?;
                inbox::place_steers(tx, &thread, &mut inbox)?;
                inbox::save(tx, &thread, &inbox)?;
                tx.thread(&thread)?.start_step()?;
                Ok(Some(Next::Continue(match self.mind {
                    Some(_) => State::Consider { round },
                    None => State::Request { round, attempt: 1 },
                })))
            }),
            &State::Consider { round } => {
                let mind = self.mind.as_ref().expect("Consider only with a mind");
                // A failed consider is skipped, not fatal: the request goes ahead without it.
                let work = on_copy(&rt, mind.as_ref(), Work::Consider).await;
                rt.commit(|tx| {
                    if let Ok((base, events)) = work {
                        apply(tx, &thread, base, events)?;
                    }
                    Ok(Some(Next::Continue(State::Request { round, attempt: 1 })))
                })
            }
            &State::Request { round, attempt } => self.request(&rt, round, attempt).await,
            State::Tools {
                round,
                tasks,
                calls,
            } => {
                let outcomes = rt.outcomes(tasks)?;
                let results = calls
                    .iter()
                    .zip(&outcomes)
                    .map(|(call, outcome)| {
                        let output = outcome
                            .result::<ToolOutput>()
                            .unwrap_or_else(|_| ToolOutput::error(describe(outcome)));
                        result_block(call, output)
                    })
                    .collect();
                let next = round + 1;
                rt.commit(|tx| {
                    tx.thread(&thread)?.push(Message {
                        role: Role::User,
                        content: results,
                    })?;
                    Ok(Some(Next::Continue(State::Prepare { round: next })))
                })
            }
            State::Finish { answer } => rt.commit(|tx| {
                let answer = answer.clone();
                inbox::end_run(
                    tx,
                    &thread,
                    Settled::Done {
                        answer: answer.clone(),
                    },
                    false,
                )?;
                Ok(Some(Next::Complete(answer)))
            }),
        }
    }

    /// Settles the run's inputs and withdraws the queue. Tool calls left without results get
    /// "aborted" ones, so the thread stays valid for the next request.
    async fn abort(&self, rt: Runtime<Self>) -> Result<()> {
        let thread = rt.thread_id().to_string();
        rt.commit(|tx| {
            if let State::Tools { calls, .. } = rt.state() {
                let results = calls
                    .iter()
                    .map(|call| result_block(call, ToolOutput::error("aborted".into())))
                    .collect();
                tx.thread(&thread)?.push(Message {
                    role: Role::User,
                    content: results,
                })?;
            }
            let reason = "aborted".to_string();
            inbox::end_run(tx, &thread, Settled::Unanswered { reason }, true)?;
            Ok(Some(Next::Aborted(None)))
        })
    }
}

impl<M: Model> Generation<M> {
    async fn request(&self, rt: &Runtime<Self>, round: u32, attempt: u32) -> Result<()> {
        // The process stopped while this request was in flight: a failed attempt, not a resend.
        if rt.restarts() > 0 {
            return self.retry(rt, round, attempt, "interrupted by a restart".into());
        }
        let ctx = rt.thread(|t| t.context().clone())?;
        let reply = tokio::select! {
            reply = self.model.chat(&ctx) => reply,
            _ = rt.cancelled() => bail!("cancelled"),
        };
        let message = match reply {
            Ok(response) => response.message,
            Err(error) => return self.retry(rt, round, attempt, format!("{error:#}")),
        };

        let thread = rt.thread_id().to_string();
        let max_rounds = self.settings.max_rounds;
        rt.commit(|tx| {
            let calls: Vec<ToolCallInput> = message
                .tool_calls()
                .map(|(id, name, input)| ToolCallInput {
                    call_id: id.to_string(),
                    name: name.to_string(),
                    arguments: input.clone(),
                })
                .collect();
            let answer = message.text();
            tx.thread(&thread)?.push(message)?;
            if calls.is_empty() {
                return Ok(Some(Next::Continue(State::Finish { answer })));
            }
            if round >= max_rounds {
                return turn_limit(tx, &thread, &calls).map(Some);
            }
            let mut tasks = Vec::new();
            let mut ids = Vec::new();
            for call in calls {
                ids.push(call.call_id.clone());
                let options = TaskOptions::owned_by(rt.task_id());
                tasks.push(tx.create_task::<ToolCall>(&thread, call, options)?);
            }
            Ok(Some(Next::Wait {
                state: State::Tools {
                    round,
                    tasks: tasks.clone(),
                    calls: ids,
                },
                on: tasks,
                policy: Policy::AllSettled,
            }))
        })
    }

    /// Another attempt after a durable backoff, or the run ends unanswered.
    fn retry(&self, rt: &Runtime<Self>, round: u32, attempt: u32, error: String) -> Result<()> {
        if attempt < self.settings.max_attempts {
            let backoff = self.settings.backoff_ms << (attempt - 1).min(16);
            let next = State::Request {
                round,
                attempt: attempt + 1,
            };
            return rt.sleep_until(rt.now() + backoff, next);
        }
        let thread = rt.thread_id().to_string();
        rt.commit(|tx| {
            let reason = format!("model_error: {error}");
            inbox::end_run(tx, &thread, Settled::Unanswered { reason }, false)?;
            Ok(Some(Next::Fail(error)))
        })
    }
}

/// Out of rounds: the calls get error results (so the thread stays valid) and the run ends.
fn turn_limit(tx: &mut Tx, thread: &str, calls: &[ToolCallInput]) -> Result<Next<State, String>> {
    let results = calls
        .iter()
        .map(|call| {
            result_block(
                &call.call_id,
                ToolOutput::error("not run: turn limit".into()),
            )
        })
        .collect();
    tx.thread(thread)?.push(Message {
        role: Role::User,
        content: results,
    })?;
    let reason = "turn_limit".to_string();
    inbox::end_run(tx, thread, Settled::Unanswered { reason }, false)?;
    Ok(Next::Fail("turn limit".into()))
}

fn result_block(call_id: &str, output: ToolOutput) -> Block {
    Block::ToolResult {
        call_id: call_id.to_string(),
        content: output.content,
        is_error: output.is_error,
    }
}

fn describe(outcome: &engine::Outcome) -> String {
    match outcome {
        engine::Outcome::Completed { .. } => "the tool returned an unreadable result".into(),
        engine::Outcome::Failed { error } | engine::Outcome::Faulted { error } => error.clone(),
        engine::Outcome::Aborted { .. } => "aborted".into(),
        engine::Outcome::Orphaned { reason } => reason.clone(),
    }
}
