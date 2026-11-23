//! Reading what happened: the state database (threads, tasks), the memory database, and the
//! journal, laid side by side. Every command renders plain text, or JSON with `json`.

use crate::time::{clock, us};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use context::{Block, Message};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use state::{Checkpoint, Checkpointer, Event, Sqlite, Thread};

use crate::journal::{BODIES, Entry, Filter, Journal, Severity, as_text, words};

/// What a command found: text for a person, JSON for a program.
pub struct Output {
    pub text: String,
    pub json: Value,
}

impl Output {
    pub fn render(&self, json: bool) -> String {
        match json {
            true => serde_json::to_string_pretty(&self.json).unwrap_or_default(),
            false => self.text.clone(),
        }
    }
}

pub struct Inspector {
    store: Arc<Sqlite>,
    journal: Journal,
    memory: Option<Connection>,
}

/// One turn of a thread, as `recent` lists it.
struct TurnSummary {
    thread: String,
    turn: u32,
    start: u64,
    end: Option<u64>,
    first: usize,
    input: String,
    answer: Option<String>,
}

impl Inspector {
    pub fn new(store: Arc<Sqlite>, journal: Journal, memory: Option<Connection>) -> Self {
        Self {
            store,
            journal,
            memory,
        }
    }

    /// Opens the state database at `db`, its journal beside it, and the memory database if
    /// there is one at `memory`. The memory database is opened read-only.
    pub fn open(db: &Path, memory: Option<&Path>) -> Result<Self> {
        let store = Sqlite::open(db).with_context(|| format!("state database {}", db.display()))?;
        let journal = Journal::open(Journal::beside(db))?;
        let memory = match memory.filter(|m| m.exists()) {
            Some(path) => Some(Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?),
            None => None,
        };
        Ok(Self::new(Arc::new(store), journal, memory))
    }

    /// Re-sends a recorded model call (`replay::resend`), as it was or with `body` instead.
    pub async fn replay(&self, id: i64, body: Option<Value>) -> Result<Output> {
        let entry = self
            .journal
            .entry(id)?
            .with_context(|| format!("no entry {id}"))?;
        crate::replay::resend(&entry, body).await
    }

    /// Deletes journal entries older than `before` (µs since the epoch). Returns how many.
    pub fn prune(&self, before: u64) -> Result<usize> {
        self.journal.prune(before)
    }

    /// The latest `n` turns across threads, newest first, with their warnings and errors.
    pub fn recent(&self, n: usize) -> Result<Output> {
        let mut turns = Vec::new();
        for info in self.store.threads()? {
            let oldest = turns.iter().map(|t: &TurnSummary| t.start).min();
            if turns.len() >= n && oldest.is_some_and(|o| us(info.active_at) < o) {
                break;
            }
            turns.extend(summarize_turns(&info.id, &self.store.load(&info.id)?));
        }
        turns.sort_by_key(|t| std::cmp::Reverse(t.start));
        turns.truncate(n);

        let (mut text, mut rows) = (String::new(), Vec::new());
        for turn in &turns {
            let problems = self.problems_in(&turn.thread, turn.start, turn.end)?;
            let answer = turn.answer.as_deref().unwrap_or("(no answer)");
            text += &format!(
                "{}  {} turn {}  (#{})\n  > {}\n  < {}\n",
                clock(turn.start),
                turn.thread,
                turn.turn,
                turn.first,
                clip(&turn.input, 160),
                clip(answer, 160),
            );
            for entry in &problems {
                text += &format!("  ! {}\n", line(entry));
            }
            rows.push(json!({
                "thread": turn.thread,
                "turn": turn.turn,
                "at": turn.start,
                "checkpoint": turn.first,
                "input": turn.input,
                "answer": turn.answer,
                "problems": entries_json(&problems),
            }));
        }
        if turns.is_empty() {
            text = "no turns yet\n".into();
        }
        Ok(Output {
            text,
            json: rows.into(),
        })
    }

    /// Warnings and errors since `since` (µs since the epoch), oldest first.
    pub fn problems(&self, since: Option<u64>) -> Result<Output> {
        let entries = self.journal.entries(&Filter {
            min: Some(Severity::Warn),
            since,
            limit: Some(200),
            ..Filter::default()
        })?;
        let mut text: String = entries.iter().map(|e| format!("{}\n", line(e))).collect();
        if entries.is_empty() {
            text = "no warnings or errors\n".into();
        }
        Ok(Output {
            text,
            json: entries_json(&entries),
        })
    }

    /// Where the conversation, the journal and memory mention these words.
    pub fn search(&self, text: &str, limit: usize) -> Result<Output> {
        let wanted = query_words(text);
        let mut said = Vec::new();
        for info in self.store.threads()? {
            for checkpoint in self.store.load(&info.id)? {
                let Event::Message(message) = &checkpoint.event else {
                    continue;
                };
                let content = message_text(message);
                let score = score(&wanted, &content);
                if score > 0 {
                    said.push((score, info.id.clone(), checkpoint, content));
                }
            }
        }
        said.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.at.cmp(&a.2.at)));
        said.truncate(limit);
        let traced = self.journal.search(text, limit)?;
        let remembered = self.memory_matching(&wanted, limit)?;

        let mut out = String::from("In conversations:\n");
        for (_, thread, checkpoint, content) in &said {
            out += &format!(
                "  {}  {thread} #{} turn {}  {}\n",
                clock(us(checkpoint.at)),
                checkpoint.id,
                checkpoint.turn,
                clip(content, 200)
            );
        }
        out += "In the journal:\n";
        for entry in &traced {
            out += &format!("  {}\n", line(entry));
        }
        if self.memory.is_some() {
            out += "In memory:\n";
            for row in &remembered {
                out += &format!("  {}\n", memory_line(row));
            }
        }
        Ok(Output {
            text: out,
            json: json!({
                "conversations": said.iter().map(|(score, thread, c, content)| json!({
                    "score": score, "thread": thread, "checkpoint": c.id, "turn": c.turn,
                    "at": us(c.at), "text": content,
                })).collect::<Vec<_>>(),
                "journal": entries_json(&traced),
                "memory": remembered,
            }),
        })
    }

    /// A thread's checkpoints and journal entries, merged in time order. With `turn`, only
    /// that turn (and what led into it: the routing, the idle work before it). Routine task
    /// bookkeeping (`task state`, `task completed`) only with `all`.
    pub fn thread(&self, id: &str, turn: Option<u32>, all: bool) -> Result<Output> {
        let checkpoints = self.store.load(id)?;
        if checkpoints.is_empty() {
            bail!("no thread {id}");
        }
        let (since, until) = match turn {
            Some(turn) => window(&checkpoints, turn)?,
            None => (None, None),
        };
        let shown: Vec<&Checkpoint> = checkpoints
            .iter()
            .filter(|c| since.is_none_or(|s| us(c.at) >= s) && until.is_none_or(|u| us(c.at) < u))
            .collect();
        let entries = self.journal.entries(&Filter {
            thread: Some(id.to_string()),
            since,
            until,
            ..Filter::default()
        })?;
        let entries: Vec<Entry> = entries
            .into_iter()
            .filter(|e| all || !ROUTINE.contains(&e.message.as_str()))
            .collect();

        // Merge: a checkpoint goes before journal entries from the same microsecond, since
        // those are mostly about it.
        let mut text = String::new();
        let mut rows = Vec::new();
        let mut next = entries.iter().peekable();
        for checkpoint in &shown {
            while let Some(entry) = next.next_if(|e| e.at < us(checkpoint.at)) {
                text += &format!("{}\n", line(entry));
                rows.push(json!({ "entry": entry.to_json() }));
            }
            text += &format!("{}\n", checkpoint_line(checkpoint));
            rows.push(json!({ "checkpoint": checkpoint_json(checkpoint) }));
        }
        for entry in next {
            text += &format!("{}\n", line(entry));
            rows.push(json!({ "entry": entry.to_json() }));
        }
        Ok(Output {
            text,
            json: rows.into(),
        })
    }

    /// A task as stored, the tasks it owns, and its journal entries.
    pub fn task(&self, id: i64) -> Result<Output> {
        let (task, owned) = self.store.read(|conn| {
            let task = rows(conn, "SELECT * FROM tasks WHERE id = ?1", [id])?;
            let owned = rows(
                conn,
                "SELECT id, kind, status FROM tasks WHERE owner = ?1 ORDER BY id",
                [id],
            )?;
            Ok((task.into_iter().next(), owned))
        })?;
        let Some(task) = task else {
            bail!("no task {id}");
        };
        let entries = self.journal.entries(&Filter {
            task: Some(id),
            ..Filter::default()
        })?;
        let mut text = serde_json::to_string_pretty(&task)?;
        text += "\nOwns:\n";
        for child in &owned {
            text += &format!("  {} {} {}\n", child["id"], child["kind"], child["status"]);
        }
        text += "Journal:\n";
        for entry in &entries {
            text += &format!("  {}\n", line(entry));
        }
        Ok(Output {
            text,
            json: json!({
                "task": task,
                "owns": owned,
                "journal": entries_json(&entries),
            }),
        })
    }

    /// One journal entry in full: for a model call, the exact request and raw reply.
    pub fn entry(&self, id: i64) -> Result<Output> {
        let entry = self
            .journal
            .entry(id)?
            .with_context(|| format!("no entry {id}"))?;
        let json = entry.to_json();
        Ok(Output {
            text: serde_json::to_string_pretty(&json)?,
            json,
        })
    }

    /// The context the model would have seen at a checkpoint: system, tools, messages.
    pub fn context(&self, thread: &str, checkpoint: usize) -> Result<Output> {
        let thread = Thread::load(thread, self.store.as_ref())?;
        ensure_checkpoint(&thread, checkpoint)?;
        let ctx = thread.context_at(checkpoint);
        let tools: Vec<&str> = ctx.tools.iter().map(|t| t.name.as_str()).collect();
        let mut text = format!(
            "system: {}\ntools: {}\n",
            ctx.system.as_deref().unwrap_or("(none)"),
            tools.join(", ")
        );
        for (i, message) in ctx.messages.iter().enumerate() {
            text += &format!("[{i}] {:?}: {}\n", message.role, message_text(message));
        }
        Ok(Output {
            text,
            json: json!({
                "system": ctx.system,
                "tools": ctx.tools,
                "messages": ctx.messages,
            }),
        })
    }

    /// Memory items (every version), cards, episodes and resume cards mentioning these words.
    pub fn memory(&self, text: &str, limit: usize) -> Result<Output> {
        if self.memory.is_none() {
            bail!("no memory database (pass --memory <path>)");
        }
        let wanted = query_words(text);
        let rows = self.memory_matching(&wanted, limit)?;
        let mut out = String::new();
        for row in &rows {
            out += &format!("{}\n", memory_line(row));
        }
        Ok(Output {
            text: out,
            json: rows.into(),
        })
    }

    fn memory_matching(&self, wanted: &BTreeSet<String>, limit: usize) -> Result<Vec<Value>> {
        let Some(conn) = &self.memory else {
            return Ok(Vec::new());
        };
        let sources = [
            (
                "item",
                "SELECT id, lineage, version, current, scope, kind, key, source, created_at, body
                 FROM items",
            ),
            (
                "card",
                "SELECT id, thread, scope, to_checkpoint, at, text FROM cards",
            ),
            (
                "episode",
                "SELECT id, thread, scope, from_checkpoint, to_checkpoint, at, text FROM episodes",
            ),
            (
                "resume",
                "SELECT id, thread, scope, to_checkpoint, at, text FROM resumes",
            ),
        ];
        let mut found = Vec::new();
        for (kind, sql) in sources {
            for mut row in rows(conn, sql, [])? {
                row["kind_of"] = kind.into();
                let content = row.to_string();
                let score = score(wanted, &content);
                if score > 0 {
                    found.push((score, row));
                }
            }
        }
        found.sort_by_key(|f| std::cmp::Reverse(f.0));
        Ok(found.into_iter().take(limit).map(|(_, row)| row).collect())
    }

    fn problems_in(&self, thread: &str, since: u64, until: Option<u64>) -> Result<Vec<Entry>> {
        self.journal.entries(&Filter {
            thread: Some(thread.to_string()),
            min: Some(Severity::Warn),
            since: Some(since),
            until,
            ..Filter::default()
        })
    }
}

fn ensure_checkpoint(thread: &Thread, checkpoint: usize) -> Result<()> {
    if checkpoint >= thread.checkpoints().len() {
        bail!(
            "{} has checkpoints 0..{}",
            thread.id,
            thread.checkpoints().len()
        );
    }
    Ok(())
}

/// The turns in a thread's checkpoints.
fn summarize_turns(thread: &str, checkpoints: &[Checkpoint]) -> Vec<TurnSummary> {
    let mut turns: Vec<TurnSummary> = Vec::new();
    for checkpoint in checkpoints {
        match &checkpoint.event {
            Event::TurnStarted => turns.push(TurnSummary {
                thread: thread.to_string(),
                turn: checkpoint.turn,
                start: us(checkpoint.at),
                end: None,
                first: checkpoint.id,
                input: String::new(),
                answer: None,
            }),
            Event::Message(message) => {
                let Some(turn) = turns.last_mut().filter(|t| t.turn == checkpoint.turn) else {
                    continue;
                };
                let content = message.text();
                match message.role {
                    context::Role::User if turn.input.is_empty() => turn.input = content,
                    context::Role::Assistant if !content.is_empty() => turn.answer = Some(content),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for i in 1..turns.len() {
        turns[i - 1].end = Some(turns[i].start);
    }
    turns
}

/// From the end of the turn before `turn` (or the thread's start) to the start of the next:
/// what happened between two turns (routing, idle work) shows with both.
fn window(checkpoints: &[Checkpoint], turn: u32) -> Result<(Option<u64>, Option<u64>)> {
    let started = |t: u32| {
        checkpoints
            .iter()
            .find(|c| c.turn == t && matches!(c.event, Event::TurnStarted))
    };
    let start = started(turn).with_context(|| format!("no turn {turn}"))?;
    let since = checkpoints[..start.id]
        .iter()
        .rev()
        .find(|c| matches!(c.event, Event::TurnEnded))
        .map_or(0, |c| us(c.at) + 1);
    let until = started(turn + 1).map(|c| us(c.at));
    Ok((Some(since), until))
}

fn entries_json(entries: &[Entry]) -> Value {
    entries.iter().map(Entry::to_json).collect()
}

/// The words of a question worth matching on.
fn query_words(text: &str) -> BTreeSet<String> {
    words(text).filter(|w| w.len() > 2).collect()
}

/// How many of the wanted words `text` has.
fn score(wanted: &BTreeSet<String>, text: &str) -> usize {
    let found: BTreeSet<String> = words(text).collect();
    wanted.intersection(&found).count()
}

/// Task bookkeeping a timeline leaves out unless asked: every phase commits one.
const ROUTINE: &[&str] = &["task state", "task completed"];

/// A journal entry on one line: when, its ID, level, message, and its small fields.
fn line(entry: &Entry) -> String {
    // Flags and counters that are almost always off: shown only when set.
    const USUALLY_OFF: &[&str] = &["aborting", "abort", "restarts", "replayed"];
    let mut fields = String::new();
    for (key, value) in &entry.fields {
        let off = matches!(value, Value::Bool(false)) || value == 0;
        let key = key.as_str();
        if key == "thread" || BODIES.contains(&key) || (off && USUALLY_OFF.contains(&key)) {
            continue;
        }
        fields += &format!(" {key}={}", clip(&as_text(value), 120));
    }
    let mut text = format!(
        "{}  [e{}] {:<5} {}{}",
        clock(entry.at),
        entry.id,
        entry.severity.name(),
        entry.message,
        fields
    );
    if BODIES.iter().any(|body| entry.fields.contains_key(*body)) {
        text += &format!("  (full call: entry {})", entry.id);
    }
    text
}

fn checkpoint_line(checkpoint: &Checkpoint) -> String {
    let what = match &checkpoint.event {
        Event::Message(message) => {
            format!("{:?}: {}", message.role, clip(&message_text(message), 300))
        }
        Event::SystemSet(system) => {
            format!("system set: {}", clip(system.as_deref().unwrap_or(""), 120))
        }
        Event::ToolRegistered(tool) => format!("tool registered: {}", tool.name),
        Event::ToolRemoved(name) => format!("tool removed: {name}"),
        Event::TurnStarted => "turn started".into(),
        Event::StepStarted => "step started".into(),
        Event::TurnEnded => "turn ended".into(),
        Event::Replaced { range, with } => {
            format!("replaced messages {range:?} with {} message(s)", with.len())
        }
        Event::Considered { upto, recalled } => {
            let texts: Vec<String> = recalled
                .iter()
                .map(|r| format!("m{}: {}", r.id, clip(&r.text, 80)))
                .collect();
            format!("considered up to #{upto}; recalled [{}]", texts.join("; "))
        }
        Event::Routed { by, reason } => format!("routed here by {by}: {reason}"),
    };
    format!(
        "{}  #{:<4} t{}.{}  {what}",
        clock(us(checkpoint.at)),
        checkpoint.id,
        checkpoint.turn,
        checkpoint.step
    )
}

fn checkpoint_json(checkpoint: &Checkpoint) -> Value {
    json!({
        "id": checkpoint.id,
        "turn": checkpoint.turn,
        "step": checkpoint.step,
        "at": us(checkpoint.at),
        "event": checkpoint.event,
    })
}

fn memory_line(row: &Value) -> String {
    let kind = row["kind_of"].as_str().unwrap_or("?");
    let text = row
        .get("body")
        .or(row.get("text"))
        .map(as_text)
        .unwrap_or_default();
    let mut label = format!("{kind} {}", row["id"]);
    if kind == "item" {
        let current = if row["current"] == 1 {
            "current"
        } else {
            "old"
        };
        label += &format!(
            " (lineage {} v{} {current}, {})",
            row["lineage"], row["version"], row["scope"]
        );
    } else {
        label += &format!(" ({} {})", row["thread"], row["scope"]);
    }
    format!("{label}: {}", clip(&text, 200))
}

/// Text, tool calls and tool results, on one line.
fn message_text(message: &Message) -> String {
    let parts: Vec<String> = message
        .content
        .iter()
        .map(|block| match block {
            Block::Text { text } => text.clone(),
            Block::ToolCall { name, input, .. } => format!("[call {name} {input}]"),
            Block::ToolResult {
                content, is_error, ..
            } => match is_error {
                true => format!("[error result: {content}]"),
                false => format!("[result: {content}]"),
            },
            Block::Thinking { text, .. } if text.trim().is_empty() => String::new(),
            Block::Thinking { text, .. } => format!("[thinking: {text}]"),
            Block::RedactedThinking { .. } => "[redacted thinking]".into(),
        })
        .filter(|part| !part.is_empty())
        .collect();
    parts.join(" ")
}

/// The rows `sql` selects, each as a JSON object of its columns.
fn rows(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Value>> {
    let mut select = conn.prepare(sql)?;
    let names: Vec<String> = select.column_names().map(String::from).collect();
    let rows = select.query_map(params, |row| {
        let object = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), column(row, i)));
        Ok(Value::Object(object.collect()))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn column(row: &rusqlite::Row, i: usize) -> Value {
    use rusqlite::types::ValueRef;
    match row.get_ref(i) {
        Ok(ValueRef::Integer(n)) => n.into(),
        Ok(ValueRef::Real(n)) => n.into(),
        Ok(ValueRef::Text(text)) => {
            let text = String::from_utf8_lossy(text);
            match text.starts_with(['{', '[']) {
                true => serde_json::from_str(&text).unwrap_or(Value::String(text.into())),
                false => Value::String(text.into()),
            }
        }
        _ => Value::Null,
    }
}

/// At most `max` characters, on one line.
pub fn clip(text: &str, max: usize) -> String {
    let flat = text.replace('\n', " ⏎ ");
    match flat.chars().count() > max {
        true => format!("{}…", flat.chars().take(max).collect::<String>()),
        false => flat,
    }
}

#[cfg(test)]
mod tests;
