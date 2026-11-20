//! A `tracing` layer that writes every event to the journal, with the fields of the spans it
//! happened in: an event inside a task's phase carries that phase's `thread` and `task`.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::journal::{Entry, Journal, Severity};

pub struct JournalLayer {
    /// `None`: the journal `capture` set for the current thread, if any.
    journal: Option<Journal>,
}

thread_local! {
    pub(crate) static CAPTURE: std::cell::RefCell<Option<Journal>> = const { std::cell::RefCell::new(None) };
}

impl JournalLayer {
    pub fn new(journal: Journal) -> Self {
        Self {
            journal: Some(journal),
        }
    }

    /// Writes to the journal `capture` set for the thread an event happens on.
    pub(crate) fn captured() -> Self {
        Self { journal: None }
    }

    fn append(&self, entry: &Entry) -> anyhow::Result<()> {
        match &self.journal {
            Some(journal) => journal.append(entry).map(drop),
            None => match CAPTURE
                .try_with(|slot| slot.borrow().clone())
                .ok()
                .flatten()
            {
                Some(journal) => journal.append(entry).map(drop),
                None => Ok(()),
            },
        }
    }
}

/// A span's name and fields, kept in its extensions.
struct SpanFields {
    name: &'static str,
    fields: Map<String, Value>,
}

impl<S> Layer<S> for JournalLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = Map::new();
        attrs.record(&mut Fields(&mut fields));
        span.extensions_mut().insert(SpanFields {
            name: attrs.metadata().name(),
            fields,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if let Some(data) = span.extensions_mut().get_mut::<SpanFields>() {
            values.record(&mut Fields(&mut data.fields));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Map::new();
        let mut spans = Vec::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(data) = span.extensions().get::<SpanFields>() {
                    spans.push(data.name);
                    fields.extend(data.fields.clone());
                }
            }
        }
        let mut own = Map::new();
        event.record(&mut Fields(&mut own));
        let message = match own.remove("message") {
            Some(Value::String(message)) => message,
            Some(other) => other.to_string(),
            None => String::new(),
        };
        fields.extend(own);
        let thread = fields
            .get("thread")
            .and_then(Value::as_str)
            .map(String::from);
        let task = fields.get("task").and_then(Value::as_i64);
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64);
        let entry = Entry {
            id: 0,
            at,
            severity: Severity::of(event.metadata().level()),
            target: event.metadata().target().to_string(),
            spans: spans.join(" > "),
            thread,
            task,
            message,
            fields,
        };
        // Tracing must never take the program down: a journal that can't write is reported
        // and skipped.
        if let Err(error) = self.append(&entry) {
            eprintln!("telemetry: journal write failed: {error:#}");
        }
    }
}

/// Records fields as JSON. Text that is itself JSON (a request body, say) is kept as JSON.
struct Fields<'a>(&'a mut Map<String, Value>);

impl Fields<'_> {
    fn text(&mut self, field: &Field, text: String) {
        let value = match text.starts_with(['{', '[']) {
            true => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            false => Value::String(text),
        };
        self.0.insert(field.name().to_string(), value);
    }
}

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.text(field, format!("{value:?}"));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_string(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_string(), value.into());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.text(field, value.to_string());
    }
}
