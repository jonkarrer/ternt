use super::*;
use crate::capture;
use crate::time::duration;
use context::Role;

/// A thread with two turns, and traced events inside the second.
fn setup() -> (Inspector, Journal, crate::Capture) {
    let (journal, guard) = capture();
    let store = Sqlite::memory().unwrap();
    let tick = || std::thread::sleep(std::time::Duration::from_millis(3));
    let mut thread = Thread::new("thread-1");
    thread
        .start_turn(Message::user("What's for dinner?"))
        .unwrap();
    thread.push(Message::assistant("Chicken curry.")).unwrap();
    thread.end_turn().unwrap();
    tick();
    thread
        .routed("rule", "the conversation is still going")
        .unwrap();
    thread
        .start_turn(Message::user("I told you I'm vegetarian!"))
        .unwrap();
    tick();
    tracing::info_span!("phase", thread = "thread-1", task = 4u64).in_scope(|| {
        tracing::info!(recalled = 0, "considered");
        tracing::warn!(attempt = 1, error = "overloaded", "model call failed");
    });
    tick();
    thread
        .push(Message {
            role: Role::Assistant,
            content: vec![Block::Text {
                text: "Sorry, lentil curry then.".into(),
            }],
        })
        .unwrap();
    thread.end_turn().unwrap();
    store.save_thread(&mut thread).unwrap();
    let inspector = Inspector::new(Arc::new(store), journal.clone(), None);
    (inspector, journal, guard)
}

#[test]
fn recent_lists_turns_newest_first_with_their_problems() {
    let (inspector, ..) = setup();
    let out = inspector.recent(5).unwrap();
    let turns = out.json.as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["turn"], 2);
    assert_eq!(turns[0]["input"], "I told you I'm vegetarian!");
    assert_eq!(turns[0]["answer"], "Sorry, lentil curry then.");
    assert_eq!(turns[0]["problems"].as_array().unwrap().len(), 1);
    assert_eq!(turns[1]["problems"].as_array().unwrap().len(), 0);
    assert!(out.text.contains("! "), "{}", out.text);
}

#[test]
fn a_turns_timeline_merges_checkpoints_and_entries_in_order() {
    let (inspector, ..) = setup();
    let out = inspector.thread("thread-1", Some(2), false).unwrap();
    let lines: Vec<&str> = out.text.lines().collect();
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from\n{}", out.text))
    };
    // The routing leads into the turn; the first turn is left out.
    assert!(!out.text.contains("Chicken"));
    assert!(position("routed here") < position("vegetarian"));
    assert!(position("vegetarian") < position("model call failed"));
    assert!(position("model call failed") < position("lentil"));
}

#[test]
fn search_finds_what_was_said_and_what_was_traced() {
    let (inspector, ..) = setup();
    let out = inspector
        .search("why did it say chicken when I'm vegetarian", 5)
        .unwrap();
    let said = out.json["conversations"].as_array().unwrap();
    assert!(said.iter().any(|s| s["text"] == "Chicken curry."));
    assert!(
        said.iter()
            .any(|s| s["text"] == "I told you I'm vegetarian!")
    );
    let out = inspector.search("overloaded", 5).unwrap();
    assert_eq!(out.json["journal"].as_array().unwrap().len(), 1);
}

#[test]
fn problems_and_entries() {
    let (inspector, journal, _guard) = setup();
    let out = inspector.problems(None).unwrap();
    let problems = out.json.as_array().unwrap();
    assert_eq!(problems.len(), 1);
    let id = problems[0]["id"].as_i64().unwrap();
    assert_eq!(
        inspector.entry(id).unwrap().json["fields"]["error"],
        "overloaded"
    );
    assert!(journal.entry(id + 100).unwrap().is_none());
}

#[test]
fn context_at_a_checkpoint() {
    let (inspector, ..) = setup();
    // The first turn stops where the second begins.
    let first = inspector.thread("thread-1", Some(1), false).unwrap().text;
    assert!(
        first.contains("Chicken") && !first.contains("vegetarian"),
        "{first}"
    );
    let out = inspector.context("thread-1", 2).unwrap();
    assert_eq!(out.json["messages"].as_array().unwrap().len(), 2);
    assert!(inspector.context("thread-1", 99).is_err());
}

#[test]
fn formats_times_and_reads_durations() {
    assert_eq!(clock(0), "1970-01-01 00:00:00.000Z");
    assert_eq!(clock(1_795_197_792_120_456), "2026-11-20 18:03:12.120Z");
    assert_eq!(duration("90s").unwrap(), 90_000_000);
    assert_eq!(duration("2h").unwrap(), 7_200_000_000);
    assert!(duration("2w").is_err());
}
