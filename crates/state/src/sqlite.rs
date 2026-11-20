//! SQLite store: one row per checkpoint, keyed by (thread_id, id), event as JSON. The engine
//! adds its own tables to the same database and writes them in the same transactions
//! (`transaction`), so a commit is one SQLite transaction whatever it touches.
//!
//! WAL with `synchronous = NORMAL`: commits survive a process crash; the newest may be lost if
//! the whole machine loses power.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
pub use rusqlite;
use rusqlite::{Connection, Transaction, params};

use context::Message;

use crate::{Checkpoint, Checkpointer, Thread, ThreadInfo};

pub struct Sqlite {
    conn: Mutex<Connection>,
}

impl Sqlite {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    /// Gone when dropped; for tests.
    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Runs `f` in one SQLite transaction on the store's connection; committed when `f`
    /// returns `Ok`, rolled back otherwise.
    pub fn transaction<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Reads through the store's connection.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&self.conn.lock().unwrap())
    }

    /// Appends checkpoints inside a transaction from `transaction`.
    pub fn save_in(tx: &Transaction, thread_id: &str, checkpoints: &[Checkpoint]) -> Result<()> {
        let mut insert = tx.prepare_cached(
            "INSERT INTO checkpoints (thread_id, id, turn, step, at_ns, event)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for c in checkpoints {
            let at_ns = c.at.duration_since(UNIX_EPOCH)?.as_nanos() as i64;
            insert
                .execute(params![
                    thread_id,
                    c.id as i64,
                    c.turn,
                    c.step,
                    at_ns,
                    serde_json::to_string(&c.event)?,
                ])
                .with_context(|| format!("saving {thread_id} checkpoint {}", c.id))?;
        }
        Ok(())
    }

    /// Saves one thread's unsaved checkpoints, for use outside the engine. On failure the
    /// thread keeps them unsaved.
    pub fn save_thread(&self, thread: &mut Thread) -> Result<()> {
        self.save(&thread.id, thread.unsaved())?;
        thread.mark_saved();
        Ok(())
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS checkpoints (
                 thread_id TEXT NOT NULL,
                 id INTEGER NOT NULL,
                 turn INTEGER NOT NULL,
                 step INTEGER NOT NULL,
                 at_ns INTEGER NOT NULL,
                 event TEXT NOT NULL,
                 PRIMARY KEY (thread_id, id)
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl Checkpointer for Sqlite {
    fn save(&self, thread_id: &str, checkpoints: &[Checkpoint]) -> Result<()> {
        self.transaction(|tx| Self::save_in(tx, thread_id, checkpoints))
    }

    fn load(&self, thread_id: &str) -> Result<Vec<Checkpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(
            "SELECT id, turn, step, at_ns, event FROM checkpoints
             WHERE thread_id = ?1 ORDER BY id",
        )?;
        let rows = select.query_map([thread_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, turn, step, at_ns, event) = row?;
            Ok(Checkpoint {
                id: id as usize,
                turn,
                step,
                at: from_nanos(at_ns),
                event: serde_json::from_str(&event)
                    .with_context(|| format!("{thread_id} checkpoint {id}"))?,
            })
        })
        .collect()
    }

    fn recent_messages(&self, thread_id: &str, n: usize) -> Result<Vec<Message>> {
        self.recent(thread_id, n)
    }

    fn threads(&self) -> Result<Vec<ThreadInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(
            "SELECT thread_id, MAX(turn), COUNT(*), MIN(at_ns), MAX(at_ns),
                 COALESCE(MAX(CASE WHEN json_extract(event, '$.type') = 'message' THEN at_ns END), MIN(at_ns)) AS active,
                 (SELECT json_extract(first.event, '$.data') FROM checkpoints first
                  WHERE first.thread_id = c.thread_id AND json_extract(first.event, '$.type') = 'message'
                  ORDER BY first.id LIMIT 1)
             FROM checkpoints c GROUP BY thread_id ORDER BY active DESC, thread_id",
        )?;
        let rows = select.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        rows.map(|row| {
            let (id, turns, checkpoints, created, updated, active, first) = row?;
            let first: Option<Message> = first.map(|m| serde_json::from_str(&m)).transpose()?;
            Ok(ThreadInfo {
                id,
                title: first.map(|m| title(&m.text())).unwrap_or_default(),
                turns,
                checkpoints: checkpoints as usize,
                created_at: from_nanos(created),
                updated_at: from_nanos(updated),
                active_at: from_nanos(active),
            })
        })
        .collect()
    }
}

impl Sqlite {
    fn recent(&self, thread_id: &str, n: usize) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        // Walks the thread's rows backwards on the primary key and stops after `n` messages.
        let mut select = conn.prepare_cached(
            "SELECT json_extract(event, '$.data') FROM checkpoints
             WHERE thread_id = ?1 AND json_extract(event, '$.type') = 'message'
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = select.query_map(params![thread_id, n as i64], |row| row.get::<_, String>(0))?;
        let mut messages = rows
            .map(|json| Ok(serde_json::from_str(&json?)?))
            .collect::<Result<Vec<Message>>>()?;
        messages.reverse();
        Ok(messages)
    }
}

fn from_nanos(ns: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(ns as u64)
}

/// The first line, cut to 60 characters.
fn title(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(60) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{tool_turn, weather};
    use context::Message;

    #[test]
    fn thread_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("threads.db");
        let _ = std::fs::remove_file(&path);

        let saved = {
            let store = Sqlite::open(&path).unwrap();
            let mut thread = Thread::new("t1");
            thread.set_system("Be terse.").unwrap();
            thread.register(weather()).unwrap();
            tool_turn(&mut thread, "Weather in Paris?");
            thread.start_turn(Message::user("And tomorrow?")).unwrap();
            store.save_thread(&mut thread).unwrap();
            assert!(thread.unsaved().is_empty());
            thread.checkpoints().to_vec()
        };

        let store = Sqlite::open(&path).unwrap();
        let mut thread = Thread::load("t1", &store).unwrap();
        assert_eq!(thread.checkpoints(), saved);
        assert_eq!(thread.context().messages.len(), 5);
        assert_eq!(thread.turn(), 2);
        assert!(thread.in_turn());
        assert!(thread.unsaved().is_empty());

        thread.start_step().unwrap();
        assert_eq!(thread.unsaved().len(), 1);
        store.save_thread(&mut thread).unwrap();
        assert_eq!(store.load("t1").unwrap().len(), saved.len() + 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn threads_are_listed_by_activity() {
        let store = Sqlite::memory().unwrap();
        let mut paris = Thread::new("paris");
        tool_turn(&mut paris, "Weather in Paris?\nAnd a second line.");
        let mut rome = Thread::new("rome");
        rome.start_turn(Message::user("Weather in Rome?")).unwrap();
        // Not activity: routing and compaction leave `active_at` alone.
        paris.routed("rule", "test").unwrap();
        store.save_thread(&mut paris).unwrap();
        store.save_thread(&mut rome).unwrap();

        let threads = store.threads().unwrap();
        let ids: Vec<_> = threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["rome", "paris"]);
        let paris = &threads[1];
        assert_eq!(
            (paris.title.as_str(), paris.turns),
            ("Weather in Paris?", 1)
        );
        assert!(paris.updated_at > paris.active_at);
        assert_eq!(paris.checkpoints, 9);

        let recent = store.recent_messages("paris", 2).unwrap();
        let texts: Vec<_> = recent.iter().map(|m| m.text()).collect();
        // The tool result, then the reply.
        assert_eq!(texts, ["", "18C and sunny."]);
        assert!(store.recent_messages("nobody", 2).unwrap().is_empty());
    }

    #[test]
    fn fork_is_saved_and_listed() {
        let store = Sqlite::memory().unwrap();
        let mut thread = Thread::new("t1");
        tool_turn(&mut thread, "Weather in Paris?");
        store.save_thread(&mut thread).unwrap();
        let mut branch = thread.fork(3, "t2");
        branch.push(Message::assistant("Rome instead?")).unwrap();
        assert_eq!(branch.unsaved().len(), 5);
        store.save_thread(&mut branch).unwrap();

        let ids: Vec<_> = store.threads().unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["t2", "t1"]);
        let reopened = Thread::load("t2", &store).unwrap();
        assert_eq!(reopened.context(), branch.context());

        // Same id twice is refused.
        assert!(store.save_thread(&mut thread.fork(3, "t2")).is_err());
    }

    #[test]
    fn failed_save_keeps_changes_unsaved() {
        let store = Sqlite::memory().unwrap();
        let mut thread = Thread::new("t1");
        thread.set_system("Be terse.").unwrap();
        // Another writer took the first id.
        let mut other = Thread::new("t1");
        other.register(weather()).unwrap();
        store.save_thread(&mut other).unwrap();

        assert!(store.save_thread(&mut thread).is_err());
        assert_eq!(thread.unsaved().len(), 1);
        assert_eq!(store.load("t1").unwrap().len(), 1);
    }

    #[test]
    fn rollback_undoes_a_failed_commit() {
        let mut thread = Thread::new("t1");
        tool_turn(&mut thread, "Weather in Paris?");
        thread.mark_saved();
        let before = (
            thread.checkpoints().to_vec(),
            thread.context().clone(),
            thread.turn(),
        );

        let savepoint = thread.savepoint();
        thread.start_turn(Message::user("And Rome?")).unwrap();
        thread.replace(0..2, vec![]).unwrap();
        thread.rollback(savepoint);

        let after = (
            thread.checkpoints().to_vec(),
            thread.context().clone(),
            thread.turn(),
        );
        assert_eq!(after, before);
        assert!(thread.unsaved().is_empty());
        assert!(!thread.in_turn());
    }
}
