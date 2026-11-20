//! SQLite checkpointer: one row per checkpoint, keyed by (thread_id, id), event as JSON.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

use crate::{Checkpoint, Checkpointer};

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

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
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
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
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
        }
        tx.commit()?;
        Ok(())
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
                at: SystemTime::UNIX_EPOCH + Duration::from_nanos(at_ns as u64),
                event: serde_json::from_str(&event)
                    .with_context(|| format!("{thread_id} checkpoint {id}"))?,
            })
        })
        .collect()
    }

    fn threads(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut select =
            conn.prepare_cached("SELECT DISTINCT thread_id FROM checkpoints ORDER BY thread_id")?;
        let ids = select.query_map([], |row| row.get(0))?;
        Ok(ids.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Thread;
    use crate::tests::{tool_turn, weather};
    use context::Message;
    use std::sync::Arc;

    #[test]
    fn thread_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("threads.db");
        let _ = std::fs::remove_file(&path);

        let saved = {
            let store = Arc::new(Sqlite::open(&path).unwrap());
            let mut thread = Thread::open("t1", store).unwrap();
            thread.set_system("Be terse.").unwrap();
            thread.register(weather()).unwrap();
            tool_turn(&mut thread, "Weather in Paris?");
            thread.start_turn(Message::user("And tomorrow?")).unwrap();
            thread.checkpoints().to_vec()
        };

        let store = Arc::new(Sqlite::open(&path).unwrap());
        let mut thread = Thread::open("t1", store.clone()).unwrap();
        assert_eq!(thread.checkpoints(), saved);
        assert_eq!(thread.context().messages.len(), 5);
        assert_eq!(thread.turn(), 2);
        assert!(thread.in_turn());

        thread.start_step().unwrap();
        assert_eq!(store.load("t1").unwrap().len(), saved.len() + 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fork_is_saved_and_listed() {
        let store = Arc::new(Sqlite::memory().unwrap());
        let mut thread = Thread::open("t1", store.clone()).unwrap();
        tool_turn(&mut thread, "Weather in Paris?");
        let mut branch = thread.fork(3, "t2").unwrap();
        branch.push(Message::assistant("Rome instead?")).unwrap();

        assert_eq!(store.threads().unwrap(), ["t1", "t2"]);
        let reopened = Thread::open("t2", store.clone()).unwrap();
        assert_eq!(reopened.context(), branch.context());

        // Same id twice is refused, and the thread is left unchanged.
        assert!(thread.fork(3, "t2").is_err());
    }

    #[test]
    fn failed_save_leaves_thread_unchanged() {
        let store = Arc::new(Sqlite::memory().unwrap());
        let mut thread = Thread::open("t1", store.clone()).unwrap();
        thread.set_system("Be terse.").unwrap();
        // Another writer took the next id.
        let mut other = Thread::open("t1", store.clone()).unwrap();
        other.register(weather()).unwrap();

        assert!(thread.register(weather()).is_err());
        assert_eq!(thread.checkpoints().len(), 1);
        assert!(thread.context().tools.is_empty());
    }
}
