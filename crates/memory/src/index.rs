//! The search backend behind `Memory`. Keyword (FTS5) now; an embedding index can implement
//! the same trait later. An index holds only what it needs to rank: ids, scopes and text.
//! `Memory` keeps it in step and loads the items; `Memory::reindex` rebuilds it from scratch.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params, params_from_iter};

use crate::Scope;

/// One searchable memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: i64,
    pub scope: Scope,
    /// "semantic" or "procedural".
    pub kind: String,
    pub text: String,
}

pub trait Index: Send + Sync {
    /// Adds documents, replacing any with the same id.
    fn upsert(&self, documents: &[Document]) -> Result<()>;
    fn remove(&self, ids: &[i64]) -> Result<()>;
    /// Ids in `scopes` with scores, best first. Scores only compare within one search.
    fn search(&self, query: &str, scopes: &[Scope], limit: usize) -> Result<Vec<(i64, f64)>>;
    fn clear(&self) -> Result<()>;
}

/// SQLite FTS5 with BM25 ranking. Matches any word of the query; the porter stemmer makes
/// "naming" match "name". Purely lexical: no paraphrases, no matching across languages.
pub struct Fts {
    conn: Mutex<Connection>,
}

impl Fts {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                 id UNINDEXED, scope UNINDEXED, kind UNINDEXED, text,
                 tokenize = 'porter unicode61'
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl Index for Fts {
    fn upsert(&self, documents: &[Document]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for d in documents {
            tx.execute("DELETE FROM memory_fts WHERE id = ?1", [d.id])?;
            tx.execute(
                "INSERT INTO memory_fts (id, scope, kind, text) VALUES (?1, ?2, ?3, ?4)",
                params![d.id, d.scope.as_str(), d.kind, d.text],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn remove(&self, ids: &[i64]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM memory_fts WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn search(&self, query: &str, scopes: &[Scope], limit: usize) -> Result<Vec<(i64, f64)>> {
        let Some(terms) = fts_query(query) else {
            return Ok(Vec::new());
        };
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut select = conn.prepare_cached(&format!(
            "SELECT id, -bm25(memory_fts) AS score FROM memory_fts
             WHERE memory_fts MATCH ? AND scope IN ({})
             ORDER BY score DESC LIMIT {limit}",
            vec!["?"; scopes.len()].join(",")
        ))?;
        let args = std::iter::once(terms).chain(scopes.iter().map(|s| s.as_str().to_string()));
        let hits =
            select.query_map(params_from_iter(args), |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(hits.collect::<Result<_, _>>()?)
    }

    fn clear(&self) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM memory_fts", [])?;
        Ok(())
    }
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "please", "so", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "to", "us", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// The query's words worth matching, each quoted so FTS5 reads it as a plain term, OR'd.
pub(crate) fn fts_query(query: &str) -> Option<String> {
    let lower = query.to_lowercase();
    let mut terms: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1 && !STOPWORDS.contains(t))
        .collect();
    terms.sort_unstable();
    terms.dedup();
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_keeps_meaningful_words() {
        assert_eq!(fts_query("What's my name?").as_deref(), Some("\"name\""));
        assert_eq!(
            fts_query("deploy the deploy script").as_deref(),
            Some("\"deploy\" OR \"script\"")
        );
        assert_eq!(fts_query("what is it"), None);
    }

    #[test]
    fn scopes_filter_and_upsert_replaces() {
        let fts = Fts::in_memory().unwrap();
        let doc = |id, scope: Scope, text: &str| Document {
            id,
            scope,
            kind: "semantic".into(),
            text: text.into(),
        };
        fts.upsert(&[
            doc(1, Scope::user("a"), "likes tea"),
            doc(2, Scope::user("b"), "likes tea"),
        ])
        .unwrap();
        let ids = |q| -> Vec<i64> {
            fts.search(q, &[Scope::user("a")], 5)
                .unwrap()
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        };
        assert_eq!(ids("tea"), [1]);
        fts.upsert(&[doc(1, Scope::user("a"), "likes coffee")])
            .unwrap();
        assert!(ids("tea").is_empty());
        assert_eq!(ids("coffee"), [1]);
        fts.remove(&[1]).unwrap();
        assert!(ids("coffee").is_empty());
    }
}
