//! Self-update. Releases are built by GitHub Actions (.github/workflows/
//! release.yml) and published on GitHub Releases with a `latest.json`
//! manifest; every update is signed with the project's updater key and
//! verified against the public key in `tauri.conf.json` before install.
//!
//! Ternt checks shortly after launch and every few hours. When an update is
//! available the sidebar shows a button (bottom left); clicking it downloads,
//! swaps the app bundle in place and relaunches.
//!
//! `TERNT_UPDATE_URL` replaces the manifest URL (local testing, see
//! e2e/update.sh). Signatures are still checked against the built-in key.

use std::{sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::browser::UI_WEBVIEW;

const FIRST_CHECK: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

/// What the sidebar shows.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    /// Nothing to show.
    Idle,
    /// A manual check found nothing (shown briefly).
    UpToDate,
    Available { version: String },
    /// `progress` is 0–1, or `None` when the size is unknown.
    Downloading { version: String, progress: Option<f64> },
    /// Swapping the bundle; the app relaunches next.
    Installing { version: String },
    Failed { message: String },
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Option<Status>>,
    /// The update found by the last check, ready to install.
    pending: Mutex<Option<Update>>,
}

fn set_status(app: &AppHandle, status: Status) {
    *app.state::<Updates>().status.lock().unwrap() = Some(status.clone());
    for label in [UI_WEBVIEW, crate::settings::SETTINGS_WINDOW] {
        let _ = app.emit_to(EventTarget::webview(label), "update:status", status.clone());
    }
}

fn status(app: &AppHandle) -> Status {
    app.state::<Updates>().status.lock().unwrap().clone().unwrap_or(Status::Idle)
}

/// Check in the background now and then periodically. Debug builds only
/// check when pointed at a test server, so dev runs and tests stay offline.
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) && std::env::var_os("TERNT_UPDATE_URL").is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Err(err) = check(&app, false).await {
                eprintln!("update check failed: {err}");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    let mut builder = app.updater_builder();
    if let Some(url) = std::env::var("TERNT_UPDATE_URL").ok().and_then(|u| Url::parse(&u).ok()) {
        builder = builder.endpoints(vec![url])?;
    }
    builder.build()
}

/// Look for an update. `manual` checks report "up to date" too.
pub async fn check(app: &AppHandle, manual: bool) -> Result<(), String> {
    // Don't disturb an install in progress.
    if matches!(status(app), Status::Downloading { .. } | Status::Installing { .. }) {
        return Ok(());
    }
    let found = updater(app).map_err(|e| e.to_string())?.check().await;
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            *app.state::<Updates>().pending.lock().unwrap() = Some(update);
            set_status(app, Status::Available { version });
        }
        Ok(None) if manual => {
            set_status(app, Status::UpToDate);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(4)).await;
                if matches!(status(&app), Status::UpToDate) {
                    set_status(&app, Status::Idle);
                }
            });
        }
        Ok(None) => {}
        Err(err) if manual => set_status(app, Status::Failed { message: err.to_string() }),
        Err(err) => return Err(err.to_string()),
    }
    Ok(())
}

/// Download, verify and install the pending update, then relaunch.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let Some(update) = app.state::<Updates>().pending.lock().unwrap().take() else {
        return Err("no update to install".into());
    };
    let version = update.version.clone();
    set_status(app, Status::Downloading { version: version.clone(), progress: Some(0.0) });

    let mut received = 0u64;
    let mut last_percent = 0u64;
    let result = update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                let progress = total.map(|t| (received as f64 / t.max(1) as f64).min(1.0));
                // One event per percent is plenty.
                let percent = progress.map_or(received / 1_000_000, |p| (p * 100.0) as u64);
                if percent != last_percent {
                    last_percent = percent;
                    set_status(app, Status::Downloading { version: version.clone(), progress });
                }
            },
            || set_status(app, Status::Installing { version: version.clone() }),
        )
        .await;
    if let Err(err) = result {
        // Keep the update so the button can retry.
        *app.state::<Updates>().pending.lock().unwrap() = Some(update);
        set_status(app, Status::Failed { message: err.to_string() });
        return Err(err.to_string());
    }

    // Relaunching skips the normal exit path: save first.
    crate::session::save_now(app);
    crate::history::save_now(app);
    app.restart();
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    status(&app)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<(), String> {
    check(&app, true).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app).await
}

/// The running version (shown in the About panel and used by the update test).
#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
