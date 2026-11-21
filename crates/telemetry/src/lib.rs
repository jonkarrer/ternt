//! Finding out why the agent misbehaved.
//!
//! The crates emit structured `tracing` events: every model call (the exact body sent and the
//! raw reply), routing decisions, what Experience recalled and wrote, tool calls, task state
//! changes, faults and retries. Each event carries the `thread` and `task` it happened in.
//!
//! `record` sends them to a journal: an append-only SQLite file beside the state database,
//! searchable by words. A process can record several at once, one per running system: each
//! gets a span, and what is traced inside it goes to its journal and no other. The `inspect` binary reads the journal together with the state
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

/// A journal being recorded to, and the span whose events it gets. Dropping it stops the
/// recording for spans opened afterwards; flush the journal before exiting (it writes on its
/// own thread).
pub struct Recording {
    pub journal: Journal,
    /// What is traced inside this span, in a span opened inside it, or in a task spawned
    /// `in_current_span` inside it, is recorded in `journal`. Enter it around everything the
    /// system does.
    pub span: tracing::Span,
    route: u64,
}

impl Drop for Recording {
    fn drop(&mut self) {
        layer::routes().remove(&self.route);
    }
}

/// Starts recording to the journal at `path`, first pruned (in the background) of entries
/// older than its retention. The first call installs the process's subscriber: journals, and
/// the console on stderr, filtered by `TOKIDOKI_LOG` (default `warn`; e.g.
/// `TOKIDOKI_LOG=router=debug`). Fails when the program has installed a subscriber of its
/// own, which then gets the events instead.
///
/// Any number of journals can be recorded at once: an event goes to the journal of the
/// `Recording::span` it happened in, and nowhere when it happened in none.
pub fn record(path: &Path) -> Result<Recording> {
    anyhow::ensure!(
        install(true),
        "another tracing subscriber is installed in this process"
    );
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
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let route = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    layer::routes().insert(route, journal.clone());
    // The layer reads the route off this span as it opens, and sends its events there.
    let span = tracing::info_span!(target: "telemetry", "tokidoki", route);
    Ok(Recording {
        journal,
        span,
        route,
    })
}

/// Installs the one subscriber this process records through, once; `console` (of the first
/// call) adds warnings on stderr. Whether it is the process's subscriber: not when the
/// program installed its own first.
fn install(console: bool) -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        // A program records what `TOKIDOKI_JOURNAL` says; a test, always the default.
        let filter = match console {
            true => env_filter("TOKIDOKI_JOURNAL", &journal_default()),
            false => EnvFilter::new(journal_default()),
        };
        let recorded = JournalLayer.with_filter(events(filter));
        let console = console.then(|| {
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(env_filter("TOKIDOKI_LOG", "warn"))
        });
        tracing_subscriber::registry()
            .with(console)
            .with(recorded)
            .try_init()
            .is_ok()
    })
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
/// journal, filtered as `record` does by default: for tests that check what was traced.
///
/// The process's one subscriber serves every capture, sending each event that happened in
/// no recording's span to the journal of the thread it happened on (a test's current-thread
/// runtime runs its tasks there). A subscriber per thread would race: `tracing` caches per
/// call site whether anyone listens, across threads, so a test running beside others could
/// miss its own events.
pub fn capture() -> (Journal, Capture) {
    // Fails only if the program installed its own subscriber; then nothing is captured.
    install(false);
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

    /// Two recordings in one process: each journal gets what happened in its own span, also
    /// from a span opened inside it and a task spawned there, and nothing of the other's.
    #[tokio::test]
    async fn each_recording_gets_only_its_own_events() {
        use tracing::Instrument;
        let dir = std::env::temp_dir().join(format!("tokidoki-routes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let one = record(&dir.join("one.journal.db")).unwrap();
        let two = record(&dir.join("two.journal.db")).unwrap();

        one.span.in_scope(|| tracing::info!("in one"));
        two.span.in_scope(|| {
            tracing::info_span!("phase", thread = "t2").in_scope(|| tracing::info!("in two"));
        });
        tokio::spawn(async { tracing::info!("spawned in one") }.instrument(one.span.clone()))
            .await
            .unwrap();
        tracing::info!("in neither");

        let messages = |journal: &Journal| -> Vec<String> {
            let entries = journal.entries(&Filter::default()).unwrap();
            entries.into_iter().map(|e| e.message).collect()
        };
        assert_eq!(messages(&one.journal), ["in one", "spawned in one"]);
        assert_eq!(messages(&two.journal), ["in two"]);
        let entry = &two.journal.entries(&Filter::default()).unwrap()[0];
        assert_eq!(entry.thread.as_deref(), Some("t2"));
        // How it was routed is not part of what happened.
        assert!(!entry.fields.contains_key("route"));
        assert_eq!(entry.spans, "tokidoki > phase");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
