use super::*;
use anyhow::anyhow;
use context::Message;
use serde::{Deserialize, Serialize};
use state::Checkpointer;
use std::time::Instant;

const T: Duration = Duration::from_secs(5);

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry
        .register(Steps)
        .register(Hang)
        .register(Stuck)
        .register(Throws)
        .register(Child)
        .register(Parent)
        .register(Nap)
        .register(Memo)
        .register(Stubborn)
        .register(Holder)
        .register(Panics)
        .register(Tidy);
    registry
}

fn engine(store: &Arc<Sqlite>) -> Engine {
    engine_with(store, Settings::default())
}

fn engine_with(store: &Arc<Sqlite>, settings: Settings) -> Engine {
    let engine = Engine::open(store.clone(), registry(), settings).unwrap();
    engine.resume();
    engine
}

async fn wait(engine: &Engine, id: TaskId) -> Outcome {
    tokio::time::timeout(T, engine.wait(id))
        .await
        .unwrap()
        .unwrap()
}

/// Polls until `check` holds.
async fn eventually(check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < T, "timed out");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until `check` holds for the task as committed.
async fn until(engine: &Engine, id: TaskId, check: impl Fn(&TaskRecord) -> bool) {
    eventually(|| engine.task(id).unwrap().is_some_and(|t| check(&t))).await;
}

/// Waits until `check` holds for the engine's in-memory state.
async fn until_inner(engine: &Engine, check: impl Fn(&Inner) -> bool) {
    eventually(|| check(&engine.shared.inner.lock().unwrap())).await;
}

fn messages(engine: &Engine, thread: &str) -> Vec<String> {
    engine
        .thread(thread, |t| {
            t.context().messages.iter().map(|m| m.text()).collect()
        })
        .unwrap()
}

/// Two phases: writes its input to the thread, then completes with its restart count.
struct Steps;

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum StepsState {
    Write,
    Finish,
}

impl Task for Steps {
    type Input = String;
    type State = StepsState;
    type Output = u32;
    const NAME: &'static str = "test.steps";

    fn initial(&self, _: &String) -> StepsState {
        StepsState::Write
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        match rt.state() {
            StepsState::Write => rt.commit(|tx| {
                tx.thread(rt.thread_id())?.push(Message::user(rt.input()))?;
                Ok(Some(Next::Continue(StepsState::Finish)))
            }),
            StepsState::Finish => rt.commit(|_| Ok(Some(Next::Complete(rt.restarts())))),
        }
    }
}

/// Writes to the thread, then hangs until a crash; after one, completes.
struct Hang;

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum HangState {
    Write,
    Hang,
}

impl Task for Hang {
    /// Hang even after restarts.
    type Input = bool;
    type State = HangState;
    type Output = u32;
    const NAME: &'static str = "test.hang";

    fn initial(&self, _: &bool) -> HangState {
        HangState::Write
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        match rt.state() {
            HangState::Write => rt.commit(|tx| {
                tx.thread(rt.thread_id())?
                    .push(Message::user("written once"))?;
                Ok(Some(Next::Continue(HangState::Hang)))
            }),
            HangState::Hang if rt.restarts() == 0 || *rt.input() => {
                rt.cancelled().await;
                Err(anyhow!("cancelled"))
            }
            HangState::Hang => rt.commit(|_| Ok(Some(Next::Complete(rt.restarts())))),
        }
    }
}

/// Returns without committing anything.
struct Stuck;

impl Task for Stuck {
    type Input = ();
    type State = ();
    type Output = ();
    const NAME: &'static str = "test.stuck";

    fn initial(&self, _: &()) {}

    async fn run(&self, _: Runtime<Self>) -> Result<()> {
        Ok(())
    }
}

struct Throws;

impl Task for Throws {
    type Input = ();
    type State = ();
    type Output = ();
    const NAME: &'static str = "test.throws";

    fn initial(&self, _: &()) {}

    async fn run(&self, _: Runtime<Self>) -> Result<()> {
        Err(anyhow!("boom"))
    }
}

/// Doubles its input; 0 fails; negative hangs until aborted. Its abort handler writes
/// "aborted <n>" to the thread.
struct Child;

impl Task for Child {
    type Input = i64;
    type State = ();
    type Output = i64;
    const NAME: &'static str = "test.child";

    fn initial(&self, _: &i64) {}

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let n = *rt.input();
        if n < 0 {
            rt.cancelled().await;
            return Err(anyhow!("cancelled"));
        }
        rt.commit(|_| {
            Ok(Some(if n == 0 {
                Next::Fail("zero".into())
            } else {
                Next::Complete(n * 2)
            }))
        })
    }

    async fn abort(&self, rt: Runtime<Self>) -> Result<()> {
        rt.commit(|tx| {
            let text = format!("aborted {}", rt.input());
            tx.thread(rt.thread_id())?.push(Message::user(text))?;
            Ok(Some(Next::Aborted(Some("undone".into()))))
        })
    }
}

/// Spawns a child per input, waits on them, and completes with the sum of their results.
/// With `finish_early`, completes right after spawning instead.
struct Parent;

#[derive(Serialize, Deserialize)]
struct ParentInput {
    children: Vec<i64>,
    policy: Policy,
    finish_early: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum ParentState {
    Spawn,
    Collect { children: Vec<TaskId> },
}

impl Task for Parent {
    type Input = ParentInput;
    type State = ParentState;
    type Output = i64;
    const NAME: &'static str = "test.parent";

    fn initial(&self, _: &ParentInput) -> ParentState {
        ParentState::Spawn
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        match rt.state() {
            ParentState::Spawn => rt.commit(|tx| {
                let mut children = Vec::new();
                for n in &rt.input().children {
                    let options = TaskOptions::owned_by(rt.task_id());
                    children.push(tx.create_task::<Child>(rt.thread_id(), *n, options)?);
                }
                if rt.input().finish_early {
                    return Ok(Some(Next::Complete(0)));
                }
                Ok(Some(Next::Wait {
                    state: ParentState::Collect {
                        children: children.clone(),
                    },
                    on: children,
                    policy: rt.input().policy,
                }))
            }),
            ParentState::Collect { children } => {
                let outcomes = rt.outcomes(children)?;
                let next = match outcomes
                    .iter()
                    .map(|o| o.result::<i64>())
                    .sum::<Result<i64>>()
                {
                    Ok(sum) => Next::Complete(sum),
                    Err(_) => Next::Fail(format!("{outcomes:?}")),
                };
                rt.commit(|_| Ok(Some(next)))
            }
        }
    }

    async fn abort(&self, rt: Runtime<Self>) -> Result<()> {
        rt.commit(|tx| {
            tx.thread(rt.thread_id())?
                .push(Message::user("parent aborted"))?;
            Ok(Some(Next::Aborted(None)))
        })
    }
}

/// Sleeps for its input in ms, then completes with when it woke.
struct Nap;

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum NapState {
    Start,
    Wake,
}

impl Task for Nap {
    type Input = u64;
    type State = NapState;
    type Output = u64;
    const NAME: &'static str = "test.nap";

    fn initial(&self, _: &u64) -> NapState {
        NapState::Start
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        match rt.state() {
            NapState::Start => rt.sleep_until(rt.now() + rt.input(), NapState::Wake),
            NapState::Wake => rt.commit(|_| Ok(Some(Next::Complete(rt.now())))),
        }
    }
}

/// Stores its restart count as a memo, hangs on the first run, and completes with the memo.
struct Memo;

impl Task for Memo {
    type Input = ();
    type State = ();
    type Output = u32;
    const NAME: &'static str = "test.memo";

    fn initial(&self, _: &()) {}

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let first: u32 = rt.memo("first", rt.restarts())?;
        if rt.restarts() == 0 {
            rt.cancelled().await;
            return Err(anyhow!("cancelled"));
        }
        rt.commit(|_| Ok(Some(Next::Complete(first))))
    }
}

fn store() -> Arc<Sqlite> {
    Arc::new(Sqlite::memory().unwrap())
}

#[tokio::test]
async fn runs_phases_to_completion() {
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Steps>("t1", "hello".into(), TaskOptions::default())
        .unwrap();
    assert_eq!(wait(&engine, id).await.result::<u32>().unwrap(), 0);
    assert_eq!(messages(&engine, "t1"), ["hello"]);
    // Stored: a new engine on the same store sees the same.
    engine.close().unwrap();
    let reopened = engine_with(&store, Settings::default());
    assert!(reopened.task(id).unwrap().unwrap().state.is_terminal());
    assert_eq!(messages(&reopened, "t1"), ["hello"]);
    assert!(reopened.live_tasks().is_empty());
}

#[tokio::test]
async fn a_crash_resumes_from_the_last_checkpoint() {
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Hang>("t1", false, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        t.state
            == TaskState::Running {
                checkpoint: serde_json::json!({ "phase": "Hang" }),
            }
    })
    .await;
    engine.crash();

    let engine = engine_with(&store, Settings::default());
    // Ran again from `Hang`, counted as one restart; `Write` didn't run twice.
    assert_eq!(wait(&engine, id).await.result::<u32>().unwrap(), 1);
    assert_eq!(messages(&engine, "t1"), ["written once"]);
}

#[tokio::test]
async fn a_close_is_not_a_crash() {
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Hang>("t1", false, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Running { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    engine.close().unwrap();

    let reopened = Engine::open(store.clone(), registry(), Settings::default()).unwrap();
    let task = reopened.task(id).unwrap().unwrap();
    assert!(matches!(task.state, TaskState::Pending { .. }));
    assert_eq!(task.restarts, 0);
}

#[tokio::test]
async fn work_that_keeps_crashing_is_faulted() {
    let store = store();
    let settings = Settings { restart_limit: 2 };
    let id = engine_with(&store, settings.clone())
        .create_task::<Hang>("t1", true, TaskOptions::default())
        .unwrap();
    for _ in 0..2 {
        let engine = engine_with(&store, settings.clone());
        if engine.task(id).unwrap().unwrap().state.is_terminal() {
            break;
        }
        until(&engine, id, |t| {
            t.state.checkpoint() == Some(&serde_json::json!({ "phase": "Hang" }))
                && matches!(t.state, TaskState::Running { .. })
        })
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        engine.crash();
    }
    let engine = engine_with(&store, settings);
    let Outcome::Faulted { error } = wait(&engine, id).await else {
        panic!("expected faulted");
    };
    assert!(error.starts_with("restart_limit"), "{error}");
}

#[tokio::test]
async fn contract_breaks_are_faulted() {
    let engine = engine(&store());
    let stuck = engine
        .create_task::<Stuck>("t1", (), TaskOptions::default())
        .unwrap();
    let throws = engine
        .create_task::<Throws>("t1", (), TaskOptions::default())
        .unwrap();
    assert_eq!(
        wait(&engine, stuck).await,
        Outcome::Faulted {
            error: "phase returned without durable progress".into()
        }
    );
    assert_eq!(
        wait(&engine, throws).await,
        Outcome::Faulted {
            error: "boom".into()
        }
    );
}

#[tokio::test]
async fn a_failed_commit_changes_nothing() {
    let store = store();
    let engine = engine(&store);
    engine
        .commit(|tx| {
            tx.thread("t1")?.push(Message::user("kept"))?;
            Ok(())
        })
        .unwrap();
    let failed = engine.commit(|tx| {
        tx.thread("t1")?.push(Message::user("dropped"))?;
        tx.create_task::<Steps>("t1", "never".into(), TaskOptions::default())?;
        Err::<(), _>(anyhow!("changed my mind"))
    });
    assert!(failed.is_err());
    assert_eq!(messages(&engine, "t1"), ["kept"]);
    assert!(engine.live_tasks().is_empty());
    assert_eq!(store.load("t1").unwrap().len(), 1);
}

#[tokio::test]
async fn waits_on_children() {
    let engine = engine(&store());
    let input = ParentInput {
        children: vec![1, 2, 3],
        policy: Policy::AllSettled,
        finish_early: false,
    };
    let id = engine
        .create_task::<Parent>("t1", input, TaskOptions::default())
        .unwrap();
    assert_eq!(wait(&engine, id).await.result::<i64>().unwrap(), 12);
}

#[tokio::test]
async fn fail_fast_aborts_the_rest() {
    let engine = engine(&store());
    let input = ParentInput {
        children: vec![-1, 0],
        policy: Policy::FailFast,
        finish_early: false,
    };
    let id = engine
        .create_task::<Parent>("t1", input, TaskOptions::default())
        .unwrap();
    let Outcome::Failed { error } = wait(&engine, id).await else {
        panic!("expected failed");
    };
    assert!(
        error.contains("zero") && error.contains("undone"),
        "{error}"
    );
    assert_eq!(messages(&engine, "t1"), ["aborted -1"]);
}

#[tokio::test]
async fn aborts_run_bottom_up() {
    let engine = engine(&store());
    let input = ParentInput {
        children: vec![-1, -2],
        policy: Policy::AllSettled,
        finish_early: false,
    };
    let id = engine
        .create_task::<Parent>("t1", input, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Waiting { .. })
    })
    .await;
    engine.abort_task(id).unwrap();
    assert_eq!(wait(&engine, id).await, Outcome::Aborted { reason: None });
    let mut written = messages(&engine, "t1");
    // Both children undid their work before the parent's handler ran.
    assert_eq!(written.pop().unwrap(), "parent aborted");
    written.sort();
    assert_eq!(written, ["aborted -1", "aborted -2"]);
}

#[tokio::test]
async fn completes_once_owned_work_ends() {
    let engine = engine(&store());
    let input = ParentInput {
        children: vec![-1],
        policy: Policy::AllSettled,
        finish_early: true,
    };
    let id = engine
        .create_task::<Parent>("t1", input, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Completing { .. })
    })
    .await;
    let child = engine
        .live_tasks()
        .into_iter()
        .find(|t| t.owner == Some(id))
        .unwrap();
    engine.abort_task(child.id).unwrap();
    assert_eq!(
        wait(&engine, id).await,
        Outcome::Completed {
            result: serde_json::json!(0)
        }
    );
}

#[tokio::test]
async fn sleeps_survive_a_crash() {
    let store = store();
    let engine = engine(&store);
    let started = now_ms();
    let id = engine
        .create_task::<Nap>("t1", 150, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Sleeping { .. })
    })
    .await;
    engine.crash();

    let engine = engine_with(&store, Settings::default());
    let woke: u64 = wait(&engine, id).await.result().unwrap();
    assert!(woke >= started + 150);
    // A sleeper wasn't running, so the crash isn't counted against it.
    assert_eq!(engine.task(id).unwrap().unwrap().restarts, 0);
}

#[tokio::test]
async fn memos_keep_the_first_value() {
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Memo>("t1", (), TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| t.memos.contains_key("first")).await;
    engine.crash();

    let engine = engine_with(&store, Settings::default());
    // The rerun offered 1; the stored 0 won.
    assert_eq!(wait(&engine, id).await.result::<u32>().unwrap(), 0);
}

#[tokio::test]
async fn abandoned_on_restart() {
    let store = store();
    let engine = engine(&store);
    let options = TaskOptions {
        abandon_on_restart: true,
        ..TaskOptions::default()
    };
    let id = engine.create_task::<Child>("t1", -5, options).unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Running { .. })
    })
    .await;
    engine.crash();

    let engine = engine_with(&store, Settings::default());
    assert_eq!(
        wait(&engine, id).await,
        Outcome::Aborted {
            reason: Some("undone".into())
        }
    );
    assert_eq!(messages(&engine, "t1"), ["aborted -5"]);
}

#[tokio::test]
async fn docs_commit_with_everything_else() {
    let store = store();
    let engine = engine(&store);
    engine
        .commit(|tx| {
            assert_eq!(tx.doc::<u32>("count")?, None);
            tx.put_doc("count", &1u32)?;
            assert_eq!(tx.doc::<u32>("count")?, Some(1));
            Ok(())
        })
        .unwrap();
    let failed = engine.commit(|tx| {
        tx.put_doc("count", &2u32)?;
        tx.delete_doc("count");
        Err::<(), _>(anyhow!("no"))
    });
    assert!(failed.is_err());
    assert_eq!(engine.doc::<u32>("count").unwrap(), Some(1));

    let watched = engine.clone();
    let waiter = tokio::spawn(async move {
        watched
            .watch(|e| Ok(e.doc::<u32>("count")?.filter(|n| *n == 3)))
            .await
    });
    engine.commit(|tx| tx.put_doc("count", &3u32)).unwrap();
    assert_eq!(
        tokio::time::timeout(T, waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        3
    );

    engine.close().unwrap();
    let reopened = engine_with(&store, Settings::default());
    assert_eq!(reopened.doc::<u32>("count").unwrap(), Some(3));
    reopened
        .commit(|tx| {
            tx.delete_doc("count");
            Ok(())
        })
        .unwrap();
    assert_eq!(reopened.doc::<u32>("count").unwrap(), None);
}

/// Hangs until aborted; its abort handler never returns.
struct Stubborn;

impl Task for Stubborn {
    type Input = ();
    type State = ();
    type Output = ();
    const NAME: &'static str = "test.stubborn";

    fn initial(&self, _: &()) {}

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        rt.cancelled().await;
        Err(anyhow!("cancelled"))
    }

    async fn abort(&self, _: Runtime<Self>) -> Result<()> {
        std::future::pending().await
    }
}

/// Spawns a `Stubborn` it owns, then hangs until aborted.
struct Holder;

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum HolderState {
    Spawn,
    Hold,
}

impl Task for Holder {
    type Input = ();
    type State = HolderState;
    type Output = ();
    const NAME: &'static str = "test.holder";

    fn initial(&self, _: &()) -> HolderState {
        HolderState::Spawn
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        match rt.state() {
            HolderState::Spawn => rt.commit(|tx| {
                let options = TaskOptions::owned_by(rt.task_id());
                tx.create_task::<Stubborn>(rt.thread_id(), (), options)?;
                Ok(Some(Next::Continue(HolderState::Hold)))
            }),
            HolderState::Hold => {
                rt.cancelled().await;
                Err(anyhow!("cancelled"))
            }
        }
    }
}

/// Writes to the thread inside a commit, then panics there.
struct Panics;

impl Task for Panics {
    type Input = ();
    type State = ();
    type Output = ();
    const NAME: &'static str = "test.panics";

    fn initial(&self, _: &()) {}

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        rt.commit(|tx| {
            tx.thread(rt.thread_id())?.push(Message::user("never"))?;
            panic!("in commit")
        })
    }
}

/// `Parent` at a newer version.
struct ParentV2;

impl Task for ParentV2 {
    type Input = ParentInput;
    type State = ParentState;
    type Output = i64;
    const NAME: &'static str = "test.parent";
    const VERSION: u32 = 2;

    fn initial(&self, _: &ParentInput) -> ParentState {
        ParentState::Spawn
    }

    async fn run(&self, _: Runtime<Self>) -> Result<()> {
        Err(anyhow!("never runs"))
    }
}

#[tokio::test]
async fn a_completing_task_needs_no_definition() {
    for with_v2 in [false, true] {
        let store = store();
        let engine = engine(&store);
        let input = ParentInput {
            children: vec![-1],
            policy: Policy::AllSettled,
            finish_early: true,
        };
        let id = engine
            .create_task::<Parent>("t1", input, TaskOptions::default())
            .unwrap();
        until(&engine, id, |t| {
            matches!(t.state, TaskState::Completing { .. })
        })
        .await;
        engine.close().unwrap();

        // Its outcome is decided, so neither a missing nor a newer definition changes it.
        let mut registry = Registry::new();
        registry.register(Child);
        if with_v2 {
            registry.register(ParentV2);
        }
        let engine = Engine::open(store.clone(), registry, Settings::default()).unwrap();
        engine.resume();
        let child = engine
            .live_tasks()
            .into_iter()
            .find(|t| t.owner == Some(id))
            .unwrap();
        until(&engine, child.id, |t| {
            matches!(t.state, TaskState::Running { .. })
        })
        .await;
        assert!(!engine.task(child.id).unwrap().unwrap().abort_requested);
        engine.abort_task(child.id).unwrap();
        assert_eq!(
            wait(&engine, id).await,
            Outcome::Completed {
                result: serde_json::json!(0)
            }
        );
    }
}

#[tokio::test]
async fn aborting_a_completing_task_aborts_what_it_owns() {
    let engine = engine(&store());
    let input = ParentInput {
        children: vec![-1],
        policy: Policy::AllSettled,
        finish_early: true,
    };
    let id = engine
        .create_task::<Parent>("t1", input, TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Completing { .. })
    })
    .await;
    engine.abort_task(id).unwrap();
    // The outcome was decided; the owned work is undone.
    assert_eq!(
        wait(&engine, id).await,
        Outcome::Completed {
            result: serde_json::json!(0)
        }
    );
    assert_eq!(messages(&engine, "t1"), ["aborted -1"]);
}

#[tokio::test]
async fn a_panic_in_a_commit_is_faulted_and_rolled_back() {
    let engine = engine(&store());
    let id = engine
        .create_task::<Panics>("t1", (), TaskOptions::default())
        .unwrap();
    assert_eq!(
        wait(&engine, id).await,
        Outcome::Faulted {
            error: "panicked: in commit".into()
        }
    );
    assert!(messages(&engine, "t1").is_empty());
    // The engine still works.
    let id = engine
        .create_task::<Steps>("t1", "after".into(), TaskOptions::default())
        .unwrap();
    assert!(wait(&engine, id).await.is_completed());
}

#[tokio::test]
async fn ended_tasks_leave_no_bookkeeping() {
    let engine = engine(&store());
    let id = engine
        .create_task::<Steps>("t1", "hello".into(), TaskOptions::default())
        .unwrap();
    wait(&engine, id).await;
    until_inner(&engine, |inner| inner.running.is_empty()).await;
    let inner = engine.shared.inner.lock().unwrap();
    assert!(inner.progress.is_empty(), "{:?}", inner.progress);
}

#[tokio::test]
async fn a_close_counts_no_restart_for_an_owner_waiting_to_abort() {
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Holder>("t1", (), TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        t.state.checkpoint() == Some(&serde_json::json!({ "phase": "Hold" }))
            && matches!(t.state, TaskState::Running { .. })
    })
    .await;
    engine.abort_task(id).unwrap();
    // Its phase stops; its abort handler then waits on the child's, which never ends.
    until_inner(&engine, |inner| !inner.running.contains_key(&id)).await;
    engine.close().unwrap();

    let reopened = Engine::open(store.clone(), registry(), Settings::default()).unwrap();
    let task = reopened.task(id).unwrap().unwrap();
    assert!(task.abort_requested);
    assert_eq!(task.restarts, 0);
}

/// Runs until aborted; its abort handler takes two steps, sleeping between them.
struct Tidy;

static TIDY_ABORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[derive(Serialize, Deserialize)]
#[serde(tag = "phase")]
enum TidyState {
    Working,
    CleanUp,
}

impl Task for Tidy {
    type Input = ();
    type State = TidyState;
    type Output = ();
    const NAME: &'static str = "test.tidy";

    fn initial(&self, _: &()) -> TidyState {
        TidyState::Working
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        rt.cancelled().await;
        Err(anyhow!("cancelled"))
    }

    async fn abort(&self, rt: Runtime<Self>) -> Result<()> {
        TIDY_ABORTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match rt.state() {
            TidyState::Working => rt.sleep_until(rt.now() + 100, TidyState::CleanUp),
            TidyState::CleanUp => rt.commit(|_| Ok(Some(Next::Aborted(Some("cleaned".into()))))),
        }
    }
}

/// An abort handler that sleeps is parked like any sleeper, not rerun at once, and a crash
/// during its sleep resumes the abort, not the run.
#[tokio::test]
async fn abort_handlers_can_sleep() {
    use std::sync::atomic::Ordering;
    let store = store();
    let engine = engine(&store);
    let id = engine
        .create_task::<Tidy>("t1", (), TaskOptions::default())
        .unwrap();
    until(&engine, id, |t| {
        matches!(t.state, TaskState::Running { .. })
    })
    .await;
    engine.abort_task(id).unwrap();
    until(&engine, id, |t| {
        t.aborting && matches!(t.state, TaskState::Sleeping { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    // Parked: the handler ran once, and isn't spinning.
    assert_eq!(TIDY_ABORTS.load(Ordering::SeqCst), 1);
    engine.crash();

    let engine = engine_with(&store, Settings::default());
    assert_eq!(
        wait(&engine, id).await,
        Outcome::Aborted {
            reason: Some("cleaned".into())
        }
    );
    assert_eq!(TIDY_ABORTS.load(Ordering::SeqCst), 2);
}

/// A tasks table from before `aborting` existed gains the column on open.
#[tokio::test]
async fn older_task_tables_gain_the_aborting_column() {
    let store = store();
    store
        .transaction(|tx| {
            tx.execute_batch(
                "CREATE TABLE tasks (
                     id INTEGER PRIMARY KEY, thread TEXT NOT NULL, kind TEXT NOT NULL,
                     version INTEGER NOT NULL, input TEXT NOT NULL, owner INTEGER,
                     background INTEGER NOT NULL, abandon_on_restart INTEGER NOT NULL,
                     abort_requested INTEGER NOT NULL, abort_reason TEXT,
                     restarts INTEGER NOT NULL, memos TEXT NOT NULL, status TEXT NOT NULL,
                     state TEXT NOT NULL, created_at INTEGER NOT NULL, ended_at INTEGER
                 );",
            )?;
            Ok(())
        })
        .unwrap();
    let engine = engine(&store);
    let id = engine
        .create_task::<Steps>("t1", "hello".into(), TaskOptions::default())
        .unwrap();
    assert!(wait(&engine, id).await.is_completed());
}

/// Background work is a boundary: its owner's abort doesn't reach it or wait for it.
#[tokio::test]
async fn background_work_outlives_its_owner() {
    let engine = engine(&store());
    let owner = engine
        .create_task::<Hang>("t1", true, TaskOptions::default())
        .unwrap();
    let child = engine
        .create_task::<Child>(
            "t1",
            -1,
            TaskOptions {
                background: true,
                ..TaskOptions::owned_by(owner)
            },
        )
        .unwrap();
    until(&engine, child, |t| {
        matches!(t.state, TaskState::Running { .. })
    })
    .await;
    engine.abort_task(owner).unwrap();
    assert!(matches!(
        wait(&engine, owner).await,
        Outcome::Aborted { .. }
    ));
    let child = engine.task(child).unwrap().unwrap();
    assert!(!child.abort_requested && matches!(child.state, TaskState::Running { .. }));
}

#[tokio::test]
async fn subscribers_hear_what_each_commit_changed() {
    let engine = engine(&store());
    let mut changes = engine.subscribe();
    engine
        .commit(|tx| {
            tx.thread("t1")?.push(Message::user("one"))?;
            tx.thread("t1")?.push(Message::user("two"))?;
            tx.put_doc("greeting", &"hi")
        })
        .unwrap();
    let id = engine
        .create_task::<Steps>("t1", "three".into(), TaskOptions::default())
        .unwrap();

    let first = changes.recv().await.unwrap();
    assert_eq!(first.threads, [("t1".to_string(), 0..2)]);
    assert_eq!(first.docs, ["greeting"]);
    assert!(first.tasks.is_empty());
    let created = changes.recv().await.unwrap();
    assert_eq!(created.tasks[0].id, id);
    assert_eq!(created.tasks[0].state, "pending");
    assert_eq!(created.tasks[0].kind, "test.steps");
    wait(&engine, id).await;
    while changes.try_recv().is_ok() {}
    // A commit that only reads changes nothing, and says nothing.
    engine.commit(|tx| tx.thread("t1").map(drop)).unwrap();
    assert!(changes.try_recv().is_err());
}
