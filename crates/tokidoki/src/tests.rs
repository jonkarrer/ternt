use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use agent::{ToolFuture, Tools};
use context::{Block, Context, Message, Role as CtxRole};
use inference::Response;
use interface::{Change, Content, Event, Part, Speaker, Status};
use serde_json::{Value, json};

const T: Duration = Duration::from_secs(5);

enum Reply {
    Text(&'static str),
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
            Some(Reply::Text(text)) => vec![Block::Text { text: text.into() }],
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
    let script = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        ..Script::default()
    });
    let memory = Arc::new(Memory::in_memory().unwrap());
    // Nobody answers there: routing must decide in code.
    let nowhere = Provider::new(OpenAi, "Nowhere", "http://127.0.0.1:9/", vec![]);
    let router = Router::new(nowhere, Request::new("none"), memory.clone(), vec![]);
    let mut tools = Tools::new();
    tools.add(Weather);
    let tokidoki = Tokidoki::build(Parts {
        store: Arc::new(Sqlite::memory().unwrap()),
        model: script.clone(),
        mind: None,
        router,
        tools,
        system: Some("Be brief.".into()),
        settings: agent::Settings {
            backoff_ms: 10,
            ..agent::Settings::default()
        },
        speakers: vec!["owner".into()],
        journal: None,
    })
    .unwrap();
    (tokidoki, script)
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
