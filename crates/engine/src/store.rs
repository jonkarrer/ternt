//! The tasks table, in the same SQLite database as thread checkpoints and written in the same
//! transactions.

use std::collections::BTreeMap;

use anyhow::Result;
use state::Sqlite;
use state::sqlite::rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::task::{TaskId, TaskRecord, TaskState};

pub(crate) fn init(store: &Sqlite) -> Result<()> {
    store.transaction(|tx| {
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                 id INTEGER PRIMARY KEY,
                 thread TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 input TEXT NOT NULL,
                 owner INTEGER,
                 background INTEGER NOT NULL,
                 abandon_on_restart INTEGER NOT NULL,
                 abort_requested INTEGER NOT NULL,
                 abort_reason TEXT,
                 restarts INTEGER NOT NULL,
                 memos TEXT NOT NULL,
                 status TEXT NOT NULL,
                 state TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 ended_at INTEGER
             );
             CREATE INDEX IF NOT EXISTS tasks_status ON tasks (status);
             CREATE INDEX IF NOT EXISTS tasks_owner ON tasks (owner);
             CREATE TABLE IF NOT EXISTS docs (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        Ok(())
    })
}

pub(crate) fn save(tx: &Transaction, task: &TaskRecord) -> Result<()> {
    tx.prepare_cached(
        "INSERT OR REPLACE INTO tasks (id, thread, kind, version, input, owner, background,
             abandon_on_restart, abort_requested, abort_reason, restarts, memos, status, state,
             created_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?
    .execute(params![
        task.id,
        task.thread,
        task.kind,
        task.version,
        serde_json::to_string(&task.input)?,
        task.owner,
        task.background,
        task.abandon_on_restart,
        task.abort_requested,
        task.abort_reason,
        task.restarts,
        serde_json::to_string(&task.memos)?,
        task.state.name(),
        serde_json::to_string(&task.state)?,
        task.created_at as i64,
        task.ended_at.map(|t| t as i64),
    ])?;
    Ok(())
}

const COLUMNS: &str = "id, thread, kind, version, input, owner, background, abandon_on_restart,
    abort_requested, abort_reason, restarts, memos, state, created_at, ended_at";

pub(crate) fn live(conn: &Connection) -> Result<Vec<TaskRecord>> {
    let mut select = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM tasks WHERE status != 'terminal' ORDER BY id"
    ))?;
    let rows = select.query_map([], raw)?;
    rows.map(|row| parse(row?)).collect()
}

pub(crate) fn get(conn: &Connection, id: TaskId) -> Result<Option<TaskRecord>> {
    let row = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"),
            [id],
            raw,
        )
        .optional()?;
    row.map(parse).transpose()
}

pub(crate) fn next_id(conn: &Connection) -> Result<TaskId> {
    Ok(
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM tasks", [], |row| {
            row.get(0)
        })?,
    )
}

type Raw = (
    TaskId,
    String,
    String,
    u32,
    String,
    Option<TaskId>,
    bool,
    bool,
    bool,
    Option<String>,
    u32,
    String,
    String,
    i64,
    Option<i64>,
);

fn raw(row: &Row) -> state::sqlite::rusqlite::Result<Raw> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
    ))
}

fn parse(raw: Raw) -> Result<TaskRecord> {
    let (
        id,
        thread,
        kind,
        version,
        input,
        owner,
        background,
        abandon_on_restart,
        abort_requested,
        abort_reason,
        restarts,
        memos,
        state,
        created_at,
        ended_at,
    ) = raw;
    let memos: BTreeMap<String, serde_json::Value> = serde_json::from_str(&memos)?;
    let state: TaskState = serde_json::from_str(&state)?;
    Ok(TaskRecord {
        id,
        thread,
        kind,
        version,
        input: serde_json::from_str(&input)?,
        owner,
        background,
        abandon_on_restart,
        abort_requested,
        abort_reason,
        restarts,
        memos,
        state,
        created_at: created_at as u64,
        ended_at: ended_at.map(|t| t as u64),
    })
}

pub(crate) fn get_doc(conn: &Connection, key: &str) -> Result<Option<serde_json::Value>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM docs WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
}

/// Writes a document, or deletes it for `None`.
pub(crate) fn put_doc(
    tx: &Transaction,
    key: &str,
    value: Option<&serde_json::Value>,
) -> Result<()> {
    match value {
        Some(value) => tx
            .prepare_cached("INSERT OR REPLACE INTO docs (key, value) VALUES (?1, ?2)")?
            .execute(params![key, serde_json::to_string(value)?])?,
        None => tx
            .prepare_cached("DELETE FROM docs WHERE key = ?1")?
            .execute([key])?,
    };
    Ok(())
}
