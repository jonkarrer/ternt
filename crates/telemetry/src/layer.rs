//! A `tracing` layer that writes every event to the journal, with the fields of the spans it
//! happened in: an event inside a task's phase carries that phase's `thread` and `task`.

use std::fmt;

use serde_json::value::RawValue;
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::journal::{BODIES, Entry, Journal, Severity, as_text};

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

    fn record(&self, entry: Entry, bodies: Vec<(String, String)>) {
        match &self.journal {
            Some(journal) => journal.record_with(entry, bodies),
            None => {
                if let Ok(Some(journal)) = CAPTURE.try_with(|slot| slot.borrow().clone()) {
                    journal.record_with(entry, bodies);
                }
            }
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
        attrs.record(&mut Fields::new(&mut fields));
        span.extensions_mut().insert(SpanFields {
            name: attrs.metadata().name(),
            fields,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if let Some(data) = span.extensions_mut().get_mut::<SpanFields>() {
            values.record(&mut Fields::new(&mut data.fields));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // The event's own fields, then each span's, innermost first: the nearest wins, and each
        // field is copied once.
        let mut fields = Map::new();
        let mut bodies = Vec::new();
        event.record(&mut Fields {
            map: &mut fields,
            bodies: Some(&mut bodies),
        });
        let message = fields
            .remove("message")
            .as_ref()
            .map(as_text)
            .unwrap_or_default();
        let mut spans = Vec::new();
        for span in ctx.event_scope(event).into_iter().flatten() {
            if let Some(data) = span.extensions().get::<SpanFields>() {
                spans.push(data.name);
                for (key, value) in &data.fields {
                    fields.entry(key.as_str()).or_insert_with(|| value.clone());
                }
            }
        }
        spans.reverse();
        let thread = fields
            .get("thread")
            .and_then(Value::as_str)
            .map(String::from);
        let task = fields.get("task").and_then(Value::as_i64);
        let entry = Entry {
            id: 0,
            at: crate::time::now(),
            severity: Severity::of(event.metadata().level()),
            target: event.metadata().target().to_string(),
            spans: spans.join(" > "),
            thread,
            task,
            message,
            fields,
        };
        // Written on the journal's own thread; one that can't write reports it there.
        self.record(entry, bodies);
    }
}

/// Fields that hold JSON (a model request and reply, tool arguments, a task's result): kept as
/// JSON. Anything else is kept as the text it is, even text that looks like JSON.
fn is_json(field: &str) -> bool {
    BODIES.contains(&field) || matches!(field, "arguments" | "result")
}

/// Records fields as JSON values; an event's bodies (`BODIES`), JSON text already, are set
/// aside as text instead, to be written as they are.
struct Fields<'a> {
    map: &'a mut Map<String, Value>,
    bodies: Option<&'a mut Vec<(String, String)>>,
}

impl<'a> Fields<'a> {
    fn new(map: &'a mut Map<String, Value>) -> Self {
        Self { map, bodies: None }
    }

    fn text(&mut self, field: &Field, text: String) {
        let name = field.name();
        // Checked to be JSON without building it.
        if let Some(bodies) = &mut self.bodies
            && BODIES.contains(&name)
            && serde_json::from_str::<&RawValue>(&text).is_ok()
        {
            bodies.push((name.to_string(), text));
            return;
        }
        let value = match is_json(name) {
            true => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            false => Value::String(text),
        };
        self.map.insert(name.to_string(), value);
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
        self.map.insert(field.name().to_string(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.map.insert(field.name().to_string(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.map.insert(field.name().to_string(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.map.insert(field.name().to_string(), value.into());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.text(field, value.to_string());
    }
}
