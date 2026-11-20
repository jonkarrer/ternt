//! Durable tasks: state machines that commit a checkpoint at every step, so a restarted
//! process continues from the last one.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Tx;
use crate::runtime::{Invocation, Runtime};

pub type TaskId = i64;

/// How a waiting task treats the tasks it waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// The first one to end without completing aborts the rest.
    FailFast,
    /// Resume once every one has ended, however it ended.
    AllSettled,
}

/// How a task ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Completed {
        result: Value,
    },
    /// An expected failure the task committed itself.
    Failed {
        error: String,
    },
    /// Ended by an abort, through the task's abort handler.
    Aborted {
        reason: Option<String>,
    },
    /// Can't run: its definition is missing or too old.
    Orphaned {
        reason: String,
    },
    /// Broke the contract: threw, panicked, made no durable progress, or kept crashing.
    Faulted {
        error: String,
    },
}

impl Outcome {
    pub fn is_completed(&self) -> bool {
        matches!(self, Outcome::Completed { .. })
    }

    /// The result of a completed task.
    pub fn result<R: DeserializeOwned>(&self) -> Result<R> {
        match self {
            Outcome::Completed { result } => Ok(serde_json::from_value(result.clone())?),
            other => anyhow::bail!("task did not complete: {other:?}"),
        }
    }
}

/// The complete durable state of a task. A task resumes from its checkpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskState {
    /// Ready to run.
    Pending {
        checkpoint: Value,
    },
    /// Reserved by an invocation in this process. Found on open, it was cut off by a crash.
    Running {
        checkpoint: Value,
    },
    /// Parked until the tasks in `on` have ended; then runs from `checkpoint`.
    Waiting {
        checkpoint: Value,
        on: Vec<TaskId>,
        policy: Policy,
    },
    /// Parked until the clock reaches `until` (ms since the epoch); then runs from `checkpoint`.
    Sleeping {
        checkpoint: Value,
        until: u64,
    },
    /// Outcome decided; becomes terminal once the work it owns has ended. Runs no more code.
    Completing {
        outcome: Outcome,
    },
    Terminal {
        outcome: Outcome,
    },
}

impl TaskState {
    pub fn checkpoint(&self) -> Option<&Value> {
        match self {
            TaskState::Pending { checkpoint }
            | TaskState::Running { checkpoint }
            | TaskState::Waiting { checkpoint, .. }
            | TaskState::Sleeping { checkpoint, .. } => Some(checkpoint),
            TaskState::Completing { .. } | TaskState::Terminal { .. } => None,
        }
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        match self {
            TaskState::Completing { outcome } | TaskState::Terminal { outcome } => Some(outcome),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskState::Terminal { .. })
    }

    /// The outcome of a task that has ended.
    pub(crate) fn ended(&self) -> Option<&Outcome> {
        match self {
            TaskState::Terminal { outcome } => Some(outcome),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            TaskState::Pending { .. } => "pending",
            TaskState::Running { .. } => "running",
            TaskState::Waiting { .. } => "waiting",
            TaskState::Sleeping { .. } => "sleeping",
            TaskState::Completing { .. } => "completing",
            TaskState::Terminal { .. } => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRecord {
    pub id: TaskId,
    /// The thread it works in.
    pub thread: String,
    /// The registered task name.
    pub kind: String,
    pub version: u32,
    pub input: Value,
    /// The task that owns it; `None` for a top-level task.
    pub owner: Option<TaskId>,
    /// A boundary, as in pi-durable: owned work that doesn't keep its owner from finishing and
    /// isn't aborted with it. It lives on until it ends or is aborted itself (a subagent that
    /// outlives the call that started it, say).
    pub background: bool,
    /// Aborted, with what it owns, if still live when a new process opens the engine.
    pub abandon_on_restart: bool,
    /// Durable abort mark: its run commits are refused, and its abort handler runs once the
    /// work it owns has ended.
    pub abort_requested: bool,
    pub abort_reason: Option<String>,
    /// Its abort handler has started. From then on every phase runs the handler, and its own
    /// waits and sleeps are kept like any task's, so cleanup can take steps.
    pub aborting: bool,
    /// Times in a row a crash cut it off without it committing a new state since.
    pub restarts: u32,
    /// First-writer-wins values (`Runtime::memo`).
    pub memos: BTreeMap<String, Value>,
    pub state: TaskState,
    /// Ms since the epoch.
    pub created_at: u64,
    pub ended_at: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskOptions {
    pub owner: Option<TaskId>,
    pub background: bool,
    pub abandon_on_restart: bool,
}

impl TaskOptions {
    pub fn owned_by(owner: TaskId) -> Self {
        Self {
            owner: Some(owner),
            ..Self::default()
        }
    }
}

/// What a phase commits next.
#[derive(Debug, Clone)]
pub enum Next<S, R> {
    /// A new checkpoint; the next phase runs from it.
    Continue(S),
    /// Park until `on` have ended, then run from `state`.
    Wait {
        state: S,
        on: Vec<TaskId>,
        policy: Policy,
    },
    /// Park until `until` (ms since the epoch), then run from `state`.
    Sleep {
        state: S,
        until: u64,
    },
    Complete(R),
    Fail(String),
    Aborted(Option<String>),
}

/// A task definition. `State` is the checkpoint, usually an enum with one variant per phase:
/// `run` is called with the task at its current checkpoint, must commit progress (a new
/// checkpoint, a wait, a sleep, or an outcome) through `rt.commit`, and is called again while
/// the task stays running. After a crash it runs again from the last checkpoint, so anything it
/// must not redo goes into an earlier commit.
pub trait Task: Send + Sync + 'static {
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + DeserializeOwned + Send + Sync + 'static;
    /// Stored with each task; names must be unique in a registry.
    const NAME: &'static str;
    /// Stored with each task; a stored task of another version is orphaned.
    const VERSION: u32 = 1;

    fn initial(&self, input: &Self::Input) -> Self::State;

    fn run(&self, rt: Runtime<Self>) -> impl Future<Output = Result<()>> + Send
    where
        Self: Sized;

    /// Runs after an abort mark, once the work it owns has ended, and must commit an outcome.
    /// By default: aborted, with nothing to undo.
    fn abort(&self, rt: Runtime<Self>) -> impl Future<Output = Result<()>> + Send
    where
        Self: Sized,
    {
        async move { rt.commit(|_| Ok(Some(Next::Aborted(None)))) }
    }

    /// Called after the engine itself ended the task: faulted (it broke the contract, or kept
    /// crashing) or orphaned. Its own code never ran to the end, so this is where to clean up
    /// what it was in the middle of. Runs in a commit of its own; best effort, an error is
    /// ignored. By default: nothing.
    fn on_fault(&self, tx: &mut Tx, task: &TaskRecord) -> Result<()> {
        let _ = (tx, task);
        Ok(())
    }
}

pub(crate) type BoxFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// A task definition with its types erased, as the scheduler holds it.
pub(crate) trait Erased: Send + Sync {
    fn version(&self) -> u32;
    fn initial(&self, input: &Value) -> Result<Value>;
    fn run(self: Arc<Self>, invocation: Arc<Invocation>) -> BoxFuture;
    fn abort(self: Arc<Self>, invocation: Arc<Invocation>) -> BoxFuture;
    fn on_fault(&self, tx: &mut Tx, task: &TaskRecord) -> Result<()>;
}

struct Kind<T>(T);

impl<T: Task> Erased for Kind<T> {
    fn version(&self) -> u32 {
        T::VERSION
    }

    fn initial(&self, input: &Value) -> Result<Value> {
        let input: T::Input = serde_json::from_value(input.clone()).context("task input")?;
        Ok(serde_json::to_value(self.0.initial(&input))?)
    }

    fn run(self: Arc<Self>, invocation: Arc<Invocation>) -> BoxFuture {
        Box::pin(async move { self.0.run(Runtime::new(invocation)?).await })
    }

    fn abort(self: Arc<Self>, invocation: Arc<Invocation>) -> BoxFuture {
        Box::pin(async move { self.0.abort(Runtime::new(invocation)?).await })
    }

    fn on_fault(&self, tx: &mut Tx, task: &TaskRecord) -> Result<()> {
        self.0.on_fault(tx, task)
    }
}

/// The task definitions this process can run. Nothing in it is stored: tasks store names.
#[derive(Default)]
pub struct Registry {
    kinds: HashMap<&'static str, Arc<dyn Erased>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Task>(&mut self, task: T) -> &mut Self {
        self.kinds.insert(T::NAME, Arc::new(Kind(task)));
        self
    }

    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn Erased>> {
        self.kinds.get(name).cloned()
    }
}
