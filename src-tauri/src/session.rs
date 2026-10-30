//! Session persistence: spaces, tabs and sidebar layout survive relaunches.
//! The format lives in `model.rs` (`Saved`).
//!
//! Saves are debounced (state changes in bursts while pages load) and written
//! atomically; a final save runs on exit.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use tauri::{AppHandle, Manager};

use crate::{browser, model::Saved};

const SAVE_DELAY: Duration = Duration::from_millis(750);

static SAVE_PENDING: AtomicBool = AtomicBool::new(false);

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

pub fn load(app: &AppHandle) -> Option<Saved> {
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
