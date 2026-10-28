//! Session persistence: tabs and sidebar layout survive relaunches.
//!
//! Saves are debounced (state changes in bursts while pages load) and written
//! atomically; a final save runs on exit.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::browser::{self, TabKind};

const SAVE_DELAY: Duration = Duration::from_millis(750);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    pub kind: TabKind,
    pub url: String,
    pub title: String,
    pub pinned: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs`.
    pub active: Option<usize>,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
}

static SAVE_PENDING: AtomicBool = AtomicBool::new(false);

/// `MOOVE_SESSION_FILE` overrides the location (the e2e suite uses this to
/// stay away from the real session).
fn path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MOOVE_SESSION_FILE") {
        return Some(path.into());
    }
    Some(app.path().app_data_dir().ok()?.join("session.json"))
}

pub fn load(app: &AppHandle) -> Option<Session> {
    let data = std::fs::read(path(app)?).ok()?;
    match serde_json::from_slice(&data) {
        Ok(session) => Some(session),
        Err(err) => {
            eprintln!("ignoring unreadable session: {err}");
            None
        }
    }
}

pub fn save_now(app: &AppHandle) {
    let Some(path) = path(app) else { return };
    let session = browser::session(app);
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write then rename, so a crash mid-write can't corrupt the session.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&session)?)?;
        std::fs::rename(tmp, &path)
    })();
    if let Err(err) = result {
        eprintln!("failed to save session: {err}");
    }
}

pub fn schedule_save(app: &AppHandle) {
    if SAVE_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_DELAY);
        SAVE_PENDING.store(false, Ordering::Release);
        save_now(&app);
    });
}
