//! The agent loop, as durable tasks on the engine, after pi-durable.
//!
//! An input is a submission. Submitting to an idle thread places it as the user's message and
//! starts a run; submitting to a busy one queues it in the thread's inbox: a follow-up starts
//! the next run once this one answers, a steer joins the current run after its tool round.
//!
//! A run is a `Generation` task: prepare (place steers, start the step) → request the model →
//! run the tools it asked for as `ToolCall` tasks it owns and waits on → again, until the model
//! answers without tools → finish (settle the run's inputs, start the next queued follow-up).
//! Every step is a commit, so a crash resumes the run where it stopped:
//!
//! - A model request cut off by a crash counts as a failed attempt: it's retried after a
//!   durable backoff, not silently resent. Out of attempts, the run's inputs are unanswered.
//! - A tool call's intent is committed before it runs. Cut off, only a `Replay::Safe` tool runs
//!   again; others give the model an "interrupted" error result.
//! - A run that ends without settling its inputs (it faulted, or was orphaned) is noticed by
//!   the next submission or `wait` on the thread: its inputs are unanswered, its open tool
//!   calls get error results, and the queue moves on.

mod generation;
mod idle;
mod inbox;
mod tool;

pub use generation::Generation;
pub use idle::Idle;
pub use tool::{ToolCall, ToolCallInput, ToolOutput};

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use context::Context;
use engine::{Engine, Next, Registry, Runtime, Task, TaskId, Tx};
use inference::providers::Api;
use inference::{Provider, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use state::{Event, Thread};

/// What the agent loop calls for each request.
pub trait Model: Send + Sync + 'static {
    fn chat(&self, ctx: &Context) -> impl Future<Output = Result<Response>> + Send;
}

/// A provider and its request settings, as a `Model`.
pub struct ProviderModel<A> {
    pub provider: Provider<A>,
    pub request: Request,
}

impl<A: Api + Send + Sync + 'static> Model for ProviderModel<A> {
    async fn chat(&self, ctx: &Context) -> Result<Response> {
        self.provider.chat(&self.request, ctx).await
    }
}

/// Whether a tool may run again after a crash cut it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Replay {
    /// Running it twice does no harm (reads, idempotent writes).
    Safe,
    /// It may have had effects; after a crash the model is told it was interrupted.
    Unsafe,
}

pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// A tool the model can call. Its result is what the model sees; an error becomes an error
/// result.
pub trait Tool: Send + Sync + 'static {
    fn spec(&self) -> context::Tool;

    fn replay(&self) -> Replay {
        Replay::Unsafe
    }

    fn execute(&self, arguments: Value) -> ToolFuture<'_>;
}

/// The tools this process can run, by name. Which tools a thread offers the model is the
/// thread's own state (`Thread::register`).
#[derive(Default, Clone)]
pub struct Tools {
    by_name: HashMap<String, Arc<dyn Tool>>,
}

impl Tools {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, tool: impl Tool) -> &mut Self {
        self.by_name.insert(tool.spec().name, Arc::new(tool));
        self
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.by_name.get(name).cloned()
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// Model requests per round before the run fails.
    pub max_attempts: u32,
    /// Backoff before the second attempt; doubles after each.
    pub backoff_ms: u64,
    /// Tool rounds per run before it ends unanswered.
    pub max_rounds: u32,
    /// Idle time after a run before the thread dreams.
    pub dream_after_ms: u64,
    /// Idle time after a run before the thread settles into its resume card.
    pub settle_after_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 1_000,
            max_rounds: 25,
            dream_after_ms: 30 * 60_000,
            settle_after_ms: 2 * 3_600_000,
        }
    }
}

pub type MindFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// What Experience does around the loop. Each works on a copy of the thread (it may call a
/// model, which can't happen inside a commit); what it records there is applied to the thread
/// in a commit afterwards, if the thread hasn't changed meanwhile.
pub trait Mind: Send + Sync + 'static {
    /// Before each model request: recall, retain, refine.
    fn consider<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a>;
    /// After `dream_after_ms` idle: summarize, update the card, clean up memory, compact.
    fn dream<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a>;
    /// After `settle_after_ms` idle: write the resume card and reset the context to it.
    fn settle<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a>;
}

impl<A: Api + Send + Sync + 'static> Mind for experience::Experience<A> {
    fn consider<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a> {
        Box::pin(async move { self.consider(thread).await.map(drop) })
    }

    fn dream<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a> {
        Box::pin(async move { self.dream(thread).await.map(drop) })
    }

    fn settle<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a> {
        Box::pin(async move { self.settle(thread).await.map(drop) })
    }
}

/// Registers the agent's tasks; open the engine with this registry.
pub fn register<M: Model>(
    registry: &mut Registry,
    model: Arc<M>,
    tools: Tools,
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
) {
    registry
        .register(Generation::new(model, mind.clone(), settings.clone()))
        .register(ToolCall::new(tools))
        .register(Idle::new(mind, settings));
}

/// One of a mind's jobs: `Mind::consider`, `Mind::dream` or `Mind::settle`.
pub(crate) type Work = for<'a> fn(&'a dyn Mind, &'a mut Thread) -> MindFuture<'a>;

/// Runs a mind's work on a copy of the task's thread, then commits `next` with what it
/// recorded there. If the work failed, or the thread changed meanwhile (the events would land
/// in the wrong place), the work is lost: skipped, not fatal.
pub(crate) async fn think<T: Task>(
    rt: &Runtime<T>,
    mind: &dyn Mind,
    work: Work,
    next: Next<T::State, T::Output>,
) -> Result<()> {
    let mut copy = rt.thread(|t| t.snapshot())?;
    let base = copy.checkpoints().len();
    let done = tokio::select! {
        result = work(mind, &mut copy) => result.is_ok(),
        _ = rt.cancelled() => anyhow::bail!("cancelled"),
    };
    let events: Vec<Event> = copy.checkpoints()[base..]
        .iter()
        .map(|c| c.event.clone())
        .collect();
    rt.commit(|tx| {
        let thread = tx.thread(rt.thread_id())?;
        if done && thread.checkpoints().len() == base {
            thread.append(events)?;
        }
        Ok(Some(next))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhenBusy {
    /// Waits for the run to answer, then starts the next run.
    #[default]
    FollowUp,
    /// Joins the running work after its current tool round.
    Steer,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub text: String,
    /// The same request ID finds the same submission instead of submitting twice.
    pub request_id: Option<String>,
    pub when_busy: WhenBusy,
}

impl Input {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Placed,
    Done,
    Unanswered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    pub id: String,
    pub thread: String,
    pub text: String,
    pub request_id: Option<String>,
    pub when_busy: WhenBusy,
    pub status: Status,
    /// The run it was placed in.
    pub run: Option<TaskId>,
    pub answer: Option<String>,
    pub reason: Option<String>,
}

/// Where `send` put an input.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub thread: String,
    pub submission: String,
    /// `None` when the request ID was already submitted.
    pub route: Option<router::Route>,
}

const LAST_THREAD: &str = "agent/last-thread";

#[derive(Debug, Clone, PartialEq)]
pub enum Settled {
    Done { answer: String },
    Unanswered { reason: String },
}

impl Settled {
    pub(crate) fn unanswered(reason: impl Into<String>) -> Self {
        Self::Unanswered {
            reason: reason.into(),
        }
    }
}

/// Submits inputs and waits for them.
#[derive(Clone)]
pub struct Agent {
    engine: Engine,
}

impl Agent {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Routes an input to a thread (an existing one, or a new one) and submits it there. The
    /// route is noted in the thread, and the submission admitted, in one commit.
    pub async fn send<A: Api>(&self, router: &router::Router<A>, input: Input) -> Result<Sent> {
        if let Some(request_id) = &input.request_id
            && let Some(id) = self.engine.doc::<String>(&inbox::request_key(request_id))?
            && let Some(submission) = self.submission(&id)?
        {
            return Ok(Sent {
                thread: submission.thread,
                submission: id,
                route: None,
            });
        }
        let now = std::time::SystemTime::now();
        let store = self.engine.store().as_ref();
        let threads = router.threads(store, now)?;
        let routing = router::Input {
            text: input.text.clone(),
            now,
            last: self.engine.doc(LAST_THREAD)?,
        };
        let route = router.route(store, &routing, &threads).await?;
        self.engine.commit(|tx| admit(tx, route, input))
    }

    /// Admits an input durably and returns its submission ID.
    pub fn submit(&self, thread: &str, input: Input) -> Result<String> {
        self.engine.commit(|tx| inbox::submit(tx, thread, input))
    }

    pub fn submission(&self, id: &str) -> Result<Option<Submission>> {
        self.engine.doc(&inbox::submission_key(id))
    }

    /// Resolves once the submission is answered or can't be.
    pub async fn wait(&self, id: &str) -> Result<Settled> {
        self.engine
            .watch(|engine| {
                let Some(submission) = engine.doc::<Submission>(&inbox::submission_key(id))? else {
                    anyhow::bail!("no submission {id}");
                };
                match submission.status {
                    Status::Done => {
                        let answer = submission.answer.unwrap_or_default();
                        return Ok(Some(Settled::Done { answer }));
                    }
                    Status::Unanswered => {
                        let reason = submission.reason.unwrap_or_default();
                        return Ok(Some(Settled::Unanswered { reason }));
                    }
                    Status::Queued | Status::Placed => {}
                }
                // A run that ended without settling its inputs (it faulted, say) never will:
                // nothing else notices until the next submission, so recover from it here.
                let thread = &submission.thread;
                let inbox: inbox::Inbox =
                    engine.doc(&inbox::inbox_key(thread))?.unwrap_or_default();
                if let Some(run) = inbox.run
                    && let Some(ended) = engine.task(run)?.filter(|t| t.state.is_terminal())
                {
                    let reason = format!("run ended: {:?}", ended.state.outcome());
                    engine.commit(|tx| inbox::recover(tx, thread, &reason))?;
                }
                Ok(None)
            })
            .await
    }
}

/// Submits a routed input to its thread (an existing one, or a new one) and notes the route
/// there. A request ID submitted since `send` checked (by another `send`) changes nothing.
fn admit(tx: &mut Tx, route: router::Route, input: Input) -> Result<Sent> {
    if let Some(submission) = inbox::existing(tx, &input)? {
        return Ok(Sent {
            thread: inbox::submission(tx, &submission)?.thread,
            submission,
            route: None,
        });
    }
    let thread = match &route.target {
        router::Target::Continue(id) => id.clone(),
        router::Target::New => {
            let n: u64 = tx.doc("agent/next-thread")?.unwrap_or(1);
            tx.put_doc("agent/next-thread", &(n + 1))?;
            format!("thread-{n}")
        }
    };
    tx.thread(&thread)?
        .routed(route.by.as_str(), &route.reason)?;
    let submission = inbox::submit(tx, &thread, input)?;
    tx.put_doc(LAST_THREAD, &thread)?;
    Ok(Sent {
        thread,
        submission,
        route: Some(route),
    })
}

#[cfg(test)]
mod tests;
