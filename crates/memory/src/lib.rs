//! Long-term memory, shared by every thread. Passive: Experience decides what goes in and
//! comes out.
//!
//!   Semantic    What is true: facts and preferences.
//!   Procedural  How things are done: workflows worth repeating.
//!   Episodic    What happened: compact summaries of a thread's checkpoints. The full record
//!               is the thread itself; an episode points back at the range it covers.
//!
//! Semantic and procedural items are versioned. Writing an item with a key that already exists
//! in its scope, or replacing an item, adds a new version; the old one stays in `history`.
//! Search goes through an `Index` (FTS5 keyword search by default), over current versions only.

pub mod index;

pub use index::{Document, Fts, Index};

use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};

/// Who a memory belongs to. Reads take a list of scopes; every write goes to exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scope(String);

impl Scope {
    pub fn global() -> Self {
        Self("global".into())
    }

    pub fn user(id: &str) -> Self {
        Self(format!("user:{id}"))
    }

    pub fn project(id: &str) -> Self {
        Self(format!("project:{id}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Semantic {
        text: String,
    },
    Procedural {
        name: String,
        /// When the procedure applies, in words.
        trigger: String,
        steps: Vec<String>,
    },
}

impl Body {
    pub fn kind(&self) -> &'static str {
        match self {
            Body::Semantic { .. } => "semantic",
            Body::Procedural { .. } => "procedural",
        }
    }

    /// One line of text: what search indexes and what a recall shows.
    pub fn text(&self) -> String {
        match self {
            Body::Semantic { text } => text.clone(),
            Body::Procedural {
                name,
                trigger,
                steps,
            } => format!("{name} (when {trigger}): {}", steps.join(" → ")),
        }
    }
}

/// Where a memory came from: a checkpoint in a thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub thread: String,
    pub checkpoint: usize,
}

/// What a caller writes; the store assigns ids, versions and times.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub scope: Scope,
    /// Identity within the scope: writing the same key again makes a new version.
    pub key: Option<String>,
    pub source: Option<Source>,
    pub body: Body,
}

impl Write {
    pub fn fact(scope: Scope, text: impl Into<String>) -> Self {
        Self::new(scope, Body::Semantic { text: text.into() })
    }

    pub fn procedure(
        scope: Scope,
        name: impl Into<String>,
        trigger: impl Into<String>,
        steps: Vec<String>,
    ) -> Self {
        Self::new(
            scope,
            Body::Procedural {
                name: name.into(),
                trigger: trigger.into(),
                steps,
            },
        )
    }

    pub fn new(scope: Scope, body: Body) -> Self {
        Self {
            scope,
            key: None,
            source: None,
            body,
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn source(mut self, source: Source) -> Self {
        self.source = Some(source);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i64,
    /// The id of the first version; shared by every version.
    pub lineage: i64,
    pub scope: Scope,
    pub key: Option<String>,
    /// Starts at 1.
    pub version: u32,
    /// The version this one replaced.
    pub replaces: Option<i64>,
    pub source: Option<Source>,
    pub created_at: SystemTime,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub item: Item,
    /// Higher is more relevant; only comparable within one search.
    pub score: f64,
}

/// A summary of checkpoints `from..=to` of a thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    pub id: i64,
    pub thread: String,
    pub scope: Scope,
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub at: SystemTime,
}

/// A thread's resume card, written when it settles: what it's about and where it left off,
/// enough to pick it up from the card alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub id: i64,
    pub thread: String,
    pub scope: Scope,
    pub text: String,
    /// The last checkpoint the card covers.
    pub to: usize,
    /// Turns in the thread when it settled: a later turn means it's active again.
    pub turns: u32,
    /// When the last turn it covers was active.
    pub active_at: SystemTime,
    pub at: SystemTime,
}

/// A thread's card: a few lines on what it's about, written while dreaming. Routing reads
/// cards instead of threads.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: i64,
    pub thread: String,
    pub scope: Scope,
    pub text: String,
    /// The last checkpoint the card covers.
    pub to: usize,
    pub at: SystemTime,
}

pub struct Memory {
    conn: Mutex<Connection>,
    index: Box<dyn Index>,
}

const ITEM_COLUMNS: &str = "items.id, items.lineage, items.scope, items.key, items.version, items.replaces, items.source, items.created_at, items.body";

impl Memory {
    /// Items and the default keyword index, both in the file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Self::with_index(path, Box::new(Fts::open(path)?))
    }

    /// Items in the file at `path`, searched through `index`. Call `reindex` after switching
    /// to an index that is empty.
    pub fn with_index(path: impl AsRef<Path>, index: Box<dyn Index>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn, index)
    }

    /// Gone when dropped; for tests.
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, Box::new(Fts::in_memory()?))
    }

    fn init(conn: Connection, index: Box<dyn Index>) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS items (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 lineage INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 key TEXT,
                 version INTEGER NOT NULL,
                 replaces INTEGER,
                 current INTEGER NOT NULL,
                 source TEXT,
                 created_at INTEGER NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS items_lineage ON items (lineage, current);
             CREATE INDEX IF NOT EXISTS items_key ON items (kind, scope, key, current);
             CREATE INDEX IF NOT EXISTS items_scope ON items (scope, current, created_at);
             CREATE TABLE IF NOT EXISTS episodes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 thread TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 from_checkpoint INTEGER NOT NULL,
                 to_checkpoint INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS episodes_thread ON episodes (thread, to_checkpoint);
             CREATE TABLE IF NOT EXISTS resumes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 thread TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 text TEXT NOT NULL,
                 to_checkpoint INTEGER NOT NULL,
                 turns INTEGER NOT NULL,
                 active_at INTEGER NOT NULL,
                 at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS resumes_thread ON resumes (thread, id);
             CREATE TABLE IF NOT EXISTS cards (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 thread TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 text TEXT NOT NULL,
                 to_checkpoint INTEGER NOT NULL,
                 at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS cards_thread ON cards (thread, id);",
        )?;
        // The card index gained a scope column: an index from before it is rebuilt from the
        // cards, newest per thread.
        if conn.prepare("SELECT scope FROM cards_fts LIMIT 0").is_err() {
            conn.execute_batch(
                "DROP TABLE IF EXISTS cards_fts;
                 CREATE VIRTUAL TABLE cards_fts
                     USING fts5(thread UNINDEXED, scope UNINDEXED, text, tokenize = 'porter unicode61');
                 INSERT INTO cards_fts (thread, scope, text)
                     SELECT thread, scope, text FROM cards
                     WHERE id IN (SELECT MAX(id) FROM cards GROUP BY thread);",
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
            index,
        })
    }

    /// Adds an item, or a new version when its key already exists in the scope.
    pub fn write(&self, write: Write) -> Result<Item> {
        self.insert(write, |tx, write| {
            let Some(key) = &write.key else {
                return Ok(None);
            };
            Ok(tx
                .query_row(
                    "SELECT id FROM items
                     WHERE kind = ?1 AND scope = ?2 AND key = ?3 AND current = 1",
                    params![write.body.kind(), write.scope.as_str(), key],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }

    /// A new version of the item `id`, which must be current. Keeps its key unless `write`
    /// names one.
    pub fn replace(&self, id: i64, write: Write) -> Result<Item> {
        self.insert(write, |_, _| Ok(Some(id)))
    }

    /// Inserts `write`, as a new version of the item `replaces` picks, if any. One transaction,
    /// so concurrent writes to the same item can't both make a new version of it.
    fn insert(
        &self,
        write: Write,
        replaces: impl FnOnce(&Transaction, &Write) -> Result<Option<i64>>,
    ) -> Result<Item> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let old = match replaces(&tx, &write)? {
            Some(id) => {
                let Some(old) = current(&tx, id)? else {
                    bail!("memory {id} is not a current item");
                };
                if old.body.kind() != write.body.kind() {
                    bail!(
                        "memory {id} is {}, not {}",
                        old.body.kind(),
                        write.body.kind()
                    );
                }
                // A version history stays in one scope: one user's fact never becomes another's.
                if old.scope != write.scope {
                    bail!("memory {id} is in {}, not {}", old.scope, write.scope);
                }
                tx.execute("UPDATE items SET current = 0 WHERE id = ?1", [id])?;
                Some(old)
            }
            None => None,
        };
        let source = write
            .source
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            "INSERT INTO items (lineage, kind, scope, key, version, replaces, current, source, created_at, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9)",
            params![
                old.as_ref().map_or(0, |o| o.lineage),
                write.body.kind(),
                write.scope.as_str(),
                write.key.as_ref().or(old.as_ref().and_then(|o| o.key.as_ref())),
                old.as_ref().map_or(1, |o| o.version + 1),
                old.as_ref().map(|o| o.id),
                source,
                millis(now()),
                serde_json::to_string(&write.body)?,
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE items SET lineage = id WHERE id = ?1 AND lineage = 0",
            [id],
        )?;
        let item = tx.query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = ?1"),
            [id],
            item_row,
        )?;
        tx.commit()?;
        // After the commit: an index that falls behind is rebuilt by `reindex`. Still holding
        // the lock, so concurrent writes reach the index in the order they were committed.
        if let Some(old) = &old {
            self.index.remove(&[old.id])?;
        }
        self.index.upsert(&[document(&item)])?;
        tracing::debug!(
            id = item.id,
            lineage = item.lineage,
            version = item.version,
            scope = item.scope.as_str(),
            key = ?item.key,
            kind = item.body.kind(),
            source = ?item.source,
            text = %item.body.text(),
            "memory written"
        );
        Ok(item)
    }

    /// The current version only: `None` for an id that was replaced or forgotten.
    pub fn get(&self, id: i64) -> Result<Option<Item>> {
        current(&self.conn.lock().unwrap(), id)
    }

    /// Every version of the item `id` belongs to, newest first.
    pub fn history(&self, id: i64) -> Result<Vec<Item>> {
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items
             WHERE lineage = (SELECT lineage FROM items WHERE id = ?1)
             ORDER BY version DESC"
        ))?;
        let items = select.query_map([id], item_row)?;
        Ok(items.collect::<Result<_, _>>()?)
    }

    /// Removes the item and every version of it.
    pub fn forget(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let ids: Vec<i64> = conn
            .prepare_cached(
                "DELETE FROM items WHERE lineage = (SELECT lineage FROM items WHERE id = ?1)
                 RETURNING id",
            )?
            .query_map([id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        tracing::debug!(id, removed = ?ids, "memory forgotten");
        self.index.remove(&ids)
    }

    /// Current versions in `scopes` relevant to `query`, best first, as ranked by the index.
    pub fn search(&self, query: &str, scopes: &[Scope], limit: usize) -> Result<Vec<Hit>> {
        let mut hits = Vec::new();
        for (id, score) in self.index.search(query, scopes, limit)? {
            // Skips anything the index still holds that is no longer current.
            if let Some(item) = self.get(id)? {
                hits.push(Hit { item, score });
            }
        }
        Ok(hits)
    }

    /// Rebuilds the index from every current item: after switching index, or if it fell behind.
    pub fn reindex(&self) -> Result<()> {
        // Holds the lock throughout, so no write lands between reading items and indexing them.
        let conn = self.conn.lock().unwrap();
        let documents = conn
            .prepare(&format!(
                "SELECT {ITEM_COLUMNS} FROM items WHERE current = 1"
            ))?
            .query_map([], item_row)?
            .map(|item| item.map(|item| document(&item)))
            .collect::<Result<Vec<_>, _>>()?;
        self.index.clear()?;
        self.index.upsert(&documents)
    }

    /// Current versions in `scopes`, newest first.
    pub fn list(&self, scopes: &[Scope], limit: usize) -> Result<Vec<Item>> {
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM items
             WHERE current = 1 AND scope IN ({})
             ORDER BY created_at DESC, id DESC LIMIT {limit}",
            placeholders(scopes.len())
        ))?;
        let items = select.query_map(params_from_iter(scopes.iter().map(|s| &s.0)), item_row)?;
        Ok(items.collect::<Result<_, _>>()?)
    }

    pub fn add_episode(
        &self,
        thread: &str,
        scope: Scope,
        from: usize,
        to: usize,
        text: impl Into<String>,
    ) -> Result<Episode> {
        let text = text.into();
        let at = now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO episodes (thread, scope, from_checkpoint, to_checkpoint, text, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                thread,
                scope.as_str(),
                from as i64,
                to as i64,
                text,
                millis(at)
            ],
        )?;
        Ok(Episode {
            id: conn.last_insert_rowid(),
            thread: thread.into(),
            scope,
            from,
            to,
            text,
            at,
        })
    }

    /// A thread's episodes, oldest first.
    pub fn episodes(&self, thread: &str) -> Result<Vec<Episode>> {
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(
            "SELECT id, thread, scope, from_checkpoint, to_checkpoint, text, at
             FROM episodes WHERE thread = ?1 ORDER BY to_checkpoint, id",
        )?;
        let episodes = select.query_map([thread], |row| {
            Ok(Episode {
                id: row.get(0)?,
                thread: row.get(1)?,
                scope: Scope(row.get(2)?),
                from: row.get::<_, i64>(3)? as usize,
                to: row.get::<_, i64>(4)? as usize,
                text: row.get(5)?,
                at: from_millis(row.get(6)?),
            })
        })?;
        Ok(episodes.collect::<Result<_, _>>()?)
    }

    pub fn add_resume(
        &self,
        thread: &str,
        scope: Scope,
        text: impl Into<String>,
        to: usize,
        turns: u32,
        active_at: SystemTime,
    ) -> Result<Resume> {
        let text = text.into();
        let at = now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO resumes (thread, scope, text, to_checkpoint, turns, active_at, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                thread,
                scope.as_str(),
                text,
                to as i64,
                turns,
                millis(active_at),
                millis(at)
            ],
        )?;
        Ok(Resume {
            id: conn.last_insert_rowid(),
            thread: thread.into(),
            scope,
            text,
            to,
            turns,
            active_at: from_millis(millis(active_at)),
            at,
        })
    }

    /// The thread's newest resume card.
    pub fn resume(&self, thread: &str) -> Result<Option<Resume>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT id, thread, scope, text, to_checkpoint, turns, active_at, at
                 FROM resumes WHERE thread = ?1 ORDER BY id DESC LIMIT 1",
                [thread],
                |row| {
                    Ok(Resume {
                        id: row.get(0)?,
                        thread: row.get(1)?,
                        scope: Scope(row.get(2)?),
                        text: row.get(3)?,
                        to: row.get::<_, i64>(4)? as usize,
                        turns: row.get(5)?,
                        active_at: from_millis(row.get(6)?),
                        at: from_millis(row.get(7)?),
                    })
                },
            )
            .optional()?)
    }

    pub fn add_card(
        &self,
        thread: &str,
        scope: Scope,
        text: impl Into<String>,
        to: usize,
    ) -> Result<Card> {
        let text = text.into();
        let at = now();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO cards (thread, scope, text, to_checkpoint, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread, scope.as_str(), text, to as i64, millis(at)],
        )?;
        let id = tx.last_insert_rowid();
        // Only the newest card per thread is searchable.
        tx.execute("DELETE FROM cards_fts WHERE thread = ?1", [thread])?;
        tx.execute(
            "INSERT INTO cards_fts (thread, scope, text) VALUES (?1, ?2, ?3)",
            params![thread, scope.as_str(), text],
        )?;
        tx.commit()?;
        Ok(Card {
            id,
            thread: thread.into(),
            scope,
            text,
            to,
            at,
        })
    }

    /// Threads in `scopes` whose newest card matches any word of `query`, best first, with
    /// scores. Scoped, so one user's cards never match another's threads.
    pub fn search_cards(
        &self,
        query: &str,
        scopes: &[Scope],
        limit: usize,
    ) -> Result<Vec<(String, f64)>> {
        let Some(terms) = index::fts_query(query) else {
            return Ok(Vec::new());
        };
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(&format!(
            "SELECT thread, -bm25(cards_fts) AS score FROM cards_fts
             WHERE cards_fts MATCH ? AND scope IN ({}) ORDER BY score DESC LIMIT {limit}",
            placeholders(scopes.len())
        ))?;
        let args = std::iter::once(terms).chain(scopes.iter().map(|s| s.0.clone()));
        let hits =
            select.query_map(params_from_iter(args), |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(hits.collect::<Result<_, _>>()?)
    }

    /// The thread's newest card.
    pub fn card(&self, thread: &str) -> Result<Option<Card>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT id, thread, scope, text, to_checkpoint, at
                 FROM cards WHERE thread = ?1 ORDER BY id DESC LIMIT 1",
                [thread],
                |row| {
                    Ok(Card {
                        id: row.get(0)?,
                        thread: row.get(1)?,
                        scope: Scope(row.get(2)?),
                        text: row.get(3)?,
                        to: row.get::<_, i64>(4)? as usize,
                        at: from_millis(row.get(5)?),
                    })
                },
            )
            .optional()?)
    }
}

/// The item `id`, if it is the current version.
fn current(conn: &Connection, id: i64) -> Result<Option<Item>> {
    Ok(conn
        .query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = ?1 AND current = 1"),
            [id],
            item_row,
        )
        .optional()?)
}

fn item_row(row: &Row) -> rusqlite::Result<Item> {
    let parse_err = |i, e: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
    };
    let source = row
        .get::<_, Option<String>>(6)?
        .map(|s| serde_json::from_str(&s).map_err(|e| parse_err(6, e)))
        .transpose()?;
    Ok(Item {
        id: row.get(0)?,
        lineage: row.get(1)?,
        scope: Scope(row.get(2)?),
        key: row.get(3)?,
        version: row.get(4)?,
        replaces: row.get(5)?,
        source,
        created_at: from_millis(row.get(7)?),
        body: serde_json::from_str(&row.get::<_, String>(8)?).map_err(|e| parse_err(8, e))?,
    })
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn from_millis(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms as u64)
}

/// Now, to the millisecond, as it reads back once stored.
fn now() -> SystemTime {
    from_millis(millis(SystemTime::now()))
}

fn document(item: &Item) -> Document {
    Document {
        id: item.id,
        scope: item.scope.clone(),
        kind: item.body.kind().into(),
        text: item.body.text(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Scope {
        Scope::user("jon")
    }

    #[test]
    fn search_finds_current_facts_in_scope() {
        let memory = Memory::in_memory().unwrap();
        memory
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        memory
            .write(Write::fact(me(), "The user prefers tabs over spaces."))
            .unwrap();
        memory
            .write(Write::fact(
                Scope::project("x"),
                "Project x is named Jon's tool.",
            ))
            .unwrap();

        let hits = memory.search("What's my name?", &[me()], 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.body.text(), "The user's name is Jon.");
        // "naming" stems to the same word as "name".
        assert_eq!(memory.search("naming", &[me()], 5).unwrap().len(), 1);
        assert!(memory.search("what is it", &[me()], 5).unwrap().is_empty());
    }

    #[test]
    fn same_key_makes_a_new_version() {
        let memory = Memory::in_memory().unwrap();
        let first = memory
            .write(Write::fact(me(), "The user uses npm.").key("package-manager"))
            .unwrap();
        let second = memory
            .write(Write::fact(me(), "The user uses pnpm.").key("package-manager"))
            .unwrap();

        assert_eq!((second.version, second.replaces), (2, Some(first.id)));
        assert_eq!(second.lineage, first.id);
        assert!(memory.get(first.id).unwrap().is_none());
        assert_eq!(
            memory.history(first.id).unwrap(),
            [second.clone(), first.clone()]
        );
        let hits = memory.search("npm pnpm", &[me()], 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item, second);

        // Same key in another scope is a separate item.
        let other = memory
            .write(Write::fact(Scope::global(), "Default is npm.").key("package-manager"))
            .unwrap();
        assert_eq!(other.version, 1);
    }

    /// Memory is shared between threads: writes to one key must still form one lineage.
    #[test]
    fn concurrent_writes_to_a_key_keep_one_current_version() {
        let memory = Memory::in_memory().unwrap();
        std::thread::scope(|s| {
            for t in 0..8 {
                let memory = &memory;
                s.spawn(move || {
                    for i in 0..25 {
                        let fact = Write::fact(me(), format!("Fact {t}-{i}.")).key("k");
                        memory.write(fact).unwrap();
                    }
                });
            }
        });
        let current = memory.list(&[me()], 500).unwrap();
        assert_eq!(current.len(), 1);
        let history = memory.history(current[0].id).unwrap();
        let versions: Vec<u32> = history.iter().map(|item| item.version).collect();
        assert_eq!(versions, (1..=200).rev().collect::<Vec<_>>());
    }

    /// A replacement without a key keeps the item's key, so later writes to it still version it.
    #[test]
    fn replace_keeps_the_key() {
        let memory = Memory::in_memory().unwrap();
        let first = memory
            .write(Write::fact(me(), "The user uses npm.").key("package-manager"))
            .unwrap();
        let fixed = memory
            .replace(first.id, Write::fact(me(), "The user uses yarn."))
            .unwrap();
        assert_eq!(fixed.key.as_deref(), Some("package-manager"));
        let latest = memory
            .write(Write::fact(me(), "The user uses pnpm.").key("package-manager"))
            .unwrap();
        assert_eq!((latest.version, latest.lineage), (3, first.id));
        assert_eq!(memory.list(&[me()], 10).unwrap(), [latest]);
    }

    #[test]
    fn replace_and_forget() {
        let memory = Memory::in_memory().unwrap();
        let fact = memory
            .write(Write::fact(me(), "Deploys are daily."))
            .unwrap();
        let fixed = memory
            .replace(fact.id, Write::fact(me(), "Deploys are hourly."))
            .unwrap();
        assert!(memory.replace(fact.id, Write::fact(me(), "x")).is_err());
        let procedure = Write::procedure(me(), "Deploy", "deploying", vec!["test".into()]);
        assert!(memory.replace(fixed.id, procedure).is_err());
        // Never into another scope.
        let elsewhere = Write::fact(Scope::user("ann"), "Deploys are weekly.");
        assert!(memory.replace(fixed.id, elsewhere).is_err());
        assert!(memory.get(fixed.id).unwrap().is_some());

        memory.forget(fact.id).unwrap();
        assert!(memory.history(fixed.id).unwrap().is_empty());
        assert!(memory.search("deploys", &[me()], 5).unwrap().is_empty());
    }

    #[test]
    fn procedures_and_listing() {
        let memory = Memory::in_memory().unwrap();
        let source = Source {
            thread: "t1".into(),
            checkpoint: 4,
        };
        let procedure = memory
            .write(
                Write::procedure(
                    me(),
                    "Release",
                    "the user asks to ship",
                    vec!["run tests".into(), "tag".into(), "push".into()],
                )
                .source(source.clone()),
            )
            .unwrap();
        assert_eq!(procedure.source, Some(source));
        assert_eq!(
            procedure.body.text(),
            "Release (when the user asks to ship): run tests → tag → push"
        );
        memory.write(Write::fact(me(), "Newest.")).unwrap();

        let listed = memory.list(&[me()], 10).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].body.text(), "Newest.");
        assert_eq!(
            memory.search("ship", &[me()], 5).unwrap()[0].item,
            procedure
        );
    }

    #[test]
    fn episodes_by_thread() {
        let memory = Memory::in_memory().unwrap();
        memory
            .add_episode("t1", me(), 0, 9, "Asked about Paris weather.")
            .unwrap();
        memory
            .add_episode("t2", me(), 0, 3, "Other thread.")
            .unwrap();
        memory
            .add_episode("t1", me(), 10, 20, "Then Rome.")
            .unwrap();
        let texts: Vec<_> = memory
            .episodes("t1")
            .unwrap()
            .into_iter()
            .map(|e| (e.from, e.to, e.text))
            .collect();
        assert_eq!(
            texts,
            [
                (0, 9, "Asked about Paris weather.".to_string()),
                (10, 20, "Then Rome.".to_string())
            ]
        );
    }

    /// A second index kept in step with the first, then rebuilt from items.
    #[test]
    fn reindex_rebuilds_a_new_index() {
        let db = TempDb::new("reindex");
        let path = &db.0;
        let fact = Memory::open(path)
            .unwrap()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();

        let memory = Memory::with_index(path, Box::new(Fts::in_memory().unwrap())).unwrap();
        assert!(memory.search("name", &[me()], 5).unwrap().is_empty());
        memory.reindex().unwrap();
        assert_eq!(memory.search("name", &[me()], 5).unwrap()[0].item, fact);
    }

    #[test]
    fn newest_resume_and_card_win() {
        let memory = Memory::in_memory().unwrap();
        assert!(memory.resume("t1").unwrap().is_none());
        let now = SystemTime::now();
        memory
            .add_resume("t1", me(), "About: Kyoto.", 9, 2, now)
            .unwrap();
        let newer = memory
            .add_resume("t1", me(), "About: Kyoto, booked.", 20, 4, now)
            .unwrap();
        memory.add_resume("t2", me(), "Other.", 3, 1, now).unwrap();
        assert_eq!(memory.resume("t1").unwrap(), Some(newer));

        assert!(memory.card("t1").unwrap().is_none());
        memory.add_card("t1", me(), "Osaka trip.", 9).unwrap();
        let card = memory
            .add_card("t1", me(), "Kyoto trip, ryokan booked.", 20)
            .unwrap();
        assert_eq!(memory.card("t1").unwrap(), Some(card));

        memory
            .add_card("t2", me(), "Metabase dashboards on Postgres.", 4)
            .unwrap();
        let threads = |q| -> Vec<String> {
            memory
                .search_cards(q, &[me()], 5)
                .unwrap()
                .into_iter()
                .map(|(thread, _)| thread)
                .collect()
        };
        assert_eq!(threads("booked ryokan"), ["t1"]);
        assert_eq!(threads("dashboard"), ["t2"]);
        // Only the newest card counts: t1's first card said Osaka.
        assert!(threads("osaka").is_empty());
        assert!(threads("weather").is_empty());

        // Another user's card never matches.
        memory
            .add_card("t3", Scope::user("ann"), "Ann's ryokan booking.", 2)
            .unwrap();
        assert_eq!(threads("ryokan"), ["t1"]);
        let anns = memory
            .search_cards("ryokan", &[Scope::user("ann")], 5)
            .unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].0, "t3");
    }

    #[test]
    fn survives_reopen() {
        let db = TempDb::new("reopen");
        let written = Memory::open(&db.0)
            .unwrap()
            .write(Write::fact(me(), "The user's name is Jon.").key("name"))
            .unwrap();
        let memory = Memory::open(&db.0).unwrap();
        assert_eq!(memory.get(written.id).unwrap(), Some(written));
        assert_eq!(memory.search("name", &[me()], 5).unwrap().len(), 1);
    }

    /// A database file in the temp dir, removed (with its WAL files) when dropped.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(name: &str) -> Self {
            let db =
                Self(std::env::temp_dir().join(format!("memory-{name}-{}.db", std::process::id())));
            db.remove();
            db
        }

        fn remove(&self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            self.remove();
        }
    }

    /// A card index from before cards had a scope in it is rebuilt on open.
    #[test]
    fn rebuilds_an_unscoped_card_index() {
        let db = TempDb::new("unscoped-cards");
        {
            let memory = Memory::open(&db.0).unwrap();
            memory.add_card("t1", me(), "Old Osaka card.", 3).unwrap();
            memory
                .add_card("t1", me(), "Kyoto ryokan booked.", 9)
                .unwrap();
        }
        let conn = Connection::open(&db.0).unwrap();
        conn.execute_batch(
            "DROP TABLE cards_fts;
             CREATE VIRTUAL TABLE cards_fts USING fts5(thread UNINDEXED, text);",
        )
        .unwrap();
        drop(conn);

        let memory = Memory::open(&db.0).unwrap();
        let hits = memory.search_cards("ryokan", &[me()], 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(memory.search_cards("osaka", &[me()], 5).unwrap().is_empty());
    }
}
