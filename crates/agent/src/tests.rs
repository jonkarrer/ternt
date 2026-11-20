use super::*;
use anyhow::anyhow;
use context::{Block, Message, Role};
use engine::{Settings as EngineSettings, TaskState};
use serde_json::json;
use state::{Event, Sqlite, Thread};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const T: Duration = Duration::from_secs(5);

enum Reply {
    Text(&'static str),
    Call(&'static str, Value),
    /// Several tool calls in one reply, without arguments.
    Calls(Vec<&'static str>),
    Error(&'static str),
    /// Never answers: stands for a request a crash cuts off.
    Hang,
    /// Answers once notified.
    Gate(Arc<Notify>, &'static str),
}

/// A model that replies from a script and remembers what it was asked.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Context>>,
}

impl Script {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            seen: Mutex::default(),
        })
    }

    fn seen(&self) -> Vec<Context> {
        self.seen.lock().unwrap().clone()
    }
}

fn response(content: Vec<Block>) -> Response {
    Response {
        message: Message {
            role: Role::Assistant,
            content,
        },
        input_tokens: 0,
        output_tokens: 0,
    }
}

impl Model for Script {
    async fn chat(&self, ctx: &Context) -> Result<Response> {
        self.seen.lock().unwrap().push(ctx.clone());
        let reply = self.replies.lock().unwrap().pop_front();
        match reply.expect("script ran out") {
            Reply::Text(text) => Ok(response(vec![Block::Text { text: text.into() }])),
            Reply::Call(name, input) => {
                let n = self.seen.lock().unwrap().len();
                Ok(response(vec![Block::ToolCall {
                    id: format!("call-{n}"),
                    name: name.into(),
                    input,
                }]))
            }
            Reply::Calls(names) => {
                let n = self.seen.lock().unwrap().len();
                let calls = names.iter().enumerate().map(|(i, name)| Block::ToolCall {
                    id: format!("call-{n}-{i}"),
                    name: (*name).into(),
                    input: json!({}),
                });
                Ok(response(calls.collect()))
            }
            Reply::Error(error) => Err(anyhow!(error)),
            Reply::Hang => std::future::pending().await,
            Reply::Gate(gate, text) => {
                gate.notified().await;
                Ok(response(vec![Block::Text { text: text.into() }]))
            }
        }
    }
}

/// A tool for tests: answers `answer`, after `gate` if set, counting its runs.
struct TestTool {
    name: &'static str,
    answer: &'static str,
    replay: Replay,
    gate: Option<Arc<Notify>>,
    hang: bool,
    runs: Arc<AtomicUsize>,
}

impl TestTool {
    fn new(name: &'static str, answer: &'static str) -> Self {
        Self {
            name,
            answer,
            replay: Replay::Unsafe,
            gate: None,
            hang: false,
            runs: Arc::default(),
        }
    }
}

impl Tool for TestTool {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: self.name.into(),
            description: "test".into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn replay(&self) -> Replay {
        self.replay
    }

    fn execute(&self, _: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            self.runs.fetch_add(1, Ordering::SeqCst);
            if self.hang {
                std::future::pending::<()>().await;
            }
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            Ok(self.answer.to_string())
        })
    }
}

fn start(store: &Arc<Sqlite>, script: &Arc<Script>, tools: Tools) -> Agent {
    start_with(store, script, tools, None, fast())
}

fn fast() -> Settings {
    Settings {
        backoff_ms: 20,
        ..Settings::default()
    }
}

fn start_with(
    store: &Arc<Sqlite>,
    script: &Arc<Script>,
    tools: Tools,
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
) -> Agent {
    let mut registry = Registry::new();
    register(&mut registry, script.clone(), tools, mind, settings);
    let engine = Engine::open(store.clone(), registry, EngineSettings::default()).unwrap();
    engine.resume();
    Agent::new(engine)
}

async fn settled(agent: &Agent, id: &str) -> Settled {
    tokio::time::timeout(T, agent.wait(id))
        .await
        .unwrap()
        .unwrap()
}

fn done(answer: &str) -> Settled {
    Settled::Done {
        answer: answer.into(),
    }
}

async fn eventually(check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < T, "timed out");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn live_tools(agent: &Agent) -> Vec<engine::TaskRecord> {
    let mut tasks = agent.engine().live_tasks();
    tasks.retain(|t| t.kind == "agent.tool");
    tasks
}

/// Waits until a tool call is running its tool: a crash now cuts it off.
async fn tool_executing(agent: &Agent) {
    eventually(|| {
        live_tools(agent).iter().any(|t| {
            matches!(t.state, TaskState::Running { .. })
                && t.state
                    .checkpoint()
                    .is_some_and(|c| c["phase"] == "execute")
        })
    })
    .await;
}

/// The thread's messages, as (role, text or a summary of the blocks).
fn transcript(agent: &Agent, thread: &str) -> Vec<String> {
    agent
        .engine()
        .thread(thread, |t| {
            t.context()
                .messages
                .iter()
                .map(|m| {
                    let blocks: Vec<String> = m
                        .content
                        .iter()
                        .map(|b| match b {
                            Block::Text { text } => text.clone(),
                            Block::ToolCall { name, .. } => format!("call {name}"),
                            Block::ToolResult {
                                content, is_error, ..
                            } => format!("{}{content}", if *is_error { "error: " } else { "" }),
                            _ => String::new(),
                        })
                        .collect();
                    format!("{:?}: {}", m.role, blocks.join(" | "))
                })
                .collect()
        })
        .unwrap()
}

fn store() -> Arc<Sqlite> {
    Arc::new(Sqlite::memory().unwrap())
}

#[tokio::test]
async fn answers_an_input() {
    let script = Script::new(vec![Reply::Text("hello")]);
    let agent = start(&store(), &script, Tools::new());
    let id = agent.submit("t1", Input::new("hi")).unwrap();
    assert_eq!(settled(&agent, &id).await, done("hello"));
    assert_eq!(transcript(&agent, "t1"), ["User: hi", "Assistant: hello"]);
    assert!(!agent.engine().thread("t1", |t| t.in_turn()).unwrap());
}

#[tokio::test]
async fn runs_tools_between_requests() {
    let script = Script::new(vec![
        Reply::Call("weather", json!({ "city": "Paris" })),
        Reply::Text("18C and sunny"),
    ]);
    let mut tools = Tools::new();
    tools.add(TestTool::new("weather", "18C"));
    let agent = start(&store(), &script, tools);
    let id = agent.submit("t1", Input::new("Weather in Paris?")).unwrap();
    assert_eq!(settled(&agent, &id).await, done("18C and sunny"));
    assert_eq!(
        transcript(&agent, "t1"),
        [
            "User: Weather in Paris?",
            "Assistant: call weather",
            "User: 18C",
            "Assistant: 18C and sunny"
        ]
    );
    // The second request saw the tool's result.
    assert_eq!(script.seen()[1].messages.len(), 3);
}

#[tokio::test]
async fn a_request_id_submits_once() {
    let script = Script::new(vec![Reply::Text("hello")]);
    let agent = start(&store(), &script, Tools::new());
    let input = Input {
        request_id: Some("greeting-1".into()),
        ..Input::new("hi")
    };
    let first = agent.submit("t1", input.clone()).unwrap();
    let again = agent.submit("t1", input).unwrap();
    assert_eq!(first, again);
    assert_eq!(settled(&agent, &first).await, done("hello"));
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn follow_ups_wait_for_the_answer() {
    let gate = Arc::new(Notify::new());
    let script = Script::new(vec![
        Reply::Gate(gate.clone(), "first"),
        Reply::Text("second"),
    ]);
    let agent = start(&store(), &script, Tools::new());
    let a = agent.submit("t1", Input::new("one")).unwrap();
    eventually(|| script.seen().len() == 1).await;
    let b = agent.submit("t1", Input::new("two")).unwrap();
    assert_eq!(
        agent.submission(&b).unwrap().unwrap().status,
        Status::Queued
    );
    gate.notify_one();
    assert_eq!(settled(&agent, &a).await, done("first"));
    assert_eq!(settled(&agent, &b).await, done("second"));
    assert_eq!(
        transcript(&agent, "t1"),
        [
            "User: one",
            "Assistant: first",
            "User: two",
            "Assistant: second"
        ]
    );
}

#[tokio::test]
async fn steers_join_after_the_tool_round() {
    let gate = Arc::new(Notify::new());
    let script = Script::new(vec![
        Reply::Call("slow", json!({})),
        Reply::Text("done, in metric"),
    ]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        gate: Some(gate.clone()),
        ..TestTool::new("slow", "measured")
    });
    let agent = start(&store(), &script, tools);
    let a = agent.submit("t1", Input::new("measure it")).unwrap();
    eventually(|| script.seen().len() == 1).await;
    let steer = Input {
        when_busy: WhenBusy::Steer,
        ..Input::new("use metric")
    };
    let b = agent.submit("t1", steer).unwrap();
    gate.notify_one();
    assert_eq!(settled(&agent, &a).await, done("done, in metric"));
    // Answered by the same run.
    assert_eq!(settled(&agent, &b).await, done("done, in metric"));
    assert_eq!(
        transcript(&agent, "t1"),
        [
            "User: measure it",
            "Assistant: call slow",
            "User: measured",
            "User: use metric",
            "Assistant: done, in metric"
        ]
    );
}

#[tokio::test]
async fn a_request_cut_off_by_a_crash_is_retried() {
    let store = store();
    let script = Script::new(vec![Reply::Hang]);
    let agent = start(&store, &script, Tools::new());
    let id = agent.submit("t1", Input::new("hi")).unwrap();
    eventually(|| script.seen().len() == 1).await;
    agent.engine().crash();

    let script = Script::new(vec![Reply::Text("recovered")]);
    let agent = start(&store, &script, Tools::new());
    assert_eq!(settled(&agent, &id).await, done("recovered"));
    assert_eq!(script.seen().len(), 1);
    assert_eq!(
        transcript(&agent, "t1"),
        ["User: hi", "Assistant: recovered"]
    );
}

#[tokio::test]
async fn an_unsafe_tool_cut_off_is_not_run_again() {
    let store = store();
    let script = Script::new(vec![Reply::Call("order", json!({}))]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        hang: true,
        ..TestTool::new("order", "ordered")
    });
    let agent = start(&store, &script, tools);
    let id = agent.submit("t1", Input::new("order groceries")).unwrap();
    tool_executing(&agent).await;
    agent.engine().crash();

    let script = Script::new(vec![Reply::Text("I couldn't confirm the order")]);
    let runs = Arc::new(AtomicUsize::new(0));
    let mut tools = Tools::new();
    tools.add(TestTool {
        runs: runs.clone(),
        ..TestTool::new("order", "ordered")
    });
    let agent = start(&store, &script, tools);
    assert_eq!(
        settled(&agent, &id).await,
        done("I couldn't confirm the order")
    );
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let transcript = transcript(&agent, "t1");
    assert!(
        transcript[2].starts_with("User: error: tool order was interrupted"),
        "{transcript:?}"
    );
}

/// A clean close counts no restart, but an unsafe tool it cut off still isn't run again.
#[tokio::test]
async fn an_unsafe_tool_cut_off_by_a_close_is_not_run_again() {
    let store = store();
    let script = Script::new(vec![Reply::Call("order", json!({}))]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        hang: true,
        ..TestTool::new("order", "ordered")
    });
    let agent = start(&store, &script, tools);
    let id = agent.submit("t1", Input::new("order groceries")).unwrap();
    tool_executing(&agent).await;
    agent.engine().close().unwrap();

    let script = Script::new(vec![Reply::Text("I couldn't confirm the order")]);
    let runs = Arc::new(AtomicUsize::new(0));
    let mut tools = Tools::new();
    tools.add(TestTool {
        runs: runs.clone(),
        ..TestTool::new("order", "ordered")
    });
    let agent = start(&store, &script, tools);
    assert_eq!(
        settled(&agent, &id).await,
        done("I couldn't confirm the order")
    );
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_safe_tool_cut_off_runs_again() {
    let store = store();
    let script = Script::new(vec![Reply::Call("lookup", json!({}))]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        hang: true,
        replay: Replay::Safe,
        ..TestTool::new("lookup", "found")
    });
    let agent = start(&store, &script, tools);
    let id = agent.submit("t1", Input::new("look it up")).unwrap();
    tool_executing(&agent).await;
    agent.engine().crash();

    let script = Script::new(vec![Reply::Text("found it")]);
    let runs = Arc::new(AtomicUsize::new(0));
    let mut tools = Tools::new();
    tools.add(TestTool {
        replay: Replay::Safe,
        runs: runs.clone(),
        ..TestTool::new("lookup", "found")
    });
    let agent = start(&store, &script, tools);
    assert_eq!(settled(&agent, &id).await, done("found it"));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(transcript(&agent, "t1")[2], "User: found");
}

#[tokio::test]
async fn model_errors_are_retried_then_unanswered() {
    let script = Script::new(vec![
        Reply::Error("overloaded"),
        Reply::Error("overloaded"),
        Reply::Error("overloaded"),
    ]);
    let agent = start(&store(), &script, Tools::new());
    let id = agent.submit("t1", Input::new("hi")).unwrap();
    assert_eq!(
        settled(&agent, &id).await,
        Settled::Unanswered {
            reason: "model_error: overloaded".into()
        }
    );
    assert_eq!(script.seen().len(), 3);
    // The next input starts a fresh run.
    script
        .replies
        .lock()
        .unwrap()
        .push_back(Reply::Text("better now"));
    let next = agent.submit("t1", Input::new("hi again")).unwrap();
    assert_eq!(settled(&agent, &next).await, done("better now"));
}

#[tokio::test]
async fn an_abort_leaves_the_thread_valid() {
    let script = Script::new(vec![Reply::Call("slow", json!({}))]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        hang: true,
        ..TestTool::new("slow", "never")
    });
    let agent = start(&store(), &script, tools);
    let id = agent.submit("t1", Input::new("go")).unwrap();
    eventually(|| !live_tools(&agent).is_empty()).await;
    let run = agent.submission(&id).unwrap().unwrap().run.unwrap();
    agent.engine().abort_task(run).unwrap();
    assert_eq!(
        settled(&agent, &id).await,
        Settled::Unanswered {
            reason: "aborted".into()
        }
    );
    assert_eq!(
        transcript(&agent, "t1"),
        ["User: go", "Assistant: call slow", "User: error: aborted"]
    );
    assert!(!agent.engine().thread("t1", |t| t.in_turn()).unwrap());
}

/// The whole loop against Claude, through the engine: a tool call and an answer.
#[tokio::test]
#[ignore = "calls Anthropic"]
async fn live_tool_loop() {
    use inference::Effort;
    use inference::providers::Anthropic;

    let key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let model = Arc::new(ProviderModel {
        provider: Provider::<Anthropic>::anthropic(&key),
        request: Request::new("claude-opus-5-5").effort(Effort::Low),
    });
    let weather = TestTool::new("weather", "18C and sunny");
    let spec = weather.spec();
    let mut tools = Tools::new();
    tools.add(weather);
    let mut registry = Registry::new();
    register(&mut registry, model, tools, None, Settings::default());
    let engine = Engine::open(store(), registry, EngineSettings::default()).unwrap();
    engine.resume();
    engine
        .commit(|tx| {
            let thread = tx.thread("t1")?;
            thread.set_system("Use the weather tool to answer.")?;
            thread.register(spec)
        })
        .unwrap();
    let agent = Agent::new(engine);

    let id = agent
        .submit("t1", Input::new("What's the weather in Paris?"))
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(60), agent.wait(&id))
        .await
        .unwrap()
        .unwrap();
    println!("{answer:?}");
    println!("{:#?}", transcript(&agent, "t1"));
    let Settled::Done { answer } = answer else {
        panic!("unanswered");
    };
    assert!(answer.contains("18"));
}

/// Records what it was asked to do. Dreaming notes it in the thread; settling resets the
/// context to a card.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<&'static str>>,
}

impl Mind for Recorder {
    fn consider<'a>(&'a self, _: &'a mut Thread) -> MindFuture<'a> {
        self.calls.lock().unwrap().push("consider");
        Box::pin(async { Ok(()) })
    }

    fn dream<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a> {
        self.calls.lock().unwrap().push("dream");
        Box::pin(async move { thread.routed("dream", "noted") })
    }

    fn settle<'a>(&'a self, thread: &'a mut Thread) -> MindFuture<'a> {
        self.calls.lock().unwrap().push("settle");
        Box::pin(async move {
            let len = thread.context().messages.len();
            thread.replace(0..len, vec![Message::user("card: said hi")])
        })
    }
}

#[tokio::test]
async fn idle_threads_dream_then_settle() {
    let recorder = Arc::new(Recorder::default());
    let settings = Settings {
        dream_after_ms: 30,
        settle_after_ms: 80,
        ..fast()
    };
    let script = Script::new(vec![Reply::Text("hello")]);
    let agent = start_with(
        &store(),
        &script,
        Tools::new(),
        Some(recorder.clone()),
        settings,
    );
    let id = agent.submit("t1", Input::new("hi")).unwrap();
    assert_eq!(settled(&agent, &id).await, done("hello"));
    eventually(|| recorder.calls.lock().unwrap().contains(&"settle")).await;
    eventually(|| transcript(&agent, "t1") == ["User: card: said hi"]).await;
    assert_eq!(
        *recorder.calls.lock().unwrap(),
        ["consider", "dream", "settle"]
    );
    let dreamt = agent
        .engine()
        .thread("t1", |t| {
            t.checkpoints().iter().any(|c| {
                c.event
                    == Event::Routed {
                        by: "dream".into(),
                        reason: "noted".into(),
                    }
            })
        })
        .unwrap();
    assert!(dreamt);
}

#[tokio::test]
async fn a_new_input_cancels_the_idle_countdown() {
    let recorder = Arc::new(Recorder::default());
    let settings = Settings {
        dream_after_ms: 60_000,
        ..fast()
    };
    let script = Script::new(vec![Reply::Text("hello"), Reply::Text("again")]);
    let agent = start_with(
        &store(),
        &script,
        Tools::new(),
        Some(recorder.clone()),
        settings,
    );
    let id = agent.submit("t1", Input::new("hi")).unwrap();
    assert_eq!(settled(&agent, &id).await, done("hello"));
    let engine = agent.engine().clone();
    let idle = || {
        engine
            .live_tasks()
            .into_iter()
            .find(|t| t.kind == "agent.idle")
    };
    eventually(|| idle().is_some_and(|t| matches!(t.state, TaskState::Sleeping { .. }))).await;
    let first_idle = idle().unwrap().id;

    let next = agent.submit("t1", Input::new("hi again")).unwrap();
    assert_eq!(settled(&agent, &next).await, done("again"));
    let ended = tokio::time::timeout(T, agent.engine().wait(first_idle))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(ended, engine::Outcome::Aborted { .. }),
        "{ended:?}"
    );
    // A new countdown for the new idle time, and no dream yet.
    eventually(|| idle().is_some_and(|t| t.id != first_idle)).await;
    assert!(!recorder.calls.lock().unwrap().contains(&"dream"));
}

/// Experience's recall, before the request, through the engine: the model sees the memory.
#[tokio::test]
async fn experience_recalls_before_the_request() {
    use experience::{Experience, Scopes};
    use inference::providers::Anthropic;
    use memory::{Memory, Scope, Write};

    let memory = Arc::new(Memory::in_memory().unwrap());
    memory
        .write(Write::fact(Scope::user("jon"), "The user's name is Jon."))
        .unwrap();
    // Recall is decided in code: this provider is never called.
    let experience = Experience::new(
        Provider::<Anthropic>::anthropic("unused"),
        Request::new("unused"),
        memory,
        Scopes {
            read: vec![Scope::user("jon")],
            write: Scope::user("jon"),
        },
    );
    let script = Script::new(vec![Reply::Text("You're Jon.")]);
    let agent = start_with(
        &store(),
        &script,
        Tools::new(),
        Some(Arc::new(experience)),
        fast(),
    );
    let id = agent.submit("t1", Input::new("What's my name?")).unwrap();
    assert_eq!(settled(&agent, &id).await, done("You're Jon."));
    let asked = script.seen()[0].messages[0].text();
    assert!(
        asked.contains("<recalled>\n- The user's name is Jon."),
        "{asked}"
    );
}

/// `send` routes in code (no model call here) and notes each route in its thread.
#[tokio::test]
async fn send_routes_each_input() {
    use inference::providers::OpenAi;
    use memory::Memory;

    let router = router::Router::new(
        Provider::<OpenAi>::baseten("unused"),
        Request::new("unused"),
        Arc::new(Memory::in_memory().unwrap()),
    );
    let script = Script::new(vec![
        Reply::Text("Kyoto in April, then."),
        Reply::Text("Booked."),
        Reply::Text("Here's a cron job."),
    ]);
    let agent = start(&store(), &script, Tools::new());

    let first = agent
        .send(&router, Input::new("Plan a trip to Kyoto in April."))
        .await
        .unwrap();
    assert_eq!(first.thread, "thread-1");
    settled(&agent, &first.submission).await;
    let second = agent.send(&router, Input::new("yes, do it")).await.unwrap();
    assert_eq!(second.thread, "thread-1");
    settled(&agent, &second.submission).await;
    let third = agent
        .send(
            &router,
            Input::new("Write a cron job that rotates logs nightly."),
        )
        .await
        .unwrap();
    assert_eq!(third.thread, "thread-2");
    assert_eq!(
        third.route.unwrap().reason,
        "shares nothing with any thread"
    );
    settled(&agent, &third.submission).await;

    let routes = agent
        .engine()
        .thread("thread-1", |t| {
            t.checkpoints()
                .iter()
                .filter_map(|c| match &c.event {
                    Event::Routed { by, reason } => Some(format!("{by}: {reason}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        routes,
        [
            "rule: no earlier threads",
            "rule: only makes sense in the last conversation"
        ]
    );
}

/// Two `send`s with one request ID can both route before either commits: the later commit
/// finds the first submission and changes nothing.
#[tokio::test]
async fn a_request_id_routed_twice_submits_once() {
    let script = Script::new(vec![Reply::Text("hello")]);
    let agent = start(&store(), &script, Tools::new());
    let input = Input {
        request_id: Some("r1".into()),
        ..Input::new("hi")
    };
    let first = agent.submit("t1", input.clone()).unwrap();
    let route = router::Route::new(router::By::Rule, "no earlier threads");
    let again = agent.engine().commit(|tx| admit(tx, route, input)).unwrap();
    assert_eq!(
        again,
        Sent {
            thread: "t1".into(),
            submission: first.clone(),
            route: None
        }
    );
    assert_eq!(agent.engine().doc::<String>(LAST_THREAD).unwrap(), None);
    let created = agent.engine().thread("thread-1", |t| t.checkpoints().len());
    assert_eq!(created.unwrap(), 0);
    assert_eq!(settled(&agent, &first).await, done("hello"));
}

/// Everything together, live: the router on a fast model, Experience and the loop on Claude.
/// Told in one thread; asked in a new one, it answers from memory.
#[tokio::test]
#[ignore = "calls Anthropic and Baseten"]
async fn live_remembers_across_threads() {
    use experience::{Experience, Scopes};
    use inference::Effort;
    use inference::providers::{Anthropic, OpenAi};
    use memory::{Memory, Scope};

    let anthropic = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let baseten = std::env::var("BASETEN_API_KEY").expect("set BASETEN_API_KEY");
    let claude = || Request::new("claude-opus-5-5").effort(Effort::Low);
    let memory = Arc::new(Memory::in_memory().unwrap());
    let router = router::Router::new(
        Provider::<OpenAi>::baseten(&baseten),
        Request::new("deepseek-ai/DeepSeek-V4.1-Flash").thinking(false),
        memory.clone(),
    );
    let experience = Experience::new(
        Provider::<Anthropic>::anthropic(&anthropic),
        claude(),
        memory,
        Scopes {
            read: vec![Scope::user("jon")],
            write: Scope::user("jon"),
        },
    );
    let model = Arc::new(ProviderModel {
        provider: Provider::<Anthropic>::anthropic(&anthropic),
        request: claude(),
    });
    let mut registry = Registry::new();
    register(
        &mut registry,
        model,
        Tools::new(),
        Some(Arc::new(experience)),
        Settings::default(),
    );
    let engine = Engine::open(store(), registry, EngineSettings::default()).unwrap();
    engine.resume();
    let agent = Agent::new(engine);
    let wait = |id: String| {
        let agent = agent.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(90), agent.wait(&id))
                .await
                .unwrap()
                .unwrap()
        }
    };

    let told = agent
        .send(
            &router,
            Input::new("Hi! I'm Jon, and I prefer Rust for everything."),
        )
        .await
        .unwrap();
    println!(
        "told: {:?} -> {:?}",
        told.route,
        wait(told.submission.clone()).await
    );

    let asked = agent
        .send(&router, Input::new("New topic: what's my name?"))
        .await
        .unwrap();
    let answer = wait(asked.submission.clone()).await;
    println!("asked: {:?} -> {answer:?}", asked.route);
    println!("{:#?}", transcript(&agent, &asked.thread));
    assert_ne!(asked.thread, told.thread);
    let Settled::Done { answer } = answer else {
        panic!("unanswered");
    };
    assert!(answer.contains("Jon"));
}

/// Panics in its first `consider`, once notified: a fault no handler of the run sees.
struct Faulty {
    gate: Arc<Notify>,
    calls: AtomicUsize,
}

impl Mind for Faulty {
    fn consider<'a>(&'a self, _: &'a mut Thread) -> MindFuture<'a> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            if first {
                self.gate.notified().await;
                panic!("consider broke");
            }
            Ok(())
        })
    }

    fn dream<'a>(&'a self, _: &'a mut Thread) -> MindFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn settle<'a>(&'a self, _: &'a mut Thread) -> MindFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn a_faulted_run_lets_the_queue_move_on() {
    let mind = Arc::new(Faulty {
        gate: Arc::new(Notify::new()),
        calls: AtomicUsize::new(0),
    });
    let script = Script::new(vec![Reply::Text("second")]);
    let agent = start_with(&store(), &script, Tools::new(), Some(mind.clone()), fast());
    let a = agent.submit("t1", Input::new("one")).unwrap();
    eventually(|| mind.calls.load(Ordering::SeqCst) == 1).await;
    let b = agent.submit("t1", Input::new("two")).unwrap();
    mind.gate.notify_one();
    assert!(matches!(
        settled(&agent, &a).await,
        Settled::Unanswered { .. }
    ));
    assert_eq!(settled(&agent, &b).await, done("second"));
    assert_eq!(
        transcript(&agent, "t1"),
        ["User: one", "User: two", "Assistant: second"]
    );
}

#[tokio::test]
async fn an_abort_keeps_the_results_of_finished_tools() {
    let script = Script::new(vec![Reply::Calls(vec!["order", "slow"])]);
    let mut tools = Tools::new();
    tools.add(TestTool::new("order", "ordered"));
    tools.add(TestTool {
        hang: true,
        ..TestTool::new("slow", "never")
    });
    let agent = start(&store(), &script, tools);
    let id = agent.submit("t1", Input::new("order, then wait")).unwrap();
    eventually(|| script.seen().len() == 1 && live_tools(&agent).len() == 1).await;
    let run = agent.submission(&id).unwrap().unwrap().run.unwrap();
    agent.engine().abort_task(run).unwrap();
    settled(&agent, &id).await;
    // The order went through: the model must not be told it didn't.
    assert_eq!(
        transcript(&agent, "t1")[2],
        "User: ordered | error: aborted"
    );
}

#[tokio::test]
async fn the_turn_limit_counts_tool_rounds() {
    let script = Script::new(vec![
        Reply::Call("lookup", json!({})),
        Reply::Call("lookup", json!({})),
    ]);
    let runs = Arc::new(AtomicUsize::new(0));
    let mut tools = Tools::new();
    tools.add(TestTool {
        runs: runs.clone(),
        ..TestTool::new("lookup", "found")
    });
    let settings = Settings {
        max_rounds: 1,
        ..fast()
    };
    let agent = start_with(&store(), &script, tools, None, settings);
    let id = agent.submit("t1", Input::new("go")).unwrap();
    assert_eq!(
        settled(&agent, &id).await,
        Settled::Unanswered {
            reason: "turn_limit".into()
        }
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        transcript(&agent, "t1"),
        [
            "User: go",
            "Assistant: call lookup",
            "User: found",
            "Assistant: call lookup",
            "User: error: not run: turn limit"
        ]
    );
    assert!(!agent.engine().thread("t1", |t| t.in_turn()).unwrap());
}

/// The generation task as a later version would store it: older stored runs are orphaned.
struct NextGeneration;

impl Task for NextGeneration {
    type Input = ();
    type State = ();
    type Output = ();
    const NAME: &'static str = generation::NAME;
    const VERSION: u32 = 2;

    fn initial(&self, _: &()) {}

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        rt.commit(|_| Ok(Some(engine::Next::Complete(()))))
    }
}

#[tokio::test]
async fn a_run_that_ends_abnormally_leaves_the_thread_valid() {
    let store = store();
    let script = Script::new(vec![Reply::Call("slow", json!({}))]);
    let mut tools = Tools::new();
    tools.add(TestTool {
        hang: true,
        ..TestTool::new("slow", "never")
    });
    let agent = start(&store, &script, tools);
    let id = agent.submit("t1", Input::new("go")).unwrap();
    eventually(|| !live_tools(&agent).is_empty()).await;
    agent.engine().crash();

    // Reopened by a version whose runs can't resume this one: it's orphaned mid-round.
    let mut registry = Registry::new();
    registry
        .register(NextGeneration)
        .register(ToolCall::new(Tools::new()))
        .register(Idle::new(None, fast()));
    let engine = Engine::open(store, registry, EngineSettings::default()).unwrap();
    engine.resume();
    let agent = Agent::new(engine);
    assert!(matches!(
        settled(&agent, &id).await,
        Settled::Unanswered { .. }
    ));
    let transcript = transcript(&agent, "t1");
    assert_eq!(transcript.len(), 3, "{transcript:?}");
    assert!(transcript[2].starts_with("User: error: "), "{transcript:?}");
    assert!(!agent.engine().thread("t1", |t| t.in_turn()).unwrap());
}
