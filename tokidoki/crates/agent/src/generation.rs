//! A run: request the model, run the tools it asks for, again, until it answers.

use std::sync::Arc;

use anyhow::{Result, bail};
use context::{Block, Message, Role};
use engine::{Next, Outcome, Policy, Runtime, Task, TaskId, TaskOptions};
use serde::{Deserialize, Serialize};
use state::Thread;

use crate::tool::{ToolCall, ToolCallInput, ToolOutput};
use crate::{Live, Mind, Model, Settings, Settled, inbox, think};

pub(crate) const NAME: &str = "agent.generation";

pub struct Generation<M> {
    model: Arc<M>,
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
    live: Live,
}

impl<M> Generation<M> {
    pub fn new(model: Arc<M>, mind: Option<Arc<dyn Mind>>, settings: Settings, live: Live) -> Self {
        Self {
            model,
            mind,
            settings,
            live,
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
        let thread = rt.thread_id();
        match rt.state() {
            &State::Prepare { round } => rt.commit(|tx| {
                inbox::place_steers(tx, thread)?;
                tx.thread(thread)?.start_step()?;
                Ok(Some(Next::Continue(match self.mind {
                    Some(_) => State::Consider { round },
                    None => State::Request { round, attempt: 1 },
                })))
            }),
            &State::Consider { round } => {
                let mind = self.mind.as_deref().expect("Consider only with a mind");
                let next = Next::Continue(State::Request { round, attempt: 1 });
                think(&rt, mind, Mind::consider, next).await
            }
            &State::Request { round, attempt } => self.request(&rt, round, attempt).await,
            State::Tools {
                round,
                tasks,
                calls,
            } => {
                let results = results(&rt, tasks, calls)?;
                let next = round + 1;
                rt.commit(|tx| {
                    tx.thread(thread)?.push(results)?;
                    Ok(Some(Next::Continue(State::Prepare { round: next })))
                })
            }
            State::Finish { answer } => rt.commit(|tx| {
                let settled = Settled::Done {
                    answer: answer.clone(),
                };
                inbox::end_run(tx, thread, settled, false)?;
                Ok(Some(Next::Complete(answer.clone())))
            }),
        }
    }

    /// The engine faulted or orphaned the run, so it never settled its inputs: settle them now,
    /// close its tool calls and let the queue move on, rather than waiting for the next
    /// `submit` or `wait` on the thread to notice.
    fn on_fault(&self, tx: &mut engine::Tx, task: &engine::TaskRecord) -> Result<()> {
        let reason = match task.state.outcome() {
            Some(Outcome::Faulted { error }) => format!("run faulted: {error}"),
            Some(Outcome::Orphaned { reason }) => format!("run orphaned: {reason}"),
            _ => "the run ended without an answer".to_string(),
        };
        inbox::recover(tx, &task.thread, &reason)
    }

    /// Settles the run's inputs and withdraws the queue. The round's tool calls have all ended
    /// by now (owned work is aborted first): each gets its result, so the thread stays valid
    /// and the model isn't told a call that went through was aborted.
    async fn abort(&self, rt: Runtime<Self>) -> Result<()> {
        let thread = rt.thread_id();
        let results = match rt.state() {
            State::Tools { tasks, calls, .. } => Some(results(&rt, tasks, calls)?),
            _ => None,
        };
        rt.commit(|tx| {
            if let Some(results) = results {
                tx.thread(thread)?.push(results)?;
            }
            inbox::end_run(tx, thread, Settled::unanswered("aborted"), true)?;
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
        let (mut ctx, seq) = rt.thread(|t| (t.context().clone(), t.checkpoints().len()))?;
        // What is sent, not what is kept: the thread still holds the calls as they were made.
        ctx.retell_unoffered_calls();
        let thread = rt.thread_id();
        let show = |delta| self.live.delta(thread, seq, delta);
        let reply = tokio::select! {
            reply = self.model.stream(&ctx, &show) => reply,
            _ = rt.cancelled() => bail!("cancelled"),
        };
        let message = match reply {
            Ok(response) => response.message,
            Err(error) => return self.retry(rt, round, attempt, format!("{error:#}")),
        };

        let max_rounds = self.settings.max_rounds;
        let calls: Vec<ToolCallInput> = message
            .tool_calls()
            .map(|(id, name, input)| ToolCallInput {
                call_id: id.to_string(),
                name: name.to_string(),
                arguments: input.clone(),
            })
            .collect();
        rt.commit(|tx| {
            let answer = message.text();
            tx.thread(thread)?.push(message)?;
            if calls.is_empty() {
                return Ok(Some(Next::Continue(State::Finish { answer })));
            }
            // Out of tool rounds: the calls get error results and the run ends.
            if round > max_rounds {
                tracing::warn!(round, "turn limit: the model kept calling tools");
                close_calls(tx.thread(thread)?, "not run: turn limit")?;
                inbox::end_run(tx, thread, Settled::unanswered("turn_limit"), false)?;
                return Ok(Some(Next::Fail("turn limit".into())));
            }
            let ids = calls.iter().map(|c| c.call_id.clone()).collect();
            let tasks = calls
                .into_iter()
                .map(|call| {
                    let options = TaskOptions::owned_by(rt.task_id());
                    tx.create_task::<ToolCall>(thread, call, options)
                })
                .collect::<Result<Vec<_>>>()?;
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
        let last = attempt >= self.settings.max_attempts;
        tracing::warn!(round, attempt, last, error = %error, "model request failed");
        if !last {
            let backoff = self.settings.backoff_ms << (attempt - 1).min(16);
            let next = State::Request {
                round,
                attempt: attempt + 1,
            };
            return rt.sleep_until(rt.now() + backoff, next);
        }
        let thread = rt.thread_id();
        rt.commit(|tx| {
            let reason = format!("model_error: {error}");
            inbox::end_run(tx, thread, Settled::unanswered(reason), false)?;
            Ok(Some(Next::Fail(error)))
        })
    }
}

/// The round's tool results, as one message in call order, from the ended call tasks.
fn results<M: Model>(
    rt: &Runtime<Generation<M>>,
    tasks: &[TaskId],
    calls: &[String],
) -> Result<Message> {
    let outcomes = rt.outcomes(tasks)?;
    let content = calls
        .iter()
        .zip(&outcomes)
        .map(|(call, outcome)| {
            let output = outcome
                .result::<ToolOutput>()
                .unwrap_or_else(|_| ToolOutput::error(describe(outcome)));
            result_block(call, output)
        })
        .collect();
    Ok(Message {
        role: Role::User,
        content,
    })
}

/// Gives the last message's tool calls `error` results, if it's a reply still waiting for
/// them: every call needs a result before the next request.
pub(crate) fn close_calls(thread: &mut Thread, error: &str) -> Result<()> {
    let content: Vec<Block> = thread
        .unanswered_calls()
        .into_iter()
        .map(|id| result_block(id, ToolOutput::error(error.into())))
        .collect();
    if content.is_empty() {
        return Ok(());
    }
    thread.push(Message {
        role: Role::User,
        content,
    })
}

fn result_block(call_id: &str, output: ToolOutput) -> Block {
    Block::ToolResult {
        call_id: call_id.to_string(),
        content: output.content,
        is_error: output.is_error,
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Completed { .. } => "the tool returned an unreadable result".into(),
        Outcome::Failed { error } | Outcome::Faulted { error } => error.clone(),
        Outcome::Aborted { .. } => "aborted".into(),
        Outcome::Orphaned { reason } => reason.clone(),
    }
}
