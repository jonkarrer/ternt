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

pub use journal::{Entry, Filter, Journal, Severity};
pub use layer::JournalLayer;

use std::path::Path;

use anyhow::Result;
use tracing_subscriber::filter::{FilterExt, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// What the journal records unless `TOKIDOKI_JOURNAL` says otherwise: everything from our
/// crates at debug and above, only warnings from dependencies.
pub const JOURNAL_DEFAULT: &str = "warn,agent=debug,attention=debug,context=debug,engine=debug,\
experience=debug,inference=debug,interface=debug,memory=debug,router=debug,state=debug,\
telemetry=debug,tokidoki=debug";

/// Installs the global subscriber: the journal at `journal` (if any), and the console on
/// stderr, filtered by `TOKIDOKI_LOG` (default `warn`; e.g. `TOKIDOKI_LOG=router=debug`).
pub fn init(journal: Option<&Path>) -> Result<Option<Journal>> {
    let journal = journal.map(Journal::open).transpose()?;
    let console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(env_filter("TOKIDOKI_LOG", "warn"));
    let recorded = journal.clone().map(|j| {
        JournalLayer::new(j).with_filter(events(env_filter("TOKIDOKI_JOURNAL", JOURNAL_DEFAULT)))
    });
    tracing_subscriber::registry()
        .with(console)
        .with(recorded)
        .try_init()?;
    Ok(journal)
}

fn env_filter(var: &str, default: &str) -> EnvFilter {
    EnvFilter::try_from_env(var).unwrap_or_else(|_| EnvFilter::new(default))
}

/// `filter` for events; every span passes, so an event keeps the context it happened in
/// (its thread and task) even when the span itself is below the filter's level.
fn events<S>(filter: EnvFilter) -> impl tracing_subscriber::layer::Filter<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    filter.or(filter_fn(|meta| meta.is_span()))
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
        let layer = JournalLayer::captured().with_filter(events(EnvFilter::new(JOURNAL_DEFAULT)));
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
    fn an_event_keeps_its_context_even_from_a_span_below_the_level() {
        let (journal, _guard) = capture();
        tracing::trace_span!(target: "elsewhere", "quiet", thread = "t9").in_scope(|| {
            tracing::warn!(target: "elsewhere", "loud");
            tracing::info!(target: "elsewhere", "filtered out");
        });
        let entries = journal.entries(&Filter::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].thread.as_deref(), Some("t9"));
    }
}
