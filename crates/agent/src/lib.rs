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
mod live;
mod tool;

pub use generation::Generation;
pub use idle::Idle;
pub use live::{Live, Shown};
pub use tool::{ToolCall, ToolCallInput, ToolOutput};

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use context::Context;
use engine::{Engine, Next, Registry, Runtime, Task, TaskId, Tx};
use inference::providers::Api;
use inference::providers::OnDelta;
use inference::{Provider, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use state::{Event, Thread};

use interface::{Doing, Input, Rejected, Sent, Status, Submission, WhenBusy};

/// What the agent loop calls for each request.
pub trait Model: Send + Sync + 'static {
    fn chat(&self, ctx: &Context) -> impl Future<Output = Result<Response>> + Send;

    /// `chat`, handing `on_delta` the reply as it is written. A model that can't stream just
    /// answers.
    fn stream(
        &self,
        ctx: &Context,
        on_delta: OnDelta<'_>,
    ) -> impl Future<Output = Result<Response>> + Send {
        let _ = on_delta;
        self.chat(ctx)
    }
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

    async fn stream(&self, ctx: &Context, on_delta: OnDelta<'_>) -> Result<Response> {
        self.provider.stream(&self.request, ctx, on_delta).await
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

    /// Every tool's spec, by name.
    pub fn specs(&self) -> Vec<context::Tool> {
        let mut specs: Vec<context::Tool> = self.by_name.values().map(|t| t.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }
}

/// What a new thread starts with, set in the commit that admits its first input.
#[derive(Debug, Clone, Default)]
pub struct NewThreads {
    pub system: Option<String>,
    pub tools: Vec<context::Tool>,
}

impl NewThreads {
    fn set_up(&self, tx: &mut Tx, thread: &str) -> Result<()> {
        let thread = tx.thread(thread)?;
        if !thread.checkpoints().is_empty() {
            return Ok(());
        }
        if let Some(system) = &self.system {
            thread.set_system(system.clone())?;
        }
        for tool in &self.tools {
            thread.register(tool.clone())?;
        }
        Ok(())
    }
}

/// The submission a written document holds, if it is one: for reading `Engine::subscribe`.
pub fn submission_in(key: &str, value: Option<&Value>) -> Option<Submission> {
    key.strip_prefix(inbox::SUBMISSION_PREFIX)?;
    let record: Record = serde_json::from_value(value?.clone()).ok()?;
    Some(record.shown)
}

/// What a task, as a commit left it (`Engine::subscribe`), shows someone watching: work that
/// started on a thread. `None` for the rest of the agent's bookkeeping.
pub fn doing(task: &engine::TaskRecord) -> Option<Doing> {
    if task.state.status() != engine::Status::Running {
        return None;
    }
    match task.kind.as_str() {
        generation::NAME => match task.state.checkpoint_as()? {
            generation::State::Consider { .. } => Some(Doing::Remembering),
            generation::State::Request { .. } => Some(Doing::Thinking),
            _ => None,
        },
        ToolCall::NAME => match task.state.checkpoint_as()? {
            tool::State::Call => {
                let call: ToolCallInput = serde_json::from_value(task.input.clone()).ok()?;
                Some(Doing::Tool { name: call.name })
            }
            tool::State::Execute { .. } => None,
        },
        _ => None,
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

/// Registers the agent's tasks; open the engine with this registry. Replies are shown on
/// `live` as they are written.
pub fn register<M: Model>(
    registry: &mut Registry,
    model: Arc<M>,
    tools: Tools,
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
    live: Live,
) {
    registry
        .register(Generation::new(model, mind.clone(), settings.clone(), live))
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
    let result = tokio::select! {
        result = work(mind, &mut copy) => result,
        _ = rt.cancelled() => anyhow::bail!("cancelled"),
    };
    if let Err(error) = &result {
        // The phase (consider, dream, settle) is on the span it runs in.
        tracing::warn!(error = %format!("{error:#}"), "mind work failed; skipped");
    }
    let events: Vec<Event> = copy.checkpoints()[base..]
        .iter()
        .map(|c| c.event.clone())
        .collect();
    rt.commit(|tx| {
        let thread = tx.thread(rt.thread_id())?;
        if result.is_ok() && thread.checkpoints().len() == base {
            thread.append(events)?;
        } else if result.is_ok() && !events.is_empty() {
            tracing::warn!(
                lost = events.len(),
                "mind work discarded: the thread changed meanwhile"
            );
        }
        Ok(Some(next))
    })
}

/// An input as the inbox admits it: its text, and how to queue it.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub text: String,
    pub request_id: Option<String>,
    pub when_busy: WhenBusy,
}

impl Draft {
    /// The input's text; other kinds of parts are refused until the loop can use them.
    fn of(input: &Input) -> Result<Self> {
        let mut texts = Vec::new();
        for part in &input.parts {
            match part {
                interface::Part::Text { text } => texts.push(text.as_str()),
                other => {
                    let kind = other.kind();
                    return Err(Rejected(format!("{kind} inputs aren't supported yet")).into());
                }
            }
        }
        let text = texts.join("\n\n");
        if text.trim().is_empty() {
            return Err(Rejected("the input is empty".into()).into());
        }
        Ok(Self {
            text,
            request_id: input.request_id.clone(),
            when_busy: input.when_busy,
        })
    }
}

/// A submission as stored: what `interface::Submission` shows, and what only the inbox needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    #[serde(flatten)]
    pub shown: Submission,
    pub when_busy: WhenBusy,
    /// The run it was placed in.
    pub run: Option<TaskId>,
}

const LAST_THREAD: &str = "agent/last-thread";

/// How a run settles its inputs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Settled {
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
    new_threads: Arc<NewThreads>,
}

impl Agent {
    /// Threads this agent starts get `new_threads`' system prompt and tools.
    pub fn new(engine: Engine, new_threads: NewThreads) -> Self {
        Self {
            engine,
            new_threads: Arc::new(new_threads),
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Stops the thread's run, if one is running: its inputs, and those queued behind it, are
    /// settled unanswered. Returns whether there was one.
    pub fn abort(&self, thread: &str) -> Result<bool> {
        self.engine.commit(
            |tx| match inbox::load(tx, thread)?.run.filter(|&run| tx.is_live(run)) {
                Some(run) => {
                    tx.abort_task(run, "stopped by the user")?;
                    Ok(true)
                }
                None => Ok(false),
            },
        )
    }

    /// Admits an input: to the thread it names, or else routed to a thread (an existing one,
    /// or a new one), the route noted in the thread and the input admitted in one commit.
    /// Fails with an `interface::Rejected` for an input it can't take (no text, say).
    pub async fn send<A: Api>(&self, router: &router::Router<A>, input: Input) -> Result<Sent> {
        let draft = Draft::of(&input)?;
        // A request ID already submitted finds that submission, wherever it went.
        if let Some(request_id) = &draft.request_id
            && let Some(id) = self.engine.doc::<String>(&inbox::request_key(request_id))?
            && let Some(submission) = self.submission(&id)?
        {
            return Ok(Sent {
                thread: submission.thread,
                submission: id,
                route: None,
            });
        }
        let place = match input.thread {
            Some(thread) => Place::Named(thread),
            None => {
                let now = std::time::SystemTime::now();
                let store = self.engine.store().as_ref();
                let threads = router.threads(store, now)?;
                let routing = router::Input {
                    text: draft.text.clone(),
                    now,
                    last: self.engine.doc(LAST_THREAD)?,
                };
                Place::Routed(router.route(store, &routing, &threads).await?)
            }
        };
        self.admit(place, draft)
    }

    /// Admits an input to `thread` and returns its submission ID: `send` without a router.
    #[cfg(test)]
    pub(crate) fn submit(&self, thread: &str, input: Input) -> Result<String> {
        let sent = self.admit(Place::Named(thread.to_string()), Draft::of(&input)?)?;
        Ok(sent.submission)
    }

    fn admit(&self, place: Place, draft: Draft) -> Result<Sent> {
        let text = draft.text.clone();
        let sent = self
            .engine
            .commit(|tx| admit(tx, &self.new_threads, place, draft))?;
        match &sent.route {
            // The router traced its decision, with the text.
            Some(route) => tracing::info!(
                thread = %sent.thread,
                submission = %sent.submission,
                new = route.new,
                "input routed"
            ),
            None => tracing::info!(
                thread = %sent.thread,
                submission = %sent.submission,
                text = %text,
                "input submitted"
            ),
        }
        Ok(sent)
    }

    pub fn submission(&self, id: &str) -> Result<Option<Submission>> {
        let record: Option<Record> = self.engine.doc(&inbox::submission_key(id))?;
        Ok(record.map(|r| r.shown))
    }

    /// Resolves once the submission is answered or can't be.
    pub async fn wait(&self, id: &str) -> Result<Submission> {
        self.engine
            .watch(|engine| {
                let Some(record) = engine.doc::<Record>(&inbox::submission_key(id))? else {
                    anyhow::bail!("no submission {id}");
                };
                if record.shown.is_settled() {
                    return Ok(Some(record.shown));
                }
                // A run that ended without settling its inputs never will: a version of the
                // task without `on_fault` orphaned it, say. Recover from it here.
                let thread = &record.shown.thread;
                let inbox: inbox::Inbox =
                    engine.doc(&inbox::inbox_key(thread))?.unwrap_or_default();
                if let Some(run) = inbox.run
                    && !engine.is_live(run)
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

/// Where an input goes: the thread it names, or where the router sent it.
enum Place {
    Named(String),
    Routed(router::Route),
}

/// Admits an input to its thread (named, routed to, or new), noting a route there. A request
/// ID already submitted (by a `send` since the caller checked) changes nothing.
fn admit(tx: &mut Tx, new_threads: &NewThreads, place: Place, draft: Draft) -> Result<Sent> {
    if let Some(submission) = inbox::existing(tx, &draft)? {
        return Ok(Sent {
            thread: inbox::submission(tx, &submission)?.shown.thread,
            submission,
            route: None,
        });
    }
    let (thread, route) = match place {
        Place::Named(thread) => (thread, None),
        Place::Routed(route) => match &route.target {
            router::Target::Continue(id) => (id.clone(), Some(route)),
            router::Target::New => (new_thread(tx)?, Some(route)),
        },
    };
    new_threads.set_up(tx, &thread)?;
    if let Some(route) = &route {
        tx.thread(&thread)?
            .routed(route.by.as_str(), &route.reason)?;
    }
    let submission = inbox::submit(tx, &thread, draft)?;
    tx.put_doc(LAST_THREAD, &thread)?;
    Ok(Sent {
        thread,
        submission,
        route: route.map(|route| interface::Route {
            by: route.by.as_str().to_string(),
            new: matches!(route.target, router::Target::New),
            reason: route.reason,
        }),
    })
}

/// The next free `thread-N`: a name an input took explicitly is skipped.
fn new_thread(tx: &mut Tx) -> Result<String> {
    let mut n: u64 = tx.doc("agent/next-thread")?.unwrap_or(1);
    loop {
        let id = format!("thread-{n}");
        n += 1;
        if tx.thread(&id)?.checkpoints().is_empty() {
            tx.put_doc("agent/next-thread", &n)?;
            return Ok(id);
        }
    }
}

#[cfg(test)]
mod tests;
