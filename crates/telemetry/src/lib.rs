//! Finding out why the agent misbehaved.
//!
//! The crates emit structured `tracing` events: every model call (the exact body sent and the
//! raw reply), routing decisions, what Experience recalled and wrote, tool calls, task state
//! changes, faults and retries. Each event carries the `thread` and `task` it happened in.
//!
//! `init` sends them to a journal: an append-only SQLite file beside the state database,
//! searchable by words. The `inspect` binary reads the journal together with the state
//! database (threads, tasks), so a moment described in words ("it forgot I'm vegetarian")
//! can be found, laid out step by step, and its model calls re-sent (`inspect replay`).
//! `Journal` is also an `inference::Tape`: a provider given one answers recorded requests
//! from it, so a run can be repeated without calling the model.
//!
//! Warnings and errors are the misbehavior the code can see on its own: failed or slow model
//! calls, retries, faults, crashes cut through, discarded Experience work, unanswered inputs.

pub mod inspect;
mod journal;
mod layer;
pub mod replay;
pub mod time;

pub use journal::{Entry, Filter, Journal, Severity};
use layer::JournalLayer;

use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing_subscriber::filter::{FilterExt, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Our crates: the journal records them at debug and above unless `TOKIDOKI_JOURNAL` says
/// otherwise, and keeps their spans (an event's context); dependencies only warn.
const CRATES: &[&str] = &[
    "agent",
    "attention",
    "context",
    "engine",
    "experience",
    "inference",
    "interface",
    "memory",
    "router",
    "state",
    "telemetry",
    "tokidoki",
];

fn journal_default() -> String {
    let ours: Vec<String> = CRATES.iter().map(|c| format!("{c}=debug")).collect();
    format!("warn,{}", ours.join(","))
}

fn ours(target: &str) -> bool {
    CRATES.contains(&target.split("::").next().unwrap_or(target))
}

/// The files in a data directory: the state database (the journal sits beside it,
/// `Journal::beside`) and the memory database.
pub const STATE_FILE: &str = "tokidoki.db";
pub const MEMORY_FILE: &str = "memory.db";

/// The data directory: `TOKIDOKI_DATA`, or `data`.
pub fn data_dir() -> PathBuf {
    std::env::var("TOKIDOKI_DATA")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "data".into())
        .into()
}

/// How long the journal keeps entries unless `TOKIDOKI_JOURNAL_DAYS` says otherwise: every
/// model call holds its whole conversation, so it grows fast.
pub const RETENTION_DAYS: u64 = 14;

/// Installs the global subscriber: the journal at `path`, first pruned (in the background) of
/// entries older than its retention, and the console on stderr, filtered by `TOKIDOKI_LOG` (default
/// `warn`; e.g. `TOKIDOKI_LOG=router=debug`). Flush the returned journal before exiting: it
/// writes on its own thread.
pub fn init(path: &Path) -> Result<Journal> {
    let journal = Journal::open(path)?;
    let days = std::env::var("TOKIDOKI_JOURNAL_DAYS")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(RETENTION_DAYS);
    let before = time::now().saturating_sub(days * time::DAY);
    // Off the startup path: nothing needs the old entries gone before the first input.
    let pruning = journal.clone();
    std::thread::spawn(move || {
        if let Err(error) = pruning.prune(before) {
            eprintln!("telemetry: pruning the journal failed: {error:#}");
        }
    });
    let console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(env_filter("TOKIDOKI_LOG", "warn"));
    let recorded = JournalLayer::new(journal.clone())
        .with_filter(events(env_filter("TOKIDOKI_JOURNAL", &journal_default())));
    tracing_subscriber::registry()
        .with(console)
        .with(recorded)
        .try_init()?;
    Ok(journal)
}

fn env_filter(var: &str, default: &str) -> EnvFilter {
    EnvFilter::try_from_env(var).unwrap_or_else(|_| EnvFilter::new(default))
}

/// `filter` for events; our crates' spans all pass, so an event keeps the context it happened
/// in (its thread and task) even when the span itself is below the filter's level.
fn events<S>(filter: EnvFilter) -> impl tracing_subscriber::layer::Filter<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    filter.or(filter_fn(|meta| meta.is_span() && ours(meta.target())))
}

/// Records what is traced on this thread, until the guard drops, into a fresh in-memory
/// journal, filtered as `init` does by default: for tests that check what was traced.
///
/// One global subscriber serves every capture, sending each event to the journal of the
/// thread it happened on (a test's current-thread runtime runs its tasks there). A
/// subscriber per thread would race: `tracing` caches per call site whether anyone listens,
/// across threads, so a test running beside others could miss its own events.
pub fn capture() -> (Journal, Capture) {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let layer = JournalLayer::captured().with_filter(events(EnvFilter::new(journal_default())));
        // Fails only if the program installed its own subscriber; then nothing is captured.
        let _ = tracing_subscriber::registry().with(layer).try_init();
    });
    let journal = Journal::in_memory().expect("in-memory journal");
    layer::CAPTURE.with(|slot| *slot.borrow_mut() = Some(journal.clone()));
    (journal, Capture(()))
}

/// Stops `capture` on drop.
pub struct Capture(());

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = layer::CAPTURE.try_with(|slot| slot.borrow_mut().take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_carry_their_spans_fields() {
        let (journal, _guard) = capture();
        let span = tracing::info_span!("phase", thread = "t1", task = 7u64, phase = "request");
        span.in_scope(|| {
            let body = serde_json::json!({ "model": "m" });
            tracing::warn!(attempt = 2, request = %body, "model call failed");
        });
        tracing::info!("outside");

        let entries = journal.entries(&Filter::default()).unwrap();
        assert_eq!(entries.len(), 2);
        let failed = &entries[0];
        assert_eq!(failed.message, "model call failed");
        assert_eq!(failed.severity, Severity::Warn);
        assert_eq!(failed.thread.as_deref(), Some("t1"));
        assert_eq!(failed.task, Some(7));
        assert_eq!(failed.spans, "phase");
        assert_eq!(failed.fields["phase"], "request");
        assert_eq!(failed.fields["attempt"], 2);
        // A JSON body stays JSON, not a string of it.
        assert_eq!(failed.fields["request"]["model"], "m");
        assert_eq!(entries[1].thread, None);
    }

    #[test]
    fn only_json_fields_are_kept_as_json() {
        let (journal, _guard) = capture();
        let body = serde_json::json!({ "model": "m" });
        tracing::info!(request = %body, text = "[1, 2]", said = "{\"a\": 1}", "model call");
        let entry = &journal.entries(&Filter::default()).unwrap()[0];
        assert_eq!(entry.fields["request"]["model"], "m");
        // What was said stays as it was said, even when it looks like JSON.
        assert_eq!(entry.fields["text"], "[1, 2]");
        assert_eq!(entry.fields["said"], "{\"a\": 1}");
    }

    #[test]
    fn recording_writes_on_the_journals_own_thread() {
        let journal = Journal::in_memory().unwrap();
        for i in 0..100 {
            journal.record(Entry {
                id: 0,
                at: i,
                severity: Severity::Info,
                target: "test".into(),
                spans: String::new(),
                thread: None,
                task: None,
                message: format!("e{i}"),
                fields: Default::default(),
            });
        }
        // A read waits for what was recorded before it.
        let entries = journal.entries(&Filter::default()).unwrap();
        assert_eq!(entries.len(), 100);
        assert_eq!(entries[99].message, "e99");
    }

    #[test]
    fn an_event_keeps_its_context_even_from_a_span_below_the_level() {
        let (journal, _guard) = capture();
        tracing::trace_span!(target: "engine", "quiet", thread = "t9").in_scope(|| {
            tracing::warn!(target: "elsewhere", "loud");
            tracing::info!(target: "elsewhere", "filtered out");
        });
        let entries = journal.entries(&Filter::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].thread.as_deref(), Some("t9"));
    }
}
