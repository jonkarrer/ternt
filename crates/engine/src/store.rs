//! The tasks table, in the same SQLite database as thread checkpoints and written in the same
//! transactions.

use anyhow::Result;
use state::Sqlite;
use state::sqlite::rusqlite::{Connection, OptionalExtension, Params, Row, Transaction, params};

use crate::task::{TaskId, TaskRecord};

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
    select(conn, "status != 'terminal' ORDER BY id", [])
}

pub(crate) fn get(conn: &Connection, id: TaskId) -> Result<Option<TaskRecord>> {
    Ok(select(conn, "id = ?1", [id])?.pop())
}

pub(crate) fn next_id(conn: &Connection) -> Result<TaskId> {
    Ok(
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM tasks", [], |row| {
            row.get(0)
        })?,
    )
}

/// The tasks matching `filter`, an SQL `WHERE` clause.
fn select(conn: &Connection, filter: &str, params: impl Params) -> Result<Vec<TaskRecord>> {
    let mut select = conn.prepare_cached(&format!("SELECT {COLUMNS} FROM tasks WHERE {filter}"))?;
    let mut rows = select.query(params)?;
    let mut tasks = Vec::new();
    while let Some(row) = rows.next()? {
        tasks.push(parse(row)?);
    }
    Ok(tasks)
}

fn parse(row: &Row) -> Result<TaskRecord> {
    let text = |i: usize| -> Result<String> { Ok(row.get(i)?) };
    Ok(TaskRecord {
        id: row.get(0)?,
        thread: row.get(1)?,
        kind: row.get(2)?,
        version: row.get(3)?,
        input: serde_json::from_str(&text(4)?)?,
        owner: row.get(5)?,
        background: row.get(6)?,
        abandon_on_restart: row.get(7)?,
        abort_requested: row.get(8)?,
        abort_reason: row.get(9)?,
        restarts: row.get(10)?,
        memos: serde_json::from_str(&text(11)?)?,
        state: serde_json::from_str(&text(12)?)?,
        created_at: row.get::<_, i64>(13)? as u64,
        ended_at: row.get::<_, Option<i64>>(14)?.map(|t| t as u64),
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
