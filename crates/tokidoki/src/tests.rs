use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use agent::{ToolFuture, Tools};
use context::{Block, Context, Message, Role as CtxRole};
use inference::Response;
use inference::providers::OpenAi;
use interface::{Change, Content, Event, Part, Speaker, Status};
use serde_json::{Value, json};

const T: Duration = Duration::from_secs(5);

enum Reply {
    Text(&'static str),
    /// Written word by word, for whoever watches.
    Streamed(&'static str),
    Call(&'static str, Value),
    Hang,
}

/// A model that replies from a script and remembers the contexts it saw.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Context>>,
}

impl Model for Script {
    async fn chat(&self, ctx: &Context) -> Result<Response> {
        self.seen.lock().unwrap().push(ctx.clone());
        let reply = self.replies.lock().unwrap().pop_front();
        let content = match reply {
            Some(Reply::Text(text) | Reply::Streamed(text)) => {
                vec![Block::Text { text: text.into() }]
            }
            Some(Reply::Call(name, input)) => vec![Block::ToolCall {
                id: "call-1".into(),
                name: name.into(),
                input,
            }],
            Some(Reply::Hang) => std::future::pending().await,
            None => anyhow::bail!("the script ran out"),
        };
        Ok(Response {
            message: Message {
                role: CtxRole::Assistant,
                content,
            },
            input_tokens: 0,
            output_tokens: 0,
            stop: inference::Stop::Done,
        })
    }

    async fn stream(
        &self,
        ctx: &Context,
        on_delta: inference::providers::OnDelta<'_>,
    ) -> Result<Response> {
        if let Some(Reply::Streamed(text)) = self.replies.lock().unwrap().front() {
            for word in text.split_inclusive(' ') {
                on_delta(inference::Delta::Text(word.into()));
            }
        }
        self.chat(ctx).await
    }
}

struct Weather;

impl agent::Tool for Weather {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "weather".into(),
            description: "The weather in a city.".into(),
            input_schema: json!({ "type": "object", "properties": { "city": { "type": "string" } } }),
        }
    }

    fn execute(&self, _: Value) -> ToolFuture<'_> {
        Box::pin(async { Ok("18C".to_string()) })
    }
}

fn start(replies: Vec<Reply>) -> (Tokidoki, Arc<Script>) {
    let (parts, script) = parts(replies, Sqlite::memory().unwrap());
    (Tokidoki::build(parts).unwrap(), script)
}

/// What `start` puts together, keeping its threads in `store`.
fn parts(replies: Vec<Reply>, store: Sqlite) -> (Parts<Script>, Arc<Script>) {
    let script = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        ..Script::default()
    });
    let memory = Arc::new(Memory::in_memory().unwrap());
    // Nobody answers there: routing must decide in code.
    let nowhere =
        Provider::new(OpenAi, "Nowhere", "http://127.0.0.1:9/", vec![]).any(AnyApi::OpenAi);
    let router = Router::new(nowhere, Request::new("none"), memory.clone(), vec![]);
    let mut tools = Tools::new();
    tools.add(Weather);
    let parts = Parts {
        store: Arc::new(store),
        model: script.clone(),
        mind: None,
        router,
        memory,
        scope: Scope::user("owner"),
        tools,
        system: Some("Be brief.".into()),
        settings: agent::Settings {
            backoff_ms: 10,
            ..agent::Settings::default()
        },
        speakers: vec!["owner".into()],
        journal: None,
    };
    (parts, script)
}

fn me(text: &str) -> Input {
    Input::text(Speaker::owner(), text)
}

async fn settled(tokidoki: &Tokidoki, id: &str) -> Submission {
    tokio::time::timeout(T, tokidoki.wait(id))
        .await
        .unwrap()
        .unwrap()
}

/// Events until (and with) the first that `last` matches.
async fn until(events: &mut Events, last: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(T, events.recv())
            .await
            .unwrap()
            .unwrap();
        let done = last(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn answered(event: &Event) -> bool {
    matches!(event, Event::Submission { submission } if submission.is_settled())
}

#[tokio::test]
async fn an_input_is_routed_answered_and_followed() {
    let (tokidoki, script) = start(vec![
        Reply::Call("weather", json!({ "city": "Paris" })),
        Reply::Text("18C in Paris."),
    ]);
    let mut events = tokidoki.events();
    let sent = tokidoki.send(me("Weather in Paris?")).await.unwrap();
    let route = sent.route.clone().unwrap();
    assert!(route.new);
    assert_eq!(route.by, "rule");

    let seen = until(&mut events, answered).await;
    let Some(Event::Submission { submission }) = seen.last() else {
        unreachable!()
    };
    assert_eq!(submission.status, Status::Done);
    assert_eq!(submission.answer.as_deref(), Some("18C in Paris."));
    // Each status as it was committed, in order: never a later one read early.
    let statuses: Vec<Status> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Submission { submission } if submission.id == sent.submission => {
                Some(submission.status)
            }
            _ => None,
        })
        .collect();
    assert_eq!(statuses, [Status::Placed, Status::Done]);

    // The thread's checkpoints arrive in order, without gaps, as history has them.
    let seqs: Vec<usize> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Checkpoint { thread, seq, .. } if *thread == sent.thread => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, (0..seqs.len()).collect::<Vec<_>>());
    let history = tokidoki.history(&sent.thread, 0).unwrap();
    assert_eq!(history.len(), seqs.len());
    let changes: Vec<&Change> = history
        .iter()
        .map(|e| match e {
            Event::Checkpoint { change, .. } => change,
            _ => unreachable!(),
        })
        .collect();
    assert!(matches!(changes[0], Change::System { .. }));
    assert!(changes.iter().any(|c| matches!(c, Change::Routed { .. })));
    let tool_result = changes.iter().any(|c| {
        matches!(c, Change::Message { content, .. }
            if matches!(&content[..], [Content::ToolResult { content, .. }] if content == "18C"))
    });
    assert!(tool_result, "{changes:#?}");
    assert!(matches!(changes.last(), Some(Change::TurnEnded)));
    assert_eq!(tokidoki.history(&sent.thread, 100).unwrap(), []);

    // What worked on it was reported as it went.
    assert!(seen.iter().any(
        |e| matches!(e, Event::Activity { doing: interface::Doing::Tool { name }, .. } if name == "weather")
    ));
    let thinking = |e: &Event| {
        matches!(
            e,
            Event::Activity {
                doing: interface::Doing::Thinking,
                ..
            }
        )
    };
    assert_eq!(
        seen.iter().filter(|e| thinking(e)).count(),
        2,
        "one per model request"
    );
    // The model saw the thread's setup.
    assert_eq!(
        script.seen.lock().unwrap()[0].system.as_deref(),
        Some("Be brief.")
    );

    let threads = tokidoki.threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title, "Weather in Paris?");
}

#[tokio::test]
async fn naming_a_thread_skips_the_router() {
    let (tokidoki, _) = start(vec![Reply::Text("one"), Reply::Text("two")]);
    let first = tokidoki.send(me("hi").to("mine")).await.unwrap();
    assert_eq!(first.thread, "mine");
    assert_eq!(first.route, None);
    settled(&tokidoki, &first.submission).await;
    let second = tokidoki.send(me("again").to("mine")).await.unwrap();
    assert_eq!(
        settled(&tokidoki, &second.submission)
            .await
            .answer
            .as_deref(),
        Some("two")
    );
}

#[tokio::test]
async fn refuses_strangers_and_senses_it_cant_use_yet() {
    let (tokidoki, script) = start(vec![]);
    let refused = |error: anyhow::Error| error.downcast_ref::<Rejected>().cloned().unwrap().0;

    let stranger = Input::text(Speaker::new("discord:42"), "hi");
    let error = tokidoki.send(stranger).await.unwrap_err();
    assert_eq!(refused(error), "discord:42 may not speak to this tokidoki");

    let image = Input {
        parts: vec![Part::Image {
            media_type: "image/png".into(),
            data: interface::Media::Url("https://example.com/a.png".into()),
        }],
        ..me("")
    };
    assert_eq!(
        refused(tokidoki.send(image).await.unwrap_err()),
        "image inputs aren't supported yet"
    );
    assert_eq!(
        refused(tokidoki.send(me("  ")).await.unwrap_err()),
        "the input is empty"
    );
    assert!(script.seen.lock().unwrap().is_empty());
    assert!(tokidoki.threads().unwrap().is_empty());
}

#[tokio::test]
async fn abort_stops_a_run_and_feedback_is_recorded() {
    let (journal, _guard) = telemetry::capture();
    let (tokidoki, script) = start(vec![Reply::Hang]);
    let sent = tokidoki.send(me("Think hard").to("t1")).await.unwrap();
    let started = tokio::time::Instant::now();
    while script.seen.lock().unwrap().is_empty() {
        assert!(started.elapsed() < T);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(tokidoki.abort("t1").unwrap());
    assert_eq!(
        settled(&tokidoki, &sent.submission).await.status,
        Status::Unanswered
    );

    tokidoki
        .feedback(Feedback {
            from: Speaker::owner(),
            thread: "t1".into(),
            turn: None,
            text: "it never answered".into(),
        })
        .unwrap();
    let recorded = journal
        .entries(&telemetry::Filter {
            message: Some("feedback".into()),
            ..telemetry::Filter::default()
        })
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].thread.as_deref(), Some("t1"));
    assert_eq!(recorded[0].fields["turn"], 1);
    assert_eq!(recorded[0].severity, telemetry::Severity::Warn);
    // Kept with the thread too, whatever the journal records.
    let kept: Vec<Feedback> = tokidoki
        .inner
        .agent
        .engine()
        .doc(&feedback_key("t1"))
        .unwrap()
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].turn, Some(1));
}

/// A reply shows as it is written, then arrives whole: the pieces come first, add up to the
/// message, and `history` holds only the message.
#[tokio::test]
async fn a_reply_is_shown_as_it_is_written() {
    let (tokidoki, _) = start(vec![Reply::Streamed("One two three.")]);
    let mut events = tokidoki.events();
    let sent = tokidoki.send(me("Count.")).await.unwrap();
    let seen = until(&mut events, answered).await;

    let reply = seen
        .iter()
        .position(|event| {
            matches!(event, Event::Checkpoint { change: Change::Message { role, .. }, .. }
                if *role == interface::Role::Assistant)
        })
        .unwrap();
    let written: Vec<(usize, &str)> = seen
        .iter()
        .enumerate()
        .filter_map(|(at, event)| match event {
            Event::Delta {
                thread,
                delta: interface::Delta::Text { text },
            } if *thread == sent.thread => Some((at, text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(written.len(), 3);
    assert!(written.iter().all(|(at, _)| *at < reply));
    let text: String = written.iter().map(|(_, text)| *text).collect();
    assert_eq!(text, "One two three.");
    let history = tokidoki.history(&sent.thread, 0).unwrap();
    assert!(!history.iter().any(|e| matches!(e, Event::Delta { .. })));
}

/// The user sees what is remembered, adds to it, corrects it and removes it; each change is
/// an event, and a correction keeps the history behind one memory.
#[tokio::test]
async fn memory_is_read_and_changed_by_its_owner() {
    use interface::MemoryChange;
    let (tokidoki, _) = start(vec![]);
    let owner = Speaker::owner();
    let mut events = tokidoki.events();
    async fn change(events: &mut Events) -> MemoryChange {
        match until(events, |e| matches!(e, Event::Memory { .. }))
            .await
            .pop()
        {
            Some(Event::Memory { change }) => change,
            _ => unreachable!(),
        }
    }

    let fact = tokidoki.remember(&owner, "  Vegetarian  ").unwrap();
    assert_eq!((fact.text.as_str(), fact.version), ("Vegetarian", 1));
    let learned = MemoryChange::Learned {
        memory: fact.clone(),
    };
    assert_eq!(change(&mut events).await, learned);

    let revised = tokidoki.revise(&owner, fact.id, "Vegan").unwrap();
    assert_eq!(revised.version, 2);
    let corrected = MemoryChange::Revised {
        memory: revised.clone(),
        replaces: fact.id,
        was: "Vegetarian".into(),
    };
    assert_eq!(change(&mut events).await, corrected);
    assert_eq!(tokidoki.memories(&owner).unwrap(), vec![revised.clone()]);

    // The version that was replaced can no longer be edited or forgotten.
    let stale = tokidoki.revise(&owner, fact.id, "Pescatarian").unwrap_err();
    assert!(stale.downcast_ref::<Rejected>().is_some());
    assert!(tokidoki.forget(&owner, fact.id).is_err());
    assert!(tokidoki.remember(&owner, "  ").is_err());
    assert!(tokidoki.memories(&Speaker::new("stranger")).is_err());

    tokidoki.forget(&owner, revised.id).unwrap();
    assert!(tokidoki.memories(&owner).unwrap().is_empty());
    let forgotten = MemoryChange::Forgotten {
        ids: vec![fact.id, revised.id],
    };
    assert_eq!(change(&mut events).await, forgotten);
}

/// A folder to work in, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tokidoki-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Two spaces in one process are two tokidokis: each has its own files, its own memory and
/// its own journal, reopening one finds what it had, and deleting one leaves the other.
#[tokio::test]
async fn spaces_are_separate_tokidokis() {
    let root = Scratch::new("spaces");
    let spaces = Spaces::new(&root.0);
    let config = |data: &std::path::Path| Config::new(data, Vendor::Anthropic, "no-key".into());
    let owner = Speaker::owner();

    let dev = spaces.open("dev", config, Tools::new()).await.unwrap();
    let work = spaces.open("work-2", config, Tools::new()).await.unwrap();
    dev.remember(&owner, "Uses Rust").unwrap();
    work.remember(&owner, "Standup is at nine").unwrap();
    let texts = |tokidoki: &Tokidoki| -> Vec<String> {
        let memories = tokidoki.memories(&owner).unwrap();
        memories.into_iter().map(|m| m.text).collect()
    };
    assert_eq!(texts(&dev), ["Uses Rust"]);
    assert_eq!(texts(&work), ["Standup is at nine"]);

    // Each recorded what it did in its own journal, and nothing of the other's.
    let journal = |name: &str| -> String {
        let path = telemetry::Journal::beside(root.0.join(name).join(telemetry::STATE_FILE));
        let journal = telemetry::Journal::open(path).unwrap();
        let entries = journal.entries(&telemetry::Filter::default()).unwrap();
        let lines = entries
            .iter()
            .map(|e| format!("{} {}", e.message, e.fields["id"]));
        lines.collect::<Vec<_>>().join("; ")
    };
    spaces.close("dev").await.unwrap();
    work.close().await.unwrap();
    assert!(journal("dev").contains("memory added by the user"));
    assert_eq!(journal("dev").matches("memory added").count(), 1);
    assert_eq!(journal("work-2").matches("memory added").count(), 1);

    // Closed, however it was closed, a space is no longer open; opened again, it is a new
    // one with what it had.
    assert!(spaces.get("dev").is_none() && spaces.get("work-2").is_none());
    let reopened = spaces.open("dev", config, Tools::new()).await.unwrap();
    assert_eq!(texts(&reopened), ["Uses Rust"]);

    spaces.delete("dev").await.unwrap();
    assert!(!root.0.join("dev").exists() && root.0.join("work-2").exists());
    // A name is a folder: none that leaves the root, or that could be another name's folder
    // on a file system that ignores case.
    assert!(
        spaces
            .open("../elsewhere", config, Tools::new())
            .await
            .is_err()
    );
    assert!(spaces.open("Dev", config, Tools::new()).await.is_err());
    spaces.close_all().await.unwrap();
}

/// Opened asleep (no key to run with), a tokidoki shows what it has and takes no input.
#[tokio::test]
async fn asleep_it_can_be_read_and_takes_no_input() {
    let root = Scratch::new("asleep");
    std::fs::create_dir_all(&root.0).unwrap();
    let store = || Sqlite::open(root.0.join("tokidoki.db")).unwrap();
    let owner = Speaker::owner();

    let awake = Tokidoki::build(parts(vec![Reply::Streamed("Hello there.")], store()).0).unwrap();
    let sent = awake.send(Input::text(owner.clone(), "hi")).await.unwrap();
    awake.wait(&sent.submission).await.unwrap();
    let history = awake.history(&sent.thread, 0).unwrap();
    awake.close().await.unwrap();

    let why = Some("no key".to_string());
    let asleep = Tokidoki::assemble(parts(Vec::new(), store()).0, why).unwrap();
    let threads = asleep.threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, sent.thread);
    assert_eq!(asleep.history(&sent.thread, 0).unwrap(), history);
    asleep.remember(&owner, "Uses Rust").unwrap();
    let refused = asleep.send(Input::text(owner, "again")).await.unwrap_err();
    assert_eq!(refused.downcast::<Rejected>().unwrap().0, "no key");
    assert_eq!(asleep.threads().unwrap()[0].turns, threads[0].turns);
    asleep.close().await.unwrap();
}

/// The sidecar, driven the way its host drives it: lines in, lines out.
struct Sidecar {
    to: tokio::io::DuplexStream,
    from: tokio::io::Lines<tokio::io::BufReader<tokio::io::DuplexStream>>,
    served: tokio::task::JoinHandle<Result<()>>,
    _root: Scratch,
}

impl Sidecar {
    /// One whose agents answer from `replies` (each space's agent gets its own copy).
    fn start(name: &str, replies: fn() -> Vec<Reply>) -> Self {
        use tokio::io::AsyncBufReadExt;
        let root = Scratch::new(name);
        let (to, input) = tokio::io::duplex(1 << 16);
        let (output, from) = tokio::io::duplex(1 << 16);
        let open: serve::Open = Arc::new(move |_| Ok(start(replies()).0));
        let served = tokio::spawn(serve::run(
            Arc::new(Spaces::new(&root.0)),
            open,
            json!({ "provider": "anthropic" }),
            tokio::io::BufReader::new(input),
            output,
        ));
        Self {
            to,
            from: tokio::io::BufReader::new(from).lines(),
            served,
            _root: root,
        }
    }

    async fn write(&mut self, line: Value) {
        use tokio::io::AsyncWriteExt;
        self.to
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(T, self.from.next_line())
            .await
            .unwrap();
        serde_json::from_str(&line.unwrap().expect("the sidecar stopped writing")).unwrap()
    }

    /// Lines until (and with) the first that `last` matches.
    async fn read_until(&mut self, last: impl Fn(&Value) -> bool) -> Vec<Value> {
        let mut lines = Vec::new();
        loop {
            lines.push(self.read().await);
            if last(lines.last().unwrap()) {
                return lines;
            }
        }
    }
}

/// The sidecar says it is ready, opens a space's agent on the first request naming it,
/// answers each request by its ID, tags every event with its space, and stops when told.
#[tokio::test]
async fn the_sidecar_speaks_the_line_protocol() {
    let mut sidecar = Sidecar::start("sidecar", || vec![Reply::Streamed("Hello there.")]);
    let ready = sidecar.read().await;
    assert_eq!(ready["ready"]["backend"], "tokidoki");
    assert_eq!(ready["ready"]["provider"], "anthropic");

    sidecar
        .write(json!({ "id": 1, "method": "threads", "space": "space-7" }))
        .await;
    assert_eq!(sidecar.read().await, json!({ "id": 1, "result": [] }));

    let input = json!({ "from": { "id": "owner" }, "thread": "t1", "when_busy": "steer",
        "parts": [{ "type": "text", "text": "Hi" }] });
    let send = json!({ "id": 2, "method": "send", "space": "space-7",
        "params": { "input": input, "label": "dev" } });
    sidecar.write(send).await;
    let done = |line: &Value| line["event"]["submission"]["status"] == "done";
    let lines = sidecar.read_until(done).await;
    let sent = lines.iter().find(|line| line["id"] == 2).unwrap();
    assert_eq!(sent["result"]["thread"], "t1");
    let events: Vec<&Value> = lines.iter().filter(|line| line["id"].is_null()).collect();
    assert!(events.iter().all(|line| line["space"] == "space-7"));
    let written: String = events
        .iter()
        .filter(|line| line["event"]["type"] == "delta")
        .map(|line| line["event"]["delta"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(written, "Hello there.");

    // History reads back what the events said; another space has none of it.
    let history = json!({ "id": 3, "method": "history", "space": "space-7",
        "params": { "thread": "t1", "since": 0 } });
    sidecar.write(history).await;
    let history = sidecar
        .read_until(|line| line["id"] == 3)
        .await
        .pop()
        .unwrap();
    assert_eq!(history["result"][0]["type"], "checkpoint");
    sidecar
        .write(json!({ "id": 4, "method": "threads", "space": "space-8" }))
        .await;
    assert_eq!(sidecar.read().await, json!({ "id": 4, "result": [] }));

    // What it can't do is an error with the request's ID, never silence.
    for (id, request) in [
        (5, json!({ "id": 5, "method": "dance", "space": "space-7" })),
        (6, json!({ "id": 6, "method": "threads" })),
        (7, json!({ "id": 7, "method": "threads", "space": "../up" })),
        (8, json!({ "id": 8, "method": "abort", "space": "space-7" })),
    ] {
        sidecar.write(request).await;
        let answer = sidecar.read().await;
        assert_eq!(answer["id"], id);
        assert!(answer["error"].is_string(), "{answer}");
    }

    sidecar
        .write(json!({ "id": 9, "method": "close", "space": "space-7" }))
        .await;
    assert_eq!(sidecar.read().await, json!({ "id": 9, "result": null }));
    sidecar
        .write(json!({ "id": 10, "method": "shutdown" }))
        .await;
    assert_eq!(sidecar.read().await, json!({ "id": 10, "result": null }));
    tokio::time::timeout(T, sidecar.served)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

/// When its host goes away (stdin ends), the sidecar stops its agents and ends.
#[tokio::test]
async fn the_sidecar_ends_when_its_input_does() {
    let Sidecar { to, served, .. } = Sidecar::start("sidecar-eof", Vec::new);
    drop(to);
    tokio::time::timeout(T, served)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
