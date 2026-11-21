//! Self-update, after ternt's. `scripts/publish.sh` publishes each release with a
//! `latest.json` manifest; every update is signed with the project's updater key and checked
//! against the public key in `tauri.conf.json` before it is installed.
//!
//! Tokidoki checks shortly after launch and every 15 minutes. An update it finds downloads
//! in the background and installs when Tokidoki quits; meanwhile the app says it's ready,
//! with Restart Now to use it right away. Unfinished work survives either way: the engine
//! resumes it at the next launch.
//!
//! `TOKIDOKI_UPDATE_URL` replaces the manifest URL (local testing). Signatures are still
//! checked against the built-in key.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// How often the background loop looks at the clock. Timers stop while the Mac sleeps, so it
/// compares wall-clock times rather than sleeping `CHECK_EVERY`.
const TICK: Duration = Duration::from_secs(60);

/// What the app shows.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// Nothing to show (also while an update downloads in the background).
    #[default]
    Idle,
    /// A check the user asked for found nothing.
    UpToDate,
    /// Downloaded and verified: installs on quit, or now with a restart.
    Ready { version: String },
    /// Swapping the bundle; the app relaunches next.
    Installing { version: String },
    /// A check the user asked for, or an install, failed.
    Failed { message: String },
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    /// The update the last check found, downloaded.
    pending: Mutex<Option<(Update, Vec<u8>)>>,
    /// A check is under way (the loop and a manual one can overlap).
    checking: AtomicBool,
    /// When the last check started (wall clock, which keeps counting while the Mac sleeps).
    last_check: Mutex<Option<SystemTime>>,
}

fn set_status(app: &AppHandle, status: Status) {
    *app.state::<Updates>().status.lock().unwrap() = status.clone();
    let _ = app.emit("update:status", status);
}

/// Debug builds only check when pointed at a test server, so development stays offline.
fn enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("TOKIDOKI_UPDATE_URL").is_some()
}

/// Check in the background, now and then periodically.
pub fn start(app: &AppHandle) {
    if !enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let last = *app.state::<Updates>().last_check.lock().unwrap();
            // A clock set backwards counts as due.
            let due = last.is_none_or(|at| at.elapsed().unwrap_or(Duration::MAX) >= CHECK_EVERY);
            if due && let Err(error) = check(&app, false).await {
                eprintln!("update check failed: {error}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    let mut builder = app.updater_builder();
    if let Some(url) = std::env::var("TOKIDOKI_UPDATE_URL")
        .ok()
        .and_then(|url| Url::parse(&url).ok())
    {
        builder = builder.endpoints(vec![url])?;
    }
    builder.build()
}

/// Look for an update; one found is downloaded and verified, then ready. `manual` checks
/// (the user asked) also report "up to date" and failures.
async fn check(app: &AppHandle, manual: bool) -> Result<(), String> {
    let updates = app.state::<Updates>();
    if updates.checking.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    *updates.last_check.lock().unwrap() = Some(SystemTime::now());
    let result = find(app, manual).await;
    updates.checking.store(false, Ordering::Release);
    // A failed check takes nothing away from an update that is already downloaded.
    let ready = updates.pending.lock().unwrap().is_some();
    if manual
        && !ready
        && let Err(message) = &result
    {
        set_status(
            app,
            Status::Failed {
                message: message.clone(),
            },
        );
    }
    result
}

async fn find(app: &AppHandle, manual: bool) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let found = updater(app)
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(update) = found else {
        if manual {
            set_status(app, Status::UpToDate);
        }
        return Ok(());
    };
    let version = update.version.clone();
    let ready = (updates.pending.lock().unwrap().as_ref())
        .is_some_and(|(pending, _)| pending.version == version);
    if !ready {
        let bytes = update
            .download(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        *updates.pending.lock().unwrap() = Some((update, bytes));
    }
    set_status(app, Status::Ready { version });
    Ok(())
}

/// Tokidoki is quitting: install a downloaded update, for the next launch.
pub fn install_on_exit(app: &AppHandle) {
    let pending = app.state::<Updates>().pending.lock().unwrap().take();
    if let Some((update, bytes)) = pending
        && let Err(error) = update.install(bytes)
    {
        eprintln!("installing the update on quit failed: {error}");
    }
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    app.state::<Updates>().status.lock().unwrap().clone()
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<(), String> {
    if !enabled() {
        return Err("Development builds don't update themselves.".into());
    }
    check(&app, true).await
}

/// Swap in the downloaded update and relaunch.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = app.state::<Updates>().pending.lock().unwrap().take() else {
        return Err("no update is ready".into());
    };
    let version = update.version.clone();
    set_status(&app, Status::Installing { version });
    if let Err(error) = update.install(&bytes) {
        let message = error.to_string();
        // Kept, so the button can try again.
        *app.state::<Updates>().pending.lock().unwrap() = Some((update, bytes));
        set_status(
            &app,
            Status::Failed {
                message: message.clone(),
            },
        );
        return Err(message);
    }
    // Relaunching skips the normal exit path: stop the core first.
    crate::close_core(&app).await;
    app.restart();
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
