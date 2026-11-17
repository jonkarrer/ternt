//! Session persistence: spaces, tabs and sidebar layout survive relaunches.
//! The format lives in `model.rs` (`Saved`).
//!
//! Saves are debounced (state changes in bursts while pages load) and written
//! atomically with a backup of the previous one (`persist`); a final save
//! runs on exit.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use tauri::{AppHandle, Manager};

use crate::{browser, model::Saved, persist::Loaded};

const SAVE_DELAY: Duration = Duration::from_millis(750);

static SAVE_PENDING: AtomicBool = AtomicBool::new(false);
/// What the last save wrote.
static LAST_SAVED: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// `TERNT_SESSION_FILE` overrides the location (the e2e suite uses this to
/// stay away from the real session).
fn path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TERNT_SESSION_FILE") {
        return Some(path.into());
    }
    Some(data_dir(app)?.join("session.json"))
}

/// Where sessions and history live. Debug builds get their own folder, so
/// `bun tauri dev` never leaves its tabs in the installed app (same bundle
/// identifier, same app data dir).
pub fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    Some(if cfg!(debug_assertions) {
        dir.join("dev")
    } else {
        dir
    })
}

/// The saved session (its backup if the file is damaged, see `persist`).
pub fn load(app: &AppHandle) -> Loaded<Saved> {
    match path(app) {
        Some(path) => crate::persist::load(&path),
        None => Loaded::Missing,
    }
}

pub fn save_now(app: &AppHandle) {
    let Some(path) = path(app) else { return };
    let session = browser::session(app);
    let data = match serde_json::to_vec(&session) {
        Ok(data) => data,
        Err(err) => return eprintln!("failed to save session: {err}"),
    };
    // Most state changes (a page's title, loading) aren't saved: skip
    // rewriting the same session.
    let mut last = LAST_SAVED.lock().unwrap_or_else(|e| e.into_inner());
    if *last == data {
        return;
    }
    match crate::persist::write(&path, &data, true) {
        Ok(()) => *last = data,
        Err(err) => eprintln!("failed to save session: {err}"),
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
