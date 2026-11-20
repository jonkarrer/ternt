//! The journal: every traced event, durably, in its own SQLite file beside the state database
//! (its own file, so writing it never waits on, or holds up, the engine's commits).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rusqlite::{Connection, Row, params, params_from_iter};
use serde_json::{Map, Value};

/// How serious an entry is, as stored: higher is worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl Severity {
    pub fn of(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    fn from_i64(n: i64) -> Self {
        match n {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Warn,
            _ => Self::Error,
        }
    }
}

/// One traced event, with the fields of the spans it happened in (inner spans win).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    /// µs since the epoch.
    pub at: u64,
    pub severity: Severity,
    /// The module that emitted it, e.g. `agent::generation`.
    pub target: String,
    /// The spans it happened in, outermost first, e.g. `phase > consider`.
    pub spans: String,
    pub thread: Option<String>,
    pub task: Option<i64>,
    pub message: String,
    pub fields: Map<String, Value>,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "at": self.at,
            "level": self.severity.name(),
            "target": self.target,
            "spans": self.spans,
            "thread": self.thread,
            "task": self.task,
            "message": self.message,
            "fields": self.fields,
        })
    }
}

/// Fields too large and repetitive to search: every request repeats the whole conversation.
const UNSEARCHED: &[&str] = &["request", "response"];

/// Which entries `Journal::entries` returns, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub thread: Option<String>,
    pub task: Option<i64>,
    pub message: Option<String>,
    pub min: Option<Severity>,
    /// µs since the epoch, inclusive.
    pub since: Option<u64>,
    /// µs since the epoch, exclusive.
    pub until: Option<u64>,
    /// The newest `limit` matches.
    pub limit: Option<usize>,
}

#[derive(Clone)]
pub struct Journal {
    conn: Arc<Mutex<Connection>>,
}

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Where the journal for the state database at `db` goes: `data/tokidoki.db` →
    /// `data/tokidoki.journal.db`.
    pub fn beside(db: impl AsRef<Path>) -> PathBuf {
        let db = db.as_ref();
        let stem = db.file_stem().unwrap_or_default().to_string_lossy();
        db.with_file_name(format!("{stem}.journal.db"))
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                 id INTEGER PRIMARY KEY,
                 at_us INTEGER NOT NULL,
                 severity INTEGER NOT NULL,
                 target TEXT NOT NULL,
                 spans TEXT NOT NULL,
                 thread TEXT,
                 task INTEGER,
                 message TEXT NOT NULL,
                 fields TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS entries_at ON entries (at_us);
             CREATE INDEX IF NOT EXISTS entries_thread ON entries (thread, at_us);
             CREATE INDEX IF NOT EXISTS entries_task ON entries (task);
             CREATE INDEX IF NOT EXISTS entries_severity ON entries (severity, at_us);
             CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts
                 USING fts5(text, content = '', contentless_delete = 1);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds an entry; its `id` is ignored. Returns the new ID.
    pub fn append(&self, entry: &Entry) -> Result<i64> {
        let conn = self.lock();
        conn.prepare_cached(
            "INSERT INTO entries (at_us, severity, target, spans, thread, task, message, fields)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?
        .execute(params![
            entry.at as i64,
            entry.severity as i64,
            entry.target,
            entry.spans,
            entry.thread,
            entry.task,
            entry.message,
            Value::Object(entry.fields.clone()).to_string(),
        ])?;
        let id = conn.last_insert_rowid();
        conn.prepare_cached("INSERT INTO entries_fts (rowid, text) VALUES (?1, ?2)")?
            .execute(params![id, searchable(entry)])?;
        Ok(id)
    }

    pub fn entry(&self, id: i64) -> Result<Option<Entry>> {
        let conn = self.lock();
        let mut select =
            conn.prepare_cached(&format!("SELECT {COLUMNS} FROM entries WHERE id = ?1"))?;
        let mut rows = select.query([id])?;
        rows.next()?.map(parse).transpose()
    }

    pub fn entries(&self, filter: &Filter) -> Result<Vec<Entry>> {
        let mut clauses = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        let mut add = |clause: &str, value: rusqlite::types::Value| {
            values.push(value);
            clauses.push(clause.replace('?', &format!("?{}", values.len())));
        };
        if let Some(thread) = &filter.thread {
            add("thread = ?", thread.clone().into());
        }
        if let Some(task) = filter.task {
            add("task = ?", task.into());
        }
        if let Some(message) = &filter.message {
            add("message = ?", message.clone().into());
        }
        if let Some(min) = filter.min {
            add("severity >= ?", (min as i64).into());
        }
        if let Some(since) = filter.since {
            add("at_us >= ?", (since as i64).into());
        }
        if let Some(until) = filter.until {
            add("at_us < ?", (until as i64).into());
        }
        let filter_sql = match clauses.is_empty() {
            true => String::new(),
            false => format!("WHERE {}", clauses.join(" AND ")),
        };
        let limit = filter.limit.map_or(-1, |n| n as i64);
        let sql = format!(
            "SELECT * FROM (SELECT {COLUMNS} FROM entries {filter_sql} ORDER BY id DESC LIMIT {limit})
             ORDER BY id"
        );
        let conn = self.lock();
        let mut select = conn.prepare(&sql)?;
        let mut rows = select.query(params_from_iter(values))?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(parse(row)?);
        }
        Ok(entries)
    }

    /// Entries matching any of the words in `text`, best first.
    pub fn search(&self, text: &str, limit: usize) -> Result<Vec<Entry>> {
        let Some(query) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let ids: Vec<i64> = {
            let conn = self.lock();
            let mut select = conn.prepare_cached(
                "SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            )?;
            select
                .query_map(params![query, limit as i64], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        ids.into_iter()
            .filter_map(|id| self.entry(id).transpose())
            .collect()
    }

    /// Deletes entries older than `before` (µs since the epoch). Returns how many.
    pub fn prune(&self, before: u64) -> Result<usize> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM entries_fts WHERE rowid IN (SELECT id FROM entries WHERE at_us < ?1)",
            [before as i64],
        )?;
        Ok(conn.execute("DELETE FROM entries WHERE at_us < ?1", [before as i64])?)
    }
}

/// Replays recorded model calls: the newest successful call with exactly this body.
impl inference::Tape for Journal {
    fn reply(&self, provider: &str, body: &Value) -> Option<Value> {
        let wanted = body.to_string();
        let conn = self.lock();
        let mut select = conn
            .prepare_cached(
                "SELECT fields FROM entries
                 WHERE message = 'model call' AND json_extract(fields, '$.provider') = ?1
                 ORDER BY id DESC",
            )
            .ok()?;
        let mut rows = select.query([provider]).ok()?;
        while let Ok(Some(row)) = rows.next() {
            let fields: String = row.get(0).ok()?;
            let Ok(Value::Object(mut fields)) = serde_json::from_str(&fields) else {
                continue;
            };
            if fields.get("request").map(Value::to_string).as_deref() == Some(&wanted) {
                return fields.remove("response");
            }
        }
        None
    }
}

const COLUMNS: &str = "id, at_us, severity, target, spans, thread, task, message, fields";

fn parse(row: &Row) -> Result<Entry> {
    let fields: String = row.get(8)?;
    let fields = match serde_json::from_str(&fields)? {
        Value::Object(fields) => fields,
        _ => Map::new(),
    };
    Ok(Entry {
        id: row.get(0)?,
        at: row.get::<_, i64>(1)? as u64,
        severity: Severity::from_i64(row.get(2)?),
        target: row.get(3)?,
        spans: row.get(4)?,
        thread: row.get(5)?,
        task: row.get(6)?,
        message: row.get(7)?,
        fields,
    })
}

/// The message and every searchable field value, as one text.
fn searchable(entry: &Entry) -> String {
    let mut text = entry.message.clone();
    for (key, value) in &entry.fields {
        if UNSEARCHED.contains(&key.as_str()) {
            continue;
        }
        text.push('\n');
        match value {
            Value::String(s) => text.push_str(s),
            other => text.push_str(&other.to_string()),
        }
    }
    text
}

/// Any of the words, as an FTS5 query: words are quoted, so punctuation can't break it.
pub(crate) fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = words(text).map(|w| format!("\"{w}\"")).collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// Lowercased runs of letters and digits.
pub(crate) fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inference::Tape;
    use serde_json::json;

    fn entry(message: &str, thread: &str, fields: Value) -> Entry {
        Entry {
            id: 0,
            at: 1_000,
            severity: Severity::Info,
            target: "test".into(),
            spans: String::new(),
            thread: Some(thread.into()),
            task: None,
            message: message.into(),
            fields: fields.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn finds_entries_by_their_words_but_not_by_request_bodies() {
        let journal = Journal::in_memory().unwrap();
        journal
            .append(&entry(
                "model call",
                "t1",
                json!({
                    "text": "You're vegetarian, so try the lentils.",
                    "request": "{\"messages\": \"pizza\"}",
                }),
            ))
            .unwrap();
        journal
            .append(&entry("tool call", "t2", json!({ "name": "weather" })))
            .unwrap();

        let hits = journal.search("isn't it vegetarian?", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread.as_deref(), Some("t1"));
        assert!(journal.search("pizza", 10).unwrap().is_empty());
    }

    #[test]
    fn filters_and_prunes() {
        let journal = Journal::in_memory().unwrap();
        for (i, thread) in ["a", "b", "a"].into_iter().enumerate() {
            let mut e = entry("x", thread, json!({}));
            e.at = i as u64 * 100;
            e.severity = if i == 2 {
                Severity::Warn
            } else {
                Severity::Info
            };
            journal.append(&e).unwrap();
        }
        let only = |filter: Filter| journal.entries(&filter).unwrap().len();
        assert_eq!(
            only(Filter {
                thread: Some("a".into()),
                ..Filter::default()
            }),
            2
        );
        assert_eq!(
            only(Filter {
                min: Some(Severity::Warn),
                ..Filter::default()
            }),
            1
        );
        assert_eq!(
            only(Filter {
                since: Some(100),
                ..Filter::default()
            }),
            2
        );
        assert_eq!(
            only(Filter {
                limit: Some(1),
                ..Filter::default()
            }),
            1
        );

        assert_eq!(journal.prune(150).unwrap(), 2);
        assert_eq!(only(Filter::default()), 1);
        assert!(journal.search("x", 10).unwrap().len() == 1);
    }

    #[test]
    fn replays_the_reply_to_the_same_body() {
        let journal = Journal::in_memory().unwrap();
        let body = json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] });
        journal
            .append(&entry(
                "model call",
                "t",
                json!({
                    "provider": "Anthropic",
                    "request": body,
                    "response": { "content": "hello" },
                }),
            ))
            .unwrap();
        assert_eq!(
            journal.reply("Anthropic", &body),
            Some(json!({ "content": "hello" }))
        );
        assert_eq!(journal.reply("Baseten", &body), None);
        assert_eq!(
            journal.reply("Anthropic", &json!({ "model": "other" })),
            None
        );
    }

    #[test]
    fn sits_beside_the_state_database() {
        assert_eq!(
            Journal::beside("data/tokidoki.db"),
            PathBuf::from("data/tokidoki.journal.db")
        );
    }
}

#[cfg(test)]
mod replay_tests {
    use std::sync::Arc;

    use super::*;
    use crate::capture;
    use inference::providers::{Anthropic, Api};
    use inference::{Context, Message, Provider, Request};
    use serde_json::json;

    /// A provider nobody answers at: only the tape can.
    fn nowhere(tape: Journal) -> Provider<Anthropic> {
        Provider::new(Anthropic, "Anthropic", "http://127.0.0.1:9/", vec![])
            .with_tape(Arc::new(tape))
    }

    #[tokio::test]
    async fn a_taped_provider_answers_recorded_requests_without_the_network() {
        let req = Request::new("claude-opus-5-5");
        let mut ctx = Context::new("Be brief.");
        ctx.push(Message::user("Hi"));
        let recorded = Journal::in_memory().unwrap();
        recorded
            .append(&Entry {
                id: 0,
                at: 1,
                severity: Severity::Info,
                target: "inference".into(),
                spans: String::new(),
                thread: None,
                task: None,
                message: "model call".into(),
                fields: json!({
                    "provider": "Anthropic",
                    "request": Anthropic.body(&req, &ctx),
                    "response": {
                        "id": "msg_1", "type": "message", "role": "assistant",
                        "model": "claude-opus-5-5",
                        "content": [{ "type": "text", "text": "Hello." }],
                        "stop_reason": "end_turn", "stop_sequence": null,
                        "usage": { "input_tokens": 9, "output_tokens": 2 },
                    },
                })
                .as_object()
                .cloned()
                .unwrap(),
            })
            .unwrap();
        let provider = nowhere(recorded);
        let (traced, _guard) = capture();

        let res = provider.chat(&req, &ctx).await.unwrap();
        assert_eq!(res.message.text(), "Hello.");
        // Anything else goes to the provider, here unreachable.
        ctx.push(Message::user("And?"));
        assert!(provider.chat(&req, &ctx).await.is_err());

        let entries = traced.entries(&Filter::default()).unwrap();
        let messages: Vec<&str> = entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["model call", "model call failed"]);
        assert_eq!(entries[0].fields["replayed"], true);
        assert_eq!(entries[0].fields["stop"], "end_turn");
        assert_eq!(entries[1].fields["replayed"], false);
    }
}
