//! What one invocation of a task can do.

use std::marker::PhantomData;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;

use crate::task::{Next, Outcome, Task, TaskId, TaskRecord};
use crate::{Shared, Step, Tx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Run,
    Abort,
}

pub(crate) struct Invocation {
    pub(crate) shared: Arc<Shared>,
    /// The task as it was when this phase started.
    pub(crate) record: TaskRecord,
    pub(crate) mode: Mode,
    pub(crate) generation: u64,
    pub(crate) cancel: watch::Receiver<bool>,
}

/// One phase of a task: its input, its checkpoint, and the operations it may use. Operations
/// fail once the invocation is over (the task moved on, was aborted, or the engine closed).
pub struct Runtime<T: Task> {
    invocation: Arc<Invocation>,
    input: Arc<T::Input>,
    state: Arc<T::State>,
    _task: PhantomData<fn() -> T>,
}

impl<T: Task> Clone for Runtime<T> {
    fn clone(&self) -> Self {
        Self {
            invocation: self.invocation.clone(),
            input: self.input.clone(),
            state: self.state.clone(),
            _task: PhantomData,
        }
    }
}

impl<T: Task> Runtime<T> {
    pub(crate) fn new(invocation: Arc<Invocation>) -> Result<Self> {
        let record = &invocation.record;
        let input = serde_json::from_value(record.input.clone()).context("task input")?;
        let checkpoint = record
            .state
            .checkpoint()
            .context("task has no checkpoint")?;
        let state = serde_json::from_value(checkpoint.clone()).context("task checkpoint")?;
        Ok(Self {
            invocation,
            input: Arc::new(input),
            state: Arc::new(state),
            _task: PhantomData,
        })
    }

    pub fn task_id(&self) -> TaskId {
        self.invocation.record.id
    }

    pub fn thread_id(&self) -> &str {
        &self.invocation.record.thread
    }

    pub fn input(&self) -> &T::Input {
        &self.input
    }

    /// The checkpoint this phase runs from.
    pub fn state(&self) -> &T::State {
        &self.state
    }

    /// Reads the task's thread as last committed, outside any commit: for slow work such as
    /// a model request, whose result is committed afterwards.
    pub fn thread<R>(&self, read: impl FnOnce(&state::Thread) -> R) -> Result<R> {
        self.invocation.shared.thread(self.thread_id(), read)
    }

    /// Times in a row a crash cut this task off since it last committed a new state: nonzero
    /// means this phase is running again after a crash.
    pub fn restarts(&self) -> u32 {
        self.invocation.record.restarts
    }

    /// Ms since the epoch.
    pub fn now(&self) -> u64 {
        crate::now_ms()
    }

    /// True once the task has been asked to abort: wrap up, the abort handler runs next.
    pub fn is_cancelled(&self) -> bool {
        *self.invocation.cancel.borrow()
    }

    /// Resolves once the task has been asked to abort.
    pub async fn cancelled(&self) {
        let mut cancel = self.invocation.cancel.clone();
        while !*cancel.borrow_and_update() {
            if cancel.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Commits on the engine's line, with whatever `change` writes, and the task's next state
    /// when it returns one. Fails if this invocation is over, or, while running (not aborting),
    /// once the task has an abort mark.
    pub fn commit(
        &self,
        change: impl FnOnce(&mut Tx) -> Result<Option<Next<T::State, T::Output>>>,
    ) -> Result<()> {
        let invocation = &self.invocation;
        let id = self.task_id();
        invocation.shared.commit(|tx| {
            tx.check_invocation(id, invocation.generation, invocation.mode)?;
            match change(tx)? {
                Some(next) => tx.advance(id, step(next)?),
                None => Ok(()),
            }
        })
    }

    /// Commits a sleep: the task holds nothing until `until` (ms since the epoch), then runs
    /// from `state`. Return from the phase right after.
    pub fn sleep_until(&self, until: u64, state: T::State) -> Result<()> {
        self.commit(|_| Ok(Some(Next::Sleep { state, until })))
    }

    /// The value stored under `name` for this task, storing `candidate` if there is none: the
    /// first writer wins, also across crashes.
    pub fn memo<V: Serialize + DeserializeOwned>(&self, name: &str, candidate: V) -> Result<V> {
        let candidate = serde_json::to_value(candidate)?;
        let mut stored = None;
        let id = self.task_id();
        let invocation = &self.invocation;
        invocation.shared.commit(|tx| {
            tx.check_invocation(id, invocation.generation, invocation.mode)?;
            tx.update(id, |task| {
                let value = task.memos.entry(name.to_string()).or_insert(candidate);
                stored = Some(value.clone());
            })
        })?;
        Ok(serde_json::from_value(stored.expect("set in commit"))?)
    }

    /// Outcomes of ended tasks, in order. Fails for one still live.
    pub fn outcomes(&self, ids: &[TaskId]) -> Result<Vec<Outcome>> {
        ids.iter()
            .map(|id| match self.invocation.shared.task(*id)? {
                Some(task) if task.state.is_terminal() => {
                    Ok(task.state.outcome().cloned().expect("terminal"))
                }
                Some(_) => bail!("task {id} has not ended"),
                None => bail!("no task {id}"),
            })
            .collect()
    }
}

fn step<S: Serialize, R: Serialize>(next: Next<S, R>) -> Result<Step> {
    Ok(match next {
        Next::Continue(state) => Step::Continue(serde_json::to_value(state)?),
        Next::Wait { state, on, policy } => Step::Wait {
            checkpoint: serde_json::to_value(state)?,
            on,
            policy,
        },
        Next::Sleep { state, until } => Step::Sleep {
            checkpoint: serde_json::to_value(state)?,
            until,
        },
        Next::Complete(result) => Step::End(Outcome::Completed {
            result: serde_json::to_value(result)?,
        }),
        Next::Fail(error) => Step::End(Outcome::Failed { error }),
        Next::Aborted(reason) => Step::End(Outcome::Aborted { reason }),
    })
}
