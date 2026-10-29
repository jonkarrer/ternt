//! Browsing history, per profile (a space's logins and its history go
//! together). The command bar searches it as you type.
//!
//! A visit is recorded when a page finishes loading at a new URL. Entries are
//! ranked by how well they match and by frecency (visit count, decayed by
//! age). Saved to `history.json`, debounced like the session.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const MAX_ENTRIES: usize = 5000;
const SAVE_DELAY: Duration = Duration::from_secs(2);
const DAY: f64 = 86_400.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub url: String,
    pub title: String,
    /// The space profile it was visited in (`None`: the shared one).
    #[serde(default)]
    pub profile: Option<String>,
    pub visits: u32,
    /// Unix seconds.
    pub last_visit: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub url: String,
    pub title: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct History {
    entries: Vec<Entry>,
    /// The URL last recorded per tab, so reloads and title updates of the
    /// same page aren't new visits.
    #[serde(skip)]
    last_url: HashMap<u32, String>,
}

impl History {
    /// Note that `tab` is showing `url`. Returns whether anything changed.
    pub fn record(&mut self, tab: u32, profile: Option<&str>, url: &str, title: &str, now: u64) -> bool {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return false;
        }
        let new_visit = self.last_url.get(&tab).is_none_or(|last| last != url);
        if new_visit {
            self.last_url.insert(tab, url.to_string());
        }
        let found = self.entries.iter_mut().find(|e| e.url == url && e.profile.as_deref() == profile);
        match found {
            Some(entry) => {
                let retitled = !title.is_empty() && entry.title != title;
                if retitled {
                    entry.title = title.to_string();
                }
                if new_visit {
                    entry.visits += 1;
                    entry.last_visit = now;
                }
                new_visit || retitled
            }
            None => {
                self.entries.push(Entry {
                    url: url.to_string(),
                    title: title.to_string(),
                    profile: profile.map(String::from),
                    visits: 1,
                    last_visit: now,
                });
                if self.entries.len() > MAX_ENTRIES {
                    self.prune(now);
                }
                true
            }
        }
    }

    /// Drop the least useful entries, keeping 90% of the cap.
    fn prune(&mut self, now: u64) {
        self.entries.sort_by(|a, b| frecency(b, now).total_cmp(&frecency(a, now)));
        self.entries.truncate(MAX_ENTRIES * 9 / 10);
    }

    pub fn forget_tab(&mut self, tab: u32) {
        self.last_url.remove(&tab);
    }

    /// Delete one profile's history (`None`: the shared profile's).
    pub fn clear(&mut self, profile: Option<&str>) {
        self.entries.retain(|e| e.profile.as_deref() != profile);
        self.last_url.clear();
    }

    /// Best matches for `query` among `profile`'s pages.
    pub fn search(&self, profile: Option<&str>, query: &str, now: u64, limit: usize) -> Vec<Hit> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(f64, &Entry)> = self
            .entries
            .iter()
            .filter(|e| e.profile.as_deref() == profile)
            .filter_map(|e| {
                let relevance = relevance(e, &query);
                (relevance > 0.0).then(|| (relevance * (1.0 + frecency(e, now).ln_1p()), e))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.last_visit.cmp(&a.1.last_visit)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, e)| Hit {
                url: e.url.clone(),
                title: e.title.clone(),
            })
            .collect()
    }
}

/// `https://www.github.com/x` → `github.com/x`, for matching what people type.
fn bare(url: &str) -> &str {
    let url = url.split_once("://").map_or(url, |(_, rest)| rest);
    url.strip_prefix("www.").unwrap_or(url)
}

/// How well an entry matches (0: not at all). Every word must appear in the
/// URL or title; typing the start of the address ranks highest.
fn relevance(entry: &Entry, query: &str) -> f64 {
    let url = bare(&entry.url).to_lowercase();
    let title = entry.title.to_lowercase();
    if !query.split_whitespace().all(|w| url.contains(w) || title.contains(w)) {
        return 0.0;
    }
    if url.starts_with(query) {
        4.0
    } else if title.starts_with(query) || title.split_whitespace().any(|w| w.starts_with(query)) {
        2.0
    } else {
        1.0
    }
}

/// Visits, worth less as they age (a week-old visit counts half).
fn frecency(entry: &Entry, now: u64) -> f64 {
    let days = now.saturating_sub(entry.last_visit) as f64 / DAY;
    entry.visits as f64 / (1.0 + days / 7.0)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// --- App state and persistence ----------------------------------------------

#[derive(Default)]
pub struct Store {
    history: Mutex<History>,
    save_pending: AtomicBool,
}

/// `TERNT_HISTORY_FILE` overrides the location (used by the e2e suite).
fn path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TERNT_HISTORY_FILE") {
        return Some(path.into());
    }
    Some(app.path().app_data_dir().ok()?.join("history.json"))
}

/// Read the saved history. Called at startup, before any page loads.
pub fn load(app: &AppHandle) {
    let Some(data) = path(app).and_then(|p| std::fs::read(p).ok()) else { return };
    match serde_json::from_slice::<History>(&data) {
        Ok(saved) => *app.state::<Store>().history.lock().unwrap() = saved,
        Err(err) => eprintln!("ignoring unreadable history: {err}"),
    }
}

pub fn save_now(app: &AppHandle) {
    let Some(path) = path(app) else { return };
    let data = {
        let store = app.state::<Store>();
        let history = store.history.lock().unwrap();
        serde_json::to_vec(&*history)
    };
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, data?)?;
        std::fs::rename(tmp, &path)
    })();
    if let Err(err) = result {
        eprintln!("failed to save history: {err}");
    }
}

fn schedule_save(app: &AppHandle) {
    if app.state::<Store>().save_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_DELAY);
        app.state::<Store>().save_pending.store(false, Ordering::Release);
        save_now(&app);
    });
}

pub fn record(app: &AppHandle, tab: u32, profile: Option<&str>, url: &str, title: &str) {
    let changed = app.state::<Store>().history.lock().unwrap().record(tab, profile, url, title, now());
    if changed {
        schedule_save(app);
    }
}

pub fn forget_tab(app: &AppHandle, tab: u32) {
    app.state::<Store>().history.lock().unwrap().forget_tab(tab);
}

pub fn clear(app: &AppHandle, profile: Option<&str>) {
    app.state::<Store>().history.lock().unwrap().clear(profile);
    schedule_save(app);
}

#[tauri::command]
pub async fn history_search(app: AppHandle, query: String, limit: usize) -> Vec<Hit> {
    let profile = crate::browser::active_profile(&app);
    let store = app.state::<Store>();
    let history = store.history.lock().unwrap();
    history.search(profile.as_deref(), &query, now(), limit.min(50))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn visit(h: &mut History, tab: u32, url: &str, title: &str, days_ago: u64) -> bool {
        h.record(tab, None, url, title, NOW - days_ago * 86_400)
    }

    fn urls(hits: Vec<Hit>) -> Vec<String> {
        hits.into_iter().map(|h| h.url).collect()
    }

    #[test]
    fn records_web_pages_only() {
        let mut h = History::default();
        assert!(!visit(&mut h, 1, "about:blank", "", 0));
        assert!(!visit(&mut h, 1, "file:///tmp/x", "", 0));
        assert!(visit(&mut h, 1, "https://a.com/", "A", 0));
        assert_eq!(h.entries.len(), 1);
    }

    #[test]
    fn same_page_in_same_tab_is_one_visit() {
        let mut h = History::default();
        visit(&mut h, 1, "https://a.com/", "", 0);
        assert!(visit(&mut h, 1, "https://a.com/", "A title", 0), "late title is an update");
        assert!(!visit(&mut h, 1, "https://a.com/", "A title", 0), "nothing new");
        assert_eq!((h.entries[0].visits, h.entries[0].title.as_str()), (1, "A title"));
        visit(&mut h, 1, "https://b.com/", "", 0);
        visit(&mut h, 1, "https://a.com/", "", 0);
        visit(&mut h, 2, "https://a.com/", "", 0);
        assert_eq!(h.entries[0].visits, 3, "returning, or another tab, is a new visit");
        assert_eq!(h.entries[0].title, "A title", "an empty title doesn't erase one");
    }

    #[test]
    fn address_prefix_beats_title_beats_anywhere() {
        let mut h = History::default();
        visit(&mut h, 1, "https://example.com/github-notes", "Notes", 0);
        visit(&mut h, 2, "https://news.site/", "GitHub trending", 0);
        visit(&mut h, 3, "https://www.github.com/", "GitHub", 0);
        assert_eq!(
            urls(h.search(None, "github", NOW, 10)),
            ["https://www.github.com/", "https://news.site/", "https://example.com/github-notes"]
        );
        assert!(h.search(None, "", NOW, 10).is_empty());
        assert!(h.search(None, "nothing", NOW, 10).is_empty());
    }

    #[test]
    fn all_words_must_match() {
        let mut h = History::default();
        visit(&mut h, 1, "https://docs.rs/tauri", "tauri - Rust", 0);
        visit(&mut h, 2, "https://docs.rs/serde", "serde - Rust", 0);
        assert_eq!(urls(h.search(None, "rust tauri", NOW, 10)), ["https://docs.rs/tauri"]);
    }

    #[test]
    fn frequent_and_recent_rank_higher() {
        let mut h = History::default();
        visit(&mut h, 1, "https://a.com/old", "page", 60);
        for tab in 2..6 {
            visit(&mut h, tab, "https://a.com/often", "page", 1);
        }
        visit(&mut h, 9, "https://a.com/once", "page", 0);
        assert_eq!(
            urls(h.search(None, "page", NOW, 10)),
            ["https://a.com/often", "https://a.com/once", "https://a.com/old"]
        );
    }

    #[test]
    fn profiles_are_separate() {
        let mut h = History::default();
        h.record(1, Some("work"), "https://mail.google.com/", "Work mail", NOW);
        h.record(2, None, "https://mail.google.com/", "Personal mail", NOW);
        assert_eq!(h.search(Some("work"), "mail", NOW, 10)[0].title, "Work mail");
        assert_eq!(h.search(None, "mail", NOW, 10)[0].title, "Personal mail");
        h.clear(Some("work"));
        assert!(h.search(Some("work"), "mail", NOW, 10).is_empty());
        assert_eq!(h.search(None, "mail", NOW, 10).len(), 1);
    }

    #[test]
    fn prunes_to_the_most_useful() {
        let mut h = History::default();
        h.record(0, None, "https://keep.com/", "", NOW);
        for _ in 0..5 {
            h.record(1, None, "https://keep.com/", "", NOW);
            h.record(1, None, "https://other.com/", "", NOW);
        }
        for i in 0..MAX_ENTRIES {
            h.record(2, None, &format!("https://x.com/{i}"), "", NOW - 30 * 86_400);
        }
        assert!(h.entries.len() <= MAX_ENTRIES);
        assert!(h.entries.iter().any(|e| e.url == "https://keep.com/"));
    }

    #[test]
    fn round_trips_through_json() {
        let mut h = History::default();
        h.record(1, Some("p"), "https://a.com/", "A", NOW);
        let back: History = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back.search(Some("p"), "a", NOW, 1)[0].title, "A");
    }
}
