//! The engine runs durable work, after pi-durable.
//!
//! Every change goes through one line of atomic commits: thread checkpoints and task states
//! are stored together in one SQLite transaction, or not at all, and nothing is shown before
//! it is stored. All work runs as tasks: state machines that commit a checkpoint at every step
//! (see `Task`). If the process dies, opening the engine again picks the work up where it
//! stopped:
//!
//! - A task that was running is pending again, with `restarts` counted; it runs its phase
//!   again from its last checkpoint. At `Settings::restart_limit` restarts in a row it is
//!   faulted instead, so work that keeps crashing the process ends.
//! - Waiting and sleeping tasks stay parked; a sleep's deadline is stored.
//! - A task marked `abandon_on_restart` is aborted, with everything it owns.
//!
//! Tasks own tasks. A task waits on others without running any code (`Next::Wait`); one that
//! finishes while work it owns is live is `completing` until that work has ended. Aborts run
//! bottom-up: owned work is aborted first, then the task's own abort handler runs, which may
//! itself wait and sleep. Background work (`TaskOptions::background`) is a boundary: it neither
//! holds its owner up nor is aborted with it.

pub mod runtime;
mod store;
pub mod task;

pub use runtime::Runtime;
pub use task::{Next, Outcome, Policy, Registry, Task, TaskId, TaskOptions, TaskRecord, TaskState};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use state::{Savepoint, Sqlite, Thread};
use tokio::sync::{broadcast, watch};
use tokio::task::AbortHandle;
use tracing::Instrument;

use runtime::{Invocation, Mode};
use task::Erased;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Crashes in a row after which a task is faulted instead of run again.
    pub restart_limit: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { restart_limit: 5 }
    }
}

/// A handle to the engine; cheap to clone.
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    store: Arc<Sqlite>,
    registry: Registry,
    settings: Settings,
    inner: Mutex<Inner>,
    /// Bumped after every commit and every invocation's end; the scheduler and waits watch it.
    version: watch::Sender<u64>,
    /// What each commit changed, for `subscribe`.
    changes: broadcast::Sender<Arc<Committed>>,
}

/// What one commit changed, as `Engine::subscribe` reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Committed {
    /// Threads with new checkpoints, and which (their positions).
    pub threads: Vec<(String, std::ops::Range<usize>)>,
    /// Documents written or deleted.
    pub docs: Vec<String>,
    /// Tasks written, as they now are.
    pub tasks: Vec<TaskChange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskChange {
    pub id: TaskId,
    pub thread: String,
    pub kind: String,
    /// `TaskState::name`.
    pub state: &'static str,
    /// The checkpoint's `phase` tag; "" once ended.
    pub phase: String,
}

/// How many commits a subscriber may fall behind before it misses some (and is told so).
const CHANGES_BUFFER: usize = 1024;

struct Inner {
    /// Threads loaded so far, as last committed.
    threads: HashMap<String, Thread>,
    /// Every task that hasn't ended, as last committed.
    tasks: BTreeMap<TaskId, TaskRecord>,
    /// Documents read or written so far, as last committed; `None` for absent.
    docs: HashMap<String, Option<Value>>,
    /// Invocations in this process.
    running: HashMap<TaskId, Running>,
    /// Bumped whenever a task commits a new state: a phase that leaves it unchanged made no
    /// durable progress.
    progress: HashMap<TaskId, u64>,
    next_id: TaskId,
    generation: u64,
    scheduler: Option<AbortHandle>,
    closed: bool,
    /// A storage error ends the engine; reopen to recover.
    failed: Option<String>,
}

struct Running {
    generation: u64,
    mode: Mode,
    cancel: watch::Sender<bool>,
    supervisor: Option<AbortHandle>,
    phase: Option<AbortHandle>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Whether `task` is live work that keeps `owner` from finishing.
fn holds_up(task: &TaskRecord, owner: TaskId) -> bool {
    task.owner == Some(owner) && !task.background && !task.state.is_terminal()
}

impl Engine {
    /// Opens the engine over `store` and recovers what a previous process left: tasks it was
    /// running are pending again (or faulted at the restart limit). Nothing runs until `resume`.
    pub fn open(store: Arc<Sqlite>, registry: Registry, settings: Settings) -> Result<Self> {
        store::init(&store)?;
        let (tasks, next_id) =
            store.read(|conn| Ok((store::live(conn)?, store::next_id(conn)?)))?;
        let shared = Arc::new(Shared {
            store,
            registry,
            settings,
            inner: Mutex::new(Inner {
                threads: HashMap::new(),
                tasks: tasks.into_iter().map(|t| (t.id, t)).collect(),
                docs: HashMap::new(),
                running: HashMap::new(),
                progress: HashMap::new(),
                next_id,
                generation: 0,
                scheduler: None,
                closed: false,
                failed: None,
            }),
            version: watch::channel(0).0,
            changes: broadcast::channel(CHANGES_BUFFER).0,
        });
        shared.recover()?;
        Ok(Self { shared })
    }

    /// What every commit from now on changes, in commit order. A subscriber that falls more
    /// than 1024 commits behind gets `RecvError::Lagged` and should re-read what it shows.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Committed>> {
        self.shared.changes.subscribe()
    }

    /// The store, for reads outside the engine (thread lists, recent messages).
    pub fn store(&self) -> &Arc<Sqlite> {
        &self.shared.store
    }

    /// Starts running tasks. Needs a tokio runtime.
    pub fn resume(&self) {
        let mut inner = self.shared.lock();
        if inner.scheduler.is_none() && !inner.closed {
            let handle = tokio::spawn(schedule(self.shared.clone()));
            inner.scheduler = Some(handle.abort_handle());
        }
    }

    /// One atomic commit: threads changed and tasks created or changed in `change` are stored
    /// together. If `change` fails, nothing happens.
    pub fn commit<R>(&self, change: impl FnOnce(&mut Tx) -> Result<R>) -> Result<R> {
        self.shared.commit(change)
    }

    /// Creates a top-level task, or one `options.owner` owns.
    pub fn create_task<T: Task>(
        &self,
        thread: &str,
        input: T::Input,
        options: TaskOptions,
    ) -> Result<TaskId> {
        self.commit(|tx| tx.create_task::<T>(thread, input, options))
    }

    /// Marks the task, and everything it owns, for abort. Wait for it with `wait`.
    pub fn abort_task(&self, id: TaskId) -> Result<()> {
        self.commit(|tx| {
            ensure!(tx.task(id).is_some(), "task {id} is not live");
            tx.mark_abort(id, Some("requested".into()))
        })
    }

    /// The task as last committed.
    pub fn task(&self, id: TaskId) -> Result<Option<TaskRecord>> {
        self.shared.task(id)
    }

    /// Every task that hasn't ended.
    pub fn live_tasks(&self) -> Vec<TaskRecord> {
        self.shared.lock().tasks.values().cloned().collect()
    }

    /// Resolves with the task's outcome once it has ended.
    pub async fn wait(&self, id: TaskId) -> Result<Outcome> {
        self.watch(|engine| match engine.task(id)? {
            Some(task) => Ok(task.state.ended().cloned()),
            None => bail!("no task {id}"),
        })
        .await
    }

    /// Reads a thread as last committed.
    pub fn thread<R>(&self, id: &str, read: impl FnOnce(&Thread) -> R) -> Result<R> {
        self.shared.thread(id, read)
    }

    /// A document as last committed.
    pub fn doc<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value = self.shared.lock().doc(&self.shared.store, key)?;
        Ok(value.map(serde_json::from_value).transpose()?)
    }

    /// Resolves with `check`'s value once it returns one, checking after every change.
    pub async fn watch<R>(&self, mut check: impl FnMut(&Engine) -> Result<Option<R>>) -> Result<R> {
        let mut version = self.shared.version.subscribe();
        loop {
            version.borrow_and_update();
            if let Some(value) = check(self)? {
                return Ok(value);
            }
            self.shared.lock().check_open()?;
            version.changed().await?;
        }
    }

    /// Stops cleanly: running phases are stopped and running tasks stored as pending, so a later
    /// open runs them again without counting a restart. That includes tasks between
    /// invocations, such as one whose abort handler waits on the work it owns.
    pub fn close(&self) -> Result<()> {
        self.shared.stop();
        let pending: Vec<TaskRecord> = self
            .shared
            .lock()
            .tasks
            .values()
            .filter_map(|task| match &task.state {
                TaskState::Running { checkpoint } => Some(TaskRecord {
                    state: TaskState::Pending {
                        checkpoint: checkpoint.clone(),
                    },
                    ..task.clone()
                }),
                _ => None,
            })
            .collect();
        self.shared
            .store
            .transaction(|tx| pending.iter().try_for_each(|task| store::save(tx, task)))
    }

    /// Stops as a crash would: nothing more is written. For tests of recovery.
    #[doc(hidden)]
    pub fn crash(&self) {
        self.shared.stop();
    }
}

impl Inner {
    /// Fails once the engine has stopped.
    fn check_open(&self) -> Result<()> {
        if let Some(error) = &self.failed {
            bail!("engine failed: {error}");
        }
        ensure!(!self.closed, "engine closed");
        Ok(())
    }

    /// The invocation of `id` numbered `generation`, if it still holds the task.
    fn invocation(&mut self, id: TaskId, generation: u64) -> Option<&mut Running> {
        self.running
            .get_mut(&id)
            .filter(|running| running.generation == generation)
    }

    /// A thread as last committed, loaded (or started empty) on first use.
    fn thread(&mut self, store: &Sqlite, id: &str) -> Result<&mut Thread> {
        if !self.threads.contains_key(id) {
            let thread = Thread::load(id, store)?;
            self.threads.insert(id.to_string(), thread);
        }
        Ok(self.threads.get_mut(id).expect("loaded"))
    }

    /// A document as last committed, read on first use.
    fn doc(&mut self, store: &Sqlite, key: &str) -> Result<Option<Value>> {
        if let Some(value) = self.docs.get(key) {
            return Ok(value.clone());
        }
        let value = store.read(|conn| store::get_doc(conn, key))?;
        self.docs.insert(key.to_string(), value.clone());
        Ok(value)
    }
}

impl Shared {
    /// Ignores poisoning: a commit catches panics in its `change` and rolls back, so only a
    /// read callback, which changes nothing, can panic while this is held.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn commit<R>(&self, change: impl FnOnce(&mut Tx) -> Result<R>) -> Result<R> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner.check_open()?;
        let mut tx = Tx {
            shared: self,
            inner,
            savepoints: HashMap::new(),
            tasks: BTreeMap::new(),
            docs: BTreeMap::new(),
            advanced: HashSet::new(),
            aborted: HashSet::new(),
            now: now_ms(),
        };
        // A panic in `change` rolls back like an error, then goes on to the caller (a phase,
        // which is faulted for it).
        let result = panic::catch_unwind(AssertUnwindSafe(|| change(&mut tx)));
        let Tx {
            savepoints,
            tasks,
            docs,
            advanced,
            aborted,
            ..
        } = tx;
        let value = match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                rollback(inner, savepoints);
                return Err(error);
            }
            Err(panic) => {
                rollback(inner, savepoints);
                drop(guard);
                panic::resume_unwind(panic);
            }
        };

        let stored = self.store.transaction(|sql| {
            for id in savepoints.keys() {
                let unsaved = inner.threads[id].unsaved();
                if !unsaved.is_empty() {
                    Sqlite::save_in(sql, id, unsaved)?;
                }
            }
            for task in tasks.values() {
                store::save(sql, task)?;
            }
            for (key, value) in &docs {
                store::put_doc(sql, key, value.as_ref())?;
            }
            Ok(())
        });
        if let Err(error) = stored {
            rollback(inner, savepoints);
            inner.failed = Some(format!("{error:#}"));
            return Err(error);
        }

        let mut committed = Committed::default();
        for id in savepoints.keys() {
            let thread = inner.threads.get_mut(id).expect("touched");
            if let (Some(first), Some(last)) = (thread.unsaved().first(), thread.unsaved().last()) {
                committed.threads.push((id.clone(), first.id..last.id + 1));
            }
            thread.mark_saved();
        }
        for id in advanced {
            *inner.progress.entry(id).or_default() += 1;
        }
        for (id, task) in &tasks {
            let before = inner.tasks.get(id).map(|t| t.state.name());
            if before != Some(task.state.name()) {
                trace_change(task, before);
            }
            committed.tasks.push(TaskChange {
                id: *id,
                thread: task.thread.clone(),
                kind: task.kind.clone(),
                state: task.state.name(),
                phase: task.state.checkpoint().map_or("", phase_name).to_string(),
            });
        }
        committed.docs = docs.keys().cloned().collect();
        for (id, task) in tasks {
            if task.state.is_terminal() {
                inner.tasks.remove(&id);
                inner.progress.remove(&id);
            } else {
                inner.tasks.insert(id, task);
            }
        }
        inner.docs.extend(docs);
        for id in aborted {
            if let Some(running) = inner.running.get(&id).filter(|r| r.mode == Mode::Run) {
                let _ = running.cancel.send(true);
            }
        }
        drop(guard);
        self.version.send_modify(|v| *v += 1);
        if committed != Committed::default() {
            // No subscribers is fine: nobody is listening.
            let _ = self.changes.send(Arc::new(committed));
        }
        Ok(value)
    }

    pub(crate) fn task(&self, id: TaskId) -> Result<Option<TaskRecord>> {
        match self.live(id) {
            Some(task) => Ok(Some(task)),
            None => self.store.read(|conn| store::get(conn, id)),
        }
    }

    pub(crate) fn thread<R>(&self, id: &str, read: impl FnOnce(&Thread) -> R) -> Result<R> {
        let mut inner = self.lock();
        Ok(read(inner.thread(&self.store, id)?))
    }

    fn live(&self, id: TaskId) -> Option<TaskRecord> {
        self.lock().tasks.get(&id).cloned()
    }

    fn progress(&self, id: TaskId) -> u64 {
        self.lock().progress.get(&id).copied().unwrap_or(0)
    }

    /// Lets each task's definition clean up after the engine itself ended it (`Task::on_fault`),
    /// one commit each. Best effort: errors are ignored.
    fn after_faults(&self, ids: &[TaskId]) {
        for &id in ids {
            let Ok(Some(task)) = self.task(id) else {
                continue;
            };
            if let Some(kind) = self.registry.get(&task.kind) {
                let _ = self.commit(|tx| kind.on_fault(tx, &task));
            }
        }
    }

    /// What a crash or an earlier process left: running tasks were cut off.
    fn recover(&self) -> Result<()> {
        let limit = self.settings.restart_limit;
        let mut faulted = Vec::new();
        self.commit(|tx| {
            let ids: Vec<TaskId> = tx.inner.tasks.keys().copied().collect();
            for &id in &ids {
                let task = tx.task(id).expect("live");
                let TaskState::Running { checkpoint } = &task.state else {
                    continue;
                };
                let (checkpoint, restarts) = (checkpoint.clone(), task.restarts + 1);
                tracing::warn!(
                    task = id,
                    thread = %task.thread,
                    kind = %task.kind,
                    phase = phase_name(&checkpoint),
                    restarts,
                    "task was cut off by a crash; it runs its phase again"
                );
                tx.update(id, |task| {
                    task.state = TaskState::Pending { checkpoint };
                    task.restarts = restarts;
                })?;
                if restarts >= limit {
                    let error = format!("restart_limit: cut off by {restarts} crashes in a row");
                    tx.end(id, Outcome::Faulted { error })?;
                    faulted.push(id);
                }
            }
            for id in ids {
                let abandon = tx
                    .task(id)
                    .is_some_and(|t| t.abandon_on_restart && t.state.checkpoint().is_some());
                if abandon {
                    tx.mark_abort(id, Some("restart".into()))?;
                }
            }
            Ok(())
        })?;
        self.after_faults(&faulted);
        Ok(())
    }

    /// Stops the scheduler and every invocation, and refuses further commits.
    fn stop(&self) {
        let mut inner = self.lock();
        inner.closed = true;
        if let Some(scheduler) = inner.scheduler.take() {
            scheduler.abort();
        }
        for (_, running) in inner.running.drain() {
            for handle in [running.supervisor, running.phase].into_iter().flatten() {
                handle.abort();
            }
        }
        drop(inner);
        self.version.send_modify(|v| *v += 1);
    }

    /// Settles what needs no code (owners whose work has ended, orphans, fail-fast aborts),
    /// starts what can run now, and returns when the next sleeper is due.
    fn plan(self: &Arc<Self>) -> Result<Option<u64>> {
        let now = now_ms();
        let mut starts = Vec::new();
        let mut finals = Vec::new();
        let mut orphans = Vec::new();
        let mut fail_fast = Vec::new();
        let mut earliest: Option<u64> = None;
        {
            let inner = self.lock();
            inner.check_open()?;
            let owns_live = |id| inner.tasks.values().any(|t| holds_up(t, id));
            for task in inner.tasks.values() {
                let id = task.id;
                if inner.running.contains_key(&id) {
                    continue;
                }
                // Its outcome is decided and it runs no more code, so it needs no definition.
                if let TaskState::Completing { outcome } = &task.state {
                    if !owns_live(id) {
                        finals.push((id, outcome.clone()));
                    }
                    continue;
                }
                match self.registry.get(&task.kind) {
                    // Blocked until its definition is registered.
                    None => continue,
                    Some(kind) if kind.version() != task.version => {
                        orphans.push(id);
                        continue;
                    }
                    Some(_) => {}
                }
                // The abort handler starts once the work the task owns has ended; after that it
                // runs as any phase does, its own waits and sleeps included.
                if task.abort_requested && !task.aborting {
                    if !owns_live(id) {
                        starts.push((id, Mode::Abort));
                    }
                    continue;
                }
                let mode = if task.aborting {
                    Mode::Abort
                } else {
                    Mode::Run
                };
                match &task.state {
                    TaskState::Pending { .. } | TaskState::Running { .. } => {
                        starts.push((id, mode));
                    }
                    TaskState::Waiting { on, policy, .. } => {
                        let (live, ended): (Vec<TaskId>, Vec<TaskId>) = on
                            .iter()
                            .copied()
                            .partition(|o| inner.tasks.contains_key(o));
                        if live.is_empty() {
                            starts.push((id, mode));
                        } else if *policy == Policy::FailFast && !ended.is_empty() {
                            let unmarked: Vec<TaskId> = live
                                .into_iter()
                                .filter(|o| !inner.tasks[o].abort_requested)
                                .collect();
                            if !unmarked.is_empty() {
                                fail_fast.push((ended, unmarked));
                            }
                        }
                    }
                    TaskState::Sleeping { until, .. } if *until <= now => {
                        starts.push((id, mode));
                    }
                    TaskState::Sleeping { until, .. } => {
                        earliest = Some(earliest.map_or(*until, |e| e.min(*until)));
                    }
                    TaskState::Completing { .. } | TaskState::Terminal { .. } => {}
                }
            }
        }

        // Ended tasks are read from the store, so outside the lock.
        let failed = |id: &TaskId| {
            self.task(*id)
                .ok()
                .flatten()
                .is_some_and(|t| t.state.ended().is_some_and(|o| !o.is_completed()))
        };
        let aborts: Vec<TaskId> = fail_fast
            .into_iter()
            .filter(|(ended, _)| ended.iter().any(&failed))
            .flat_map(|(_, unmarked)| unmarked)
            .collect();
        if !(finals.is_empty() && orphans.is_empty() && aborts.is_empty()) {
            self.commit(|tx| {
                for (id, outcome) in finals {
                    tx.end(id, outcome)?;
                }
                for &id in &orphans {
                    let reason = "task_too_old: stored by another version".to_string();
                    tx.end(id, Outcome::Orphaned { reason })?;
                }
                for id in aborts {
                    tx.mark_abort(id, Some("a task waited on with it failed".into()))?;
                }
                Ok(())
            })?;
            self.after_faults(&orphans);
        }
        for (id, mode) in starts {
            self.start(id, mode)?;
        }
        Ok(earliest)
    }

    /// Reserves the task for an invocation and runs it.
    fn start(self: &Arc<Self>, id: TaskId, mode: Mode) -> Result<()> {
        let (cancel, cancelled) = watch::channel(false);
        let reserved = self.commit(|tx| {
            if tx.inner.running.contains_key(&id) {
                return Ok(None);
            }
            let Some(task) = tx.task(id) else {
                return Ok(None);
            };
            let Some(checkpoint) = task.state.checkpoint().cloned() else {
                return Ok(None);
            };
            let was_running = matches!(task.state, TaskState::Running { .. });
            let kind = self.registry.get(&task.kind).context("unregistered")?;
            let starts_aborting = mode == Mode::Abort && !task.aborting;
            if !was_running || starts_aborting {
                tx.update(id, |task| {
                    task.state = TaskState::Running { checkpoint };
                    task.aborting |= mode == Mode::Abort;
                })?;
            }
            tx.inner.generation += 1;
            let generation = tx.inner.generation;
            let running = Running {
                generation,
                mode,
                cancel,
                supervisor: None,
                phase: None,
            };
            tx.inner.running.insert(id, running);
            Ok(Some((generation, kind)))
        })?;
        let Some((generation, kind)) = reserved else {
            return Ok(());
        };
        let supervisor = tokio::spawn(supervise(
            self.clone(),
            id,
            mode,
            generation,
            kind,
            cancelled,
        ));
        match self.lock().invocation(id, generation) {
            Some(running) => running.supervisor = Some(supervisor.abort_handle()),
            None => supervisor.abort(),
        }
        Ok(())
    }

    fn set_phase(&self, id: TaskId, generation: u64, phase: AbortHandle) {
        match self.lock().invocation(id, generation) {
            Some(running) => running.phase = Some(phase),
            None => phase.abort(),
        }
    }

    fn release(&self, id: TaskId, generation: u64) {
        let mut inner = self.lock();
        if inner.invocation(id, generation).is_some() {
            inner.running.remove(&id);
        }
        drop(inner);
        self.version.send_modify(|v| *v += 1);
    }
}

fn rollback(inner: &mut Inner, savepoints: HashMap<String, Savepoint>) {
    for (id, savepoint) in savepoints {
        if let Some(thread) = inner.threads.get_mut(&id) {
            thread.rollback(savepoint);
        }
    }
}

async fn schedule(shared: Arc<Shared>) {
    let mut version = shared.version.subscribe();
    loop {
        version.borrow_and_update();
        let Ok(next_wake) = shared.plan() else {
            return;
        };
        let wait = next_wake.map_or(Duration::from_secs(3600), |at| {
            Duration::from_millis(at.saturating_sub(now_ms()))
        });
        tokio::select! {
            changed = version.changed() => if changed.is_err() { return },
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Runs a task's phases while it stays running, and faults it when a phase breaks the
/// contract: an error or panic, or returning without durable progress.
async fn supervise(
    shared: Arc<Shared>,
    id: TaskId,
    mode: Mode,
    generation: u64,
    kind: Arc<dyn Erased>,
    cancel: watch::Receiver<bool>,
) {
    while let Some(record) = shared.live(id) {
        if !matches!(record.state, TaskState::Running { .. })
            || (mode == Mode::Run && record.abort_requested)
        {
            break;
        }
        let before = shared.progress(id);
        // Everything traced during the phase carries which task, thread and phase it was.
        let span = tracing::info_span!(
            "phase",
            task = id,
            thread = %record.thread,
            kind = %record.kind,
            phase = record.state.checkpoint().map_or("", phase_name),
            aborting = mode == Mode::Abort,
            restarts = record.restarts,
        );
        let invocation = Arc::new(Invocation {
            shared: shared.clone(),
            record,
            mode,
            generation,
            cancel: cancel.clone(),
        });
        let phase = tokio::spawn(
            match mode {
                Mode::Run => kind.clone().run(invocation),
                Mode::Abort => kind.clone().abort(invocation),
            }
            .instrument(span),
        );
        shared.set_phase(id, generation, phase.abort_handle());
        let error = match phase.await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("{error:#}")),
            // Stopped by a crash or close: leave everything as stored.
            Err(join) if join.is_cancelled() => return,
            Err(join) => Some(panic_message(join)),
        };
        let Some(after) = shared.live(id) else {
            break;
        };
        if mode == Mode::Run && after.abort_requested {
            // The abort handler runs next.
            break;
        }
        let stuck =
            matches!(after.state, TaskState::Running { .. }) && shared.progress(id) == before;
        let error =
            error.or_else(|| stuck.then(|| "phase returned without durable progress".into()));
        if let Some(error) = error {
            if shared
                .commit(|tx| tx.end(id, Outcome::Faulted { error }))
                .is_err()
            {
                return;
            }
            shared.after_faults(&[id]);
            break;
        }
    }
    shared.release(id, generation);
}

/// The `phase` tag of a task's checkpoint (states are tagged enums), or "".
fn phase_name(checkpoint: &Value) -> &str {
    checkpoint["phase"].as_str().unwrap_or("")
}

/// Traces a task's change of state, as committed: how serious depends on how it ended.
fn trace_change(task: &TaskRecord, before: Option<&str>) {
    let (id, thread, kind) = (task.id, task.thread.as_str(), task.kind.as_str());
    let from = before.unwrap_or("new");
    match &task.state {
        TaskState::Terminal { outcome } => match outcome {
            Outcome::Completed { result } => {
                tracing::debug!(task = id, thread, kind, from, result = %result, "task completed")
            }
            Outcome::Failed { error } => {
                tracing::warn!(task = id, thread, kind, from, error, "task failed")
            }
            Outcome::Faulted { error } => {
                tracing::error!(task = id, thread, kind, from, error, "task faulted")
            }
            Outcome::Aborted { reason } => {
                tracing::info!(task = id, thread, kind, from, reason = ?reason, "task aborted")
            }
            Outcome::Orphaned { reason } => {
                tracing::warn!(task = id, thread, kind, from, reason, "task orphaned")
            }
        },
        state => tracing::debug!(
            task = id,
            thread,
            kind,
            from,
            to = state.name(),
            phase = state.checkpoint().map_or("", phase_name),
            restarts = task.restarts,
            abort = task.abort_requested,
            "task state"
        ),
    }
}

fn panic_message(join: tokio::task::JoinError) -> String {
    let panic = join.into_panic();
    let text = panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".to_string());
    format!("panicked: {text}")
}

/// One commit in progress: what it changes is applied only once it is stored.
pub struct Tx<'a> {
    shared: &'a Shared,
    inner: &'a mut Inner,
    savepoints: HashMap<String, Savepoint>,
    /// Tasks this commit writes, as they'll be stored.
    tasks: BTreeMap<TaskId, TaskRecord>,
    /// Documents this commit writes; `None` deletes.
    docs: BTreeMap<String, Option<Value>>,
    /// Tasks whose state this commit changes.
    advanced: HashSet<TaskId>,
    /// Tasks this commit marks for abort.
    aborted: HashSet<TaskId>,
    now: u64,
}

impl Tx<'_> {
    /// Ms since the epoch, fixed for the commit.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// A thread to change in this commit, loaded (or started empty) on first use. Its changes
    /// are stored with the commit, or rolled back if the commit fails.
    pub fn thread(&mut self, id: &str) -> Result<&mut Thread> {
        let thread = self.inner.thread(&self.shared.store, id)?;
        if !self.savepoints.contains_key(id) {
            self.savepoints.insert(id.to_string(), thread.savepoint());
        }
        Ok(thread)
    }

    /// A document: small JSON state stored in the same commits as threads and tasks.
    pub fn doc<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        let value = match self.docs.get(key) {
            Some(staged) => staged.clone(),
            None => self.inner.doc(&self.shared.store, key)?,
        };
        Ok(value.map(serde_json::from_value).transpose()?)
    }

    pub fn put_doc<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        self.docs
            .insert(key.to_string(), Some(serde_json::to_value(value)?));
        Ok(())
    }

    pub fn delete_doc(&mut self, key: &str) {
        self.docs.insert(key.to_string(), None);
    }

    /// A task that hasn't ended before this commit, as this commit leaves it so far.
    pub fn task(&self, id: TaskId) -> Option<&TaskRecord> {
        self.tasks.get(&id).or_else(|| self.inner.tasks.get(&id))
    }

    pub fn create_task<T: Task>(
        &mut self,
        thread: &str,
        input: T::Input,
        options: TaskOptions,
    ) -> Result<TaskId> {
        self.create_task_named(T::NAME, thread, serde_json::to_value(input)?, options)
    }

    /// Creates a task by its registered name, for callers that can't name its type (one
    /// generic over a model, say).
    pub fn create_task_named(
        &mut self,
        name: &str,
        thread: &str,
        input: Value,
        options: TaskOptions,
    ) -> Result<TaskId> {
        let kind = self
            .shared
            .registry
            .get(name)
            .with_context(|| format!("task {name} is not registered"))?;
        if let Some(owner) = options.owner {
            ensure!(
                self.task(owner)
                    .is_some_and(|t| t.state.checkpoint().is_some()),
                "owner {owner} has ended"
            );
        }
        let checkpoint = kind.initial(&input)?;
        let id = self.inner.next_id;
        self.inner.next_id += 1;
        self.tasks.insert(
            id,
            TaskRecord {
                id,
                thread: thread.to_string(),
                kind: name.to_string(),
                version: kind.version(),
                input,
                owner: options.owner,
                background: options.background,
                abandon_on_restart: options.abandon_on_restart,
                abort_requested: false,
                abort_reason: None,
                aborting: false,
                restarts: 0,
                memos: BTreeMap::new(),
                state: TaskState::Pending { checkpoint },
                created_at: self.now,
                ended_at: None,
            },
        );
        Ok(id)
    }

    /// Changes a live task in this commit.
    pub(crate) fn update<R>(
        &mut self,
        id: TaskId,
        change: impl FnOnce(&mut TaskRecord) -> R,
    ) -> Result<R> {
        if !self.tasks.contains_key(&id) {
            let task = self
                .inner
                .tasks
                .get(&id)
                .cloned()
                .with_context(|| format!("task {id} has ended"))?;
            self.tasks.insert(id, task);
        }
        Ok(change(self.tasks.get_mut(&id).expect("staged")))
    }

    /// Fails unless this invocation still holds the task, running, and (for a run) unaborted.
    pub(crate) fn check_invocation(&self, id: TaskId, generation: u64, mode: Mode) -> Result<()> {
        ensure!(
            self.inner
                .running
                .get(&id)
                .is_some_and(|r| r.generation == generation),
            "task {id}: this invocation is over"
        );
        let task = self
            .task(id)
            .with_context(|| format!("task {id} has ended"))?;
        ensure!(
            matches!(task.state, TaskState::Running { .. }),
            "task {id} is not running"
        );
        ensure!(
            mode == Mode::Abort || !task.abort_requested,
            "task {id} is being aborted"
        );
        Ok(())
    }

    /// Commits a task's new state, which counts as durable progress.
    pub(crate) fn advance(&mut self, id: TaskId, state: TaskState) -> Result<()> {
        self.update(id, |task| {
            task.state = state;
            task.restarts = 0;
        })?;
        self.advanced.insert(id);
        Ok(())
    }

    /// Settles a task: terminal, or completing while work it owns is live. An outcome other
    /// than completed aborts that work first.
    pub(crate) fn end(&mut self, id: TaskId, outcome: Outcome) -> Result<()> {
        let owned = self.owned_live(id);
        if !outcome.is_completed() {
            for &child in &owned {
                self.mark_abort(child, Some("owner ended".into()))?;
            }
        }
        if !owned.is_empty() {
            return self.advance(id, TaskState::Completing { outcome });
        }
        let now = self.now;
        self.update(id, |task| task.ended_at = Some(now))?;
        self.advance(id, TaskState::Terminal { outcome })
    }

    /// Aborts a task, and the work it owns, with this commit. Nothing for one that has ended.
    pub fn abort_task(&mut self, id: TaskId, reason: &str) -> Result<()> {
        self.mark_abort(id, Some(reason.to_string()))
    }

    /// Marks a task and the work it owns for abort.
    pub(crate) fn mark_abort(&mut self, id: TaskId, reason: Option<String>) -> Result<()> {
        let Some(task) = self.task(id) else {
            return Ok(());
        };
        if task.abort_requested {
            return Ok(());
        }
        // A completing task keeps its decided outcome; only the work it owns is aborted.
        if task.state.checkpoint().is_some() {
            self.update(id, |task| {
                task.abort_requested = true;
                task.abort_reason = reason.clone();
            })?;
            self.aborted.insert(id);
        }
        for child in self.owned_live(id) {
            self.mark_abort(child, reason.clone())?;
        }
        Ok(())
    }

    /// Live tasks `id` owns that keep it busy (not background).
    fn owned_live(&self, id: TaskId) -> Vec<TaskId> {
        let ids: BTreeSet<TaskId> = self
            .inner
            .tasks
            .keys()
            .chain(self.tasks.keys())
            .copied()
            .collect();
        ids.into_iter()
            .filter(|&child| self.task(child).is_some_and(|t| holds_up(t, id)))
            .collect()
    }
}

#[cfg(test)]
mod tests;
