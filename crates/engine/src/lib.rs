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
//! bottom-up: owned work is aborted first, then the task's own abort handler runs.

pub mod runtime;
mod store;
pub mod task;

pub use runtime::Runtime;
pub use task::{Next, Outcome, Policy, Registry, Task, TaskId, TaskOptions, TaskRecord, TaskState};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use state::{Savepoint, Sqlite, Thread};
use tokio::sync::watch;
use tokio::task::AbortHandle;

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
}

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

/// A task's next state, with its types erased.
pub(crate) enum Step {
    Continue(Value),
    Wait {
        checkpoint: Value,
        on: Vec<TaskId>,
        policy: Policy,
    },
    Sleep {
        checkpoint: Value,
        until: u64,
    },
    End(Outcome),
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
        });
        shared.recover()?;
        Ok(Self { shared })
    }

    /// The store, for reads outside the engine (thread lists, recent messages).
    pub fn store(&self) -> &Arc<Sqlite> {
        &self.shared.store
    }

    /// Starts running tasks. Needs a tokio runtime.
    pub fn resume(&self) {
        let mut inner = self.shared.inner.lock().unwrap();
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
        let inner = self.shared.inner.lock().unwrap();
        inner.tasks.values().cloned().collect()
    }

    /// Resolves with the task's outcome once it has ended.
    pub async fn wait(&self, id: TaskId) -> Result<Outcome> {
        let mut version = self.shared.version.subscribe();
        loop {
            version.borrow_and_update();
            match self.task(id)? {
                Some(task) if task.state.is_terminal() => {
                    return Ok(task.state.outcome().cloned().expect("terminal"));
                }
                Some(_) => {}
                None => bail!("no task {id}"),
            }
            {
                let inner = self.shared.inner.lock().unwrap();
                if let Some(error) = &inner.failed {
                    bail!("engine failed: {error}");
                }
                ensure!(!inner.closed, "engine closed");
            }
            version.changed().await?;
        }
    }

    /// Reads a thread as last committed.
    pub fn thread<R>(&self, id: &str, read: impl FnOnce(&Thread) -> R) -> Result<R> {
        self.shared.thread(id, read)
    }

    /// A document as last committed.
    pub fn doc<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value = self.shared.doc(key)?;
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
            {
                let inner = self.shared.inner.lock().unwrap();
                if let Some(error) = &inner.failed {
                    bail!("engine failed: {error}");
                }
                ensure!(!inner.closed, "engine closed");
            }
            version.changed().await?;
        }
    }

    /// Stops cleanly: running phases are stopped and their tasks stored as pending, so a later
    /// open runs them again without counting a restart.
    pub fn close(&self) -> Result<()> {
        let running = self.shared.stop();
        let tasks: Vec<TaskRecord> = {
            let inner = self.shared.inner.lock().unwrap();
            running
                .iter()
                .filter_map(|id| inner.tasks.get(id))
                .filter_map(|task| match &task.state {
                    TaskState::Running { checkpoint } => Some(TaskRecord {
                        state: TaskState::Pending {
                            checkpoint: checkpoint.clone(),
                        },
                        ..task.clone()
                    }),
                    _ => None,
                })
                .collect()
        };
        self.shared.store.transaction(|tx| {
            for task in &tasks {
                store::save(tx, task)?;
            }
            Ok(())
        })
    }

    /// Stops as a crash would: nothing more is written. For tests of recovery.
    #[doc(hidden)]
    pub fn crash(&self) {
        self.shared.stop();
    }
}

impl Shared {
    pub(crate) fn commit<R>(&self, change: impl FnOnce(&mut Tx) -> Result<R>) -> Result<R> {
        let mut guard = self.inner.lock().unwrap();
        let inner = &mut *guard;
        ensure!(!inner.closed, "engine closed");
        if let Some(error) = &inner.failed {
            bail!("engine failed: {error}");
        }
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
        let result = change(&mut tx);
        let Tx {
            savepoints,
            tasks,
            docs,
            advanced,
            aborted,
            ..
        } = tx;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                rollback(inner, savepoints);
                return Err(error);
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

        for id in savepoints.keys() {
            inner.threads.get_mut(id).expect("touched").mark_saved();
        }
        for (id, task) in tasks {
            if task.state.is_terminal() {
                inner.tasks.remove(&id);
                inner.progress.remove(&id);
            } else {
                inner.tasks.insert(id, task);
            }
        }
        inner.docs.extend(docs);
        for id in advanced {
            *inner.progress.entry(id).or_default() += 1;
        }
        for id in aborted {
            if let Some(running) = inner.running.get(&id).filter(|r| r.mode == Mode::Run) {
                let _ = running.cancel.send(true);
            }
        }
        drop(guard);
        self.version.send_modify(|v| *v += 1);
        Ok(value)
    }

    pub(crate) fn task(&self, id: TaskId) -> Result<Option<TaskRecord>> {
        if let Some(task) = self.inner.lock().unwrap().tasks.get(&id) {
            return Ok(Some(task.clone()));
        }
        self.store.read(|conn| store::get(conn, id))
    }

    pub(crate) fn thread<R>(&self, id: &str, read: impl FnOnce(&Thread) -> R) -> Result<R> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.threads.contains_key(id) {
            let thread = Thread::load(id, self.store.as_ref())?;
            inner.threads.insert(id.to_string(), thread);
        }
        Ok(read(&inner.threads[id]))
    }

    fn doc(&self, key: &str) -> Result<Option<Value>> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(value) = inner.docs.get(key) {
            return Ok(value.clone());
        }
        let value = self.store.read(|conn| store::get_doc(conn, key))?;
        inner.docs.insert(key.to_string(), value.clone());
        Ok(value)
    }

    fn live(&self, id: TaskId) -> Option<TaskRecord> {
        self.inner.lock().unwrap().tasks.get(&id).cloned()
    }

    fn progress(&self, id: TaskId) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.progress.get(&id).copied().unwrap_or(0)
    }

    /// What a crash or an earlier process left: running tasks were cut off.
    fn recover(&self) -> Result<()> {
        let limit = self.settings.restart_limit;
        self.commit(|tx| {
            let ids: Vec<TaskId> = tx.inner.tasks.keys().copied().collect();
            for &id in &ids {
                let task = tx.task(id).expect("live").clone();
                let TaskState::Running { checkpoint } = task.state else {
                    continue;
                };
                let restarts = task.restarts + 1;
                tx.update(id, |task| {
                    task.state = TaskState::Pending { checkpoint };
                    task.restarts = restarts;
                })?;
                if restarts >= limit {
                    let error = format!("restart_limit: cut off by {restarts} crashes in a row");
                    tx.end(id, Outcome::Faulted { error })?;
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
        })
    }

    /// Stops the scheduler and every invocation, and refuses further commits. Returns the tasks
    /// that were running.
    fn stop(&self) -> Vec<TaskId> {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        if let Some(scheduler) = inner.scheduler.take() {
            scheduler.abort();
        }
        let running: Vec<TaskId> = inner.running.keys().copied().collect();
        for (_, running) in inner.running.drain() {
            for handle in [running.supervisor, running.phase].into_iter().flatten() {
                handle.abort();
            }
        }
        drop(inner);
        self.version.send_modify(|v| *v += 1);
        running
    }

    /// Starts what can run now; returns when the next sleeper is due.
    fn plan(self: &Arc<Self>) -> Result<Option<u64>> {
        let now = now_ms();
        let mut starts = Vec::new();
        let mut finals = Vec::new();
        let mut orphans = Vec::new();
        let mut fail_fast = Vec::new();
        let mut earliest: Option<u64> = None;
        {
            let inner = self.inner.lock().unwrap();
            ensure!(!inner.closed && inner.failed.is_none(), "engine stopped");
            let owns_live = |id: TaskId| {
                inner
                    .tasks
                    .values()
                    .any(|t| t.owner == Some(id) && !t.background)
            };
            for task in inner.tasks.values() {
                let id = task.id;
                if inner.running.contains_key(&id) {
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
                match &task.state {
                    TaskState::Terminal { .. } => {}
                    TaskState::Completing { .. } => {
                        if !owns_live(id) {
                            finals.push(id);
                        }
                    }
                    _ if task.abort_requested => {
                        if !owns_live(id) {
                            starts.push((id, Mode::Abort));
                        }
                    }
                    TaskState::Pending { .. } | TaskState::Running { .. } => {
                        starts.push((id, Mode::Run));
                    }
                    TaskState::Waiting { on, policy, .. } => {
                        let live: Vec<TaskId> = on
                            .iter()
                            .copied()
                            .filter(|o| inner.tasks.contains_key(o))
                            .collect();
                        if live.is_empty() {
                            starts.push((id, Mode::Run));
                        } else if *policy == Policy::FailFast {
                            let unmarked: Vec<TaskId> = live
                                .iter()
                                .copied()
                                .filter(|o| !inner.tasks[o].abort_requested)
                                .collect();
                            let ended: Vec<TaskId> =
                                on.iter().copied().filter(|o| !live.contains(o)).collect();
                            if !unmarked.is_empty() && !ended.is_empty() {
                                fail_fast.push((ended, unmarked));
                            }
                        }
                    }
                    TaskState::Sleeping { until, .. } => {
                        if *until <= now {
                            starts.push((id, Mode::Run));
                        } else {
                            earliest = Some(earliest.map_or(*until, |e| e.min(*until)));
                        }
                    }
                }
            }
        }

        for (ended, unmarked) in fail_fast {
            let failed = ended.iter().any(|id| {
                self.task(*id)
                    .ok()
                    .flatten()
                    .and_then(|t| t.state.outcome().cloned())
                    .is_some_and(|o| !o.is_completed())
            });
            if failed {
                self.commit(|tx| {
                    for id in &unmarked {
                        tx.mark_abort(*id, Some("a task waited on with it failed".into()))?;
                    }
                    Ok(())
                })?;
            }
        }
        for id in orphans {
            self.commit(|tx| {
                let reason = "task_too_old: stored by another version".to_string();
                tx.end(id, Outcome::Orphaned { reason })
            })?;
        }
        for id in finals {
            self.commit(|tx| {
                let Some(TaskState::Completing { outcome }) = tx.task(id).map(|t| t.state.clone())
                else {
                    return Ok(());
                };
                let now = tx.now;
                tx.update(id, |task| {
                    task.state = TaskState::Terminal { outcome };
                    task.ended_at = Some(now);
                })?;
                tx.advanced.insert(id);
                Ok(())
            })?;
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
            let Some(task) = tx.task(id).cloned() else {
                return Ok(None);
            };
            let Some(checkpoint) = task.state.checkpoint().cloned() else {
                return Ok(None);
            };
            if tx.inner.running.contains_key(&id) {
                return Ok(None);
            }
            if !matches!(task.state, TaskState::Running { .. }) {
                tx.update(id, |task| task.state = TaskState::Running { checkpoint })?;
            }
            let kind = self.registry.get(&task.kind).context("unregistered")?;
            tx.inner.generation += 1;
            let generation = tx.inner.generation;
            tx.inner.running.insert(
                id,
                Running {
                    generation,
                    mode,
                    cancel,
                    supervisor: None,
                    phase: None,
                },
            );
            Ok(Some((generation, kind)))
        })?;
        let Some((generation, kind)) = reserved else {
            return Ok(());
        };
        let handle = tokio::spawn(supervise(
            self.clone(),
            id,
            mode,
            generation,
            kind,
            cancelled,
        ));
        let mut inner = self.inner.lock().unwrap();
        match inner.running.get_mut(&id) {
            Some(running) if running.generation == generation => {
                running.supervisor = Some(handle.abort_handle());
            }
            _ => handle.abort(),
        }
        Ok(())
    }

    fn set_phase(&self, id: TaskId, generation: u64, phase: AbortHandle) {
        let mut inner = self.inner.lock().unwrap();
        match inner.running.get_mut(&id) {
            Some(running) if running.generation == generation => running.phase = Some(phase),
            _ => phase.abort(),
        }
    }

    fn release(&self, id: TaskId, generation: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .running
            .get(&id)
            .is_some_and(|r| r.generation == generation)
        {
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
        let invocation = Arc::new(Invocation {
            shared: shared.clone(),
            record,
            mode,
            generation,
            cancel: cancel.clone(),
        });
        let phase = tokio::spawn(match mode {
            Mode::Run => kind.clone().run(invocation),
            Mode::Abort => kind.clone().abort(invocation),
        });
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
        if error.is_some() || stuck {
            let error =
                error.unwrap_or_else(|| "phase returned without durable progress".to_string());
            if shared
                .commit(|tx| tx.end(id, Outcome::Faulted { error }))
                .is_err()
            {
                return;
            }
            break;
        }
    }
    shared.release(id, generation);
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
        if !self.inner.threads.contains_key(id) {
            let thread = Thread::load(id, self.shared.store.as_ref())?;
            self.inner.threads.insert(id.to_string(), thread);
        }
        if !self.savepoints.contains_key(id) {
            let savepoint = self.inner.threads[id].savepoint();
            self.savepoints.insert(id.to_string(), savepoint);
        }
        Ok(self.inner.threads.get_mut(id).expect("loaded"))
    }

    /// A document: small JSON state stored in the same commits as threads and tasks.
    pub fn doc<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        let value = match self.docs.get(key) {
            Some(staged) => staged.clone(),
            None => match self.inner.docs.get(key) {
                Some(cached) => cached.clone(),
                None => {
                    let value = self.shared.store.read(|conn| store::get_doc(conn, key))?;
                    self.inner.docs.insert(key.to_string(), value.clone());
                    value
                }
            },
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
    pub(crate) fn update(
        &mut self,
        id: TaskId,
        change: impl FnOnce(&mut TaskRecord),
    ) -> Result<()> {
        if !self.tasks.contains_key(&id) {
            let task = self
                .inner
                .tasks
                .get(&id)
                .cloned()
                .with_context(|| format!("task {id} has ended"))?;
            self.tasks.insert(id, task);
        }
        change(self.tasks.get_mut(&id).expect("staged"));
        Ok(())
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

    pub(crate) fn advance(&mut self, id: TaskId, step: Step) -> Result<()> {
        let state = match step {
            Step::Continue(checkpoint) => TaskState::Running { checkpoint },
            Step::Wait {
                checkpoint,
                on,
                policy,
            } => TaskState::Waiting {
                checkpoint,
                on,
                policy,
            },
            Step::Sleep { checkpoint, until } => TaskState::Sleeping { checkpoint, until },
            Step::End(outcome) => return self.end(id, outcome),
        };
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
        let now = self.now;
        self.update(id, |task| {
            task.restarts = 0;
            if owned.is_empty() {
                task.state = TaskState::Terminal { outcome };
                task.ended_at = Some(now);
            } else {
                task.state = TaskState::Completing { outcome };
            }
        })?;
        self.advanced.insert(id);
        Ok(())
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
        if task.abort_requested || task.state.checkpoint().is_none() {
            return Ok(());
        }
        self.update(id, |task| {
            task.abort_requested = true;
            task.abort_reason = reason.clone();
        })?;
        self.aborted.insert(id);
        for child in self.owned_live(id) {
            self.mark_abort(child, reason.clone())?;
        }
        Ok(())
    }

    /// Live tasks `id` owns that keep it busy (not background).
    fn owned_live(&self, id: TaskId) -> Vec<TaskId> {
        let mut ids: Vec<TaskId> = self
            .inner
            .tasks
            .keys()
            .chain(self.tasks.keys())
            .copied()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter(|&child| {
                self.task(child)
                    .is_some_and(|t| t.owner == Some(id) && !t.background && !t.state.is_terminal())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
