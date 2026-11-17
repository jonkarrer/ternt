//! Self-update. Releases are built and published by scripts/publish.sh on
//! GitHub Releases with a `latest.json` manifest; every update is signed
//! with the project's updater key and verified against the public key in
//! `tauri.conf.json` before install.
//!
//! Ternt checks shortly after launch, every 15 minutes, and when you come
//! back to it (at most every 2 minutes). An update it finds downloads in the
//! background (no click) and installs when Ternt quits; the sidebar card says
//! it's ready, with Restart Now to use it right away. The download is also
//! kept on disk, so if Ternt never quits normally (a crash), the next launch
//! installs it and relaunches.
//!
//! A manifest with `"required": true` (`just upload patch --required`) skips
//! all that: Ternt downloads, installs and relaunches as soon as it sees it,
//! showing "Updating to …" meanwhile.
//!
//! `TERNT_UPDATE_URL` replaces the manifest URL and `TERNT_UPDATE_DIR` the
//! folder downloads wait in (local testing, see e2e/update.sh). Signatures
//! are still checked against the built-in key.

use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::browser::{PEEK_WEBVIEW, UI_WEBVIEW};

const FIRST_CHECK: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// Coming back to the app checks again when the last check is this old.
const CHECK_ON_FOCUS_AFTER: Duration = Duration::from_secs(2 * 60);
/// How often the background loop looks at the clock. Timers stop while the
/// Mac sleeps, so it compares wall-clock times rather than sleeping
/// `CHECK_EVERY` (which could run hours late after a night asleep).
const TICK: Duration = Duration::from_secs(60);

/// What the sidebar shows.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    /// Nothing to show (also while an update downloads in the background).
    Idle,
    /// A manual check found nothing (shown briefly).
    UpToDate,
    /// Downloaded and verified: installs on quit, or now with a restart.
    Ready {
        version: String,
    },
    /// `progress` is 0–1, or `None` when the size is unknown.
    Downloading {
        version: String,
        progress: Option<f64>,
    },
    /// Swapping the bundle; the app relaunches next.
    Installing {
        version: String,
    },
    Failed {
        message: String,
    },
}

/// The update the last check found, and its package once downloaded.
struct Pending {
    update: Update,
    bytes: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Option<Status>>,
    pending: Mutex<Option<Pending>>,
    /// A download is under way (shown or not).
    downloading: Mutex<bool>,
    /// When the last check started (wall clock, which keeps counting
    /// while the Mac sleeps).
    last_check: Mutex<Option<SystemTime>>,
}

fn set_status(app: &AppHandle, status: Status) {
    *app.state::<Updates>().status.lock().unwrap() = Some(status.clone());
    for label in [UI_WEBVIEW, PEEK_WEBVIEW, crate::settings::SETTINGS_WINDOW] {
        let _ = app.emit_to(EventTarget::webview(label), "update:status", status.clone());
    }
}

fn status(app: &AppHandle) -> Status {
    app.state::<Updates>()
        .status
        .lock()
        .unwrap()
        .clone()
        .unwrap_or(Status::Idle)
}

/// Debug builds only check when pointed at a test server, so dev runs and
/// tests stay offline.
fn enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("TERNT_UPDATE_URL").is_some()
}

/// Check in the background now and then periodically. A download left from
/// a run that didn't quit normally installs first.
pub fn start(app: &AppHandle) {
    if !enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if staged(&app).is_some() {
            if let Err(err) = check(&app, false).await {
                eprintln!("update check failed: {err}");
            }
            if matches!(status(&app), Status::Ready { .. })
                && let Err(err) = install(&app).await
            {
                eprintln!("installing the downloaded update failed: {err}");
            }
        } else {
            tokio::time::sleep(FIRST_CHECK).await;
        }
        loop {
            if since_last_check(&app).is_none_or(|age| age >= CHECK_EVERY)
                && let Err(err) = check(&app, false).await
            {
                eprintln!("update check failed: {err}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// The main window came forward: check if it's been a while.
pub fn on_focus(app: &AppHandle) {
    // Before the first check, `start` has it covered.
    if !enabled() || !since_last_check(app).is_some_and(|age| age >= CHECK_ON_FOCUS_AFTER) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = check(&app, false).await {
            eprintln!("update check failed: {err}");
        }
    });
}

fn since_last_check(app: &AppHandle) -> Option<Duration> {
    let last = (*app.state::<Updates>().last_check.lock().unwrap())?;
    // A clock set backwards counts as due.
    Some(last.elapsed().unwrap_or(Duration::MAX))
}

fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    let mut builder = app.updater_builder();
    if let Some(url) = std::env::var("TERNT_UPDATE_URL")
        .ok()
        .and_then(|u| Url::parse(&u).ok())
    {
        builder = builder.endpoints(vec![url])?;
    }
    builder.build()
}

/// Where a downloaded update waits to be installed.
fn update_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TERNT_UPDATE_DIR") {
        return Some(dir.into());
    }
    Some(app.path().app_cache_dir().ok()?.join("update"))
}

fn package_path(app: &AppHandle, version: &str) -> Option<PathBuf> {
    Some(update_dir(app)?.join(format!("Ternt-{version}.app.tar.gz")))
}

/// The version of the package waiting on disk, if any (one for an older or
/// the running version is removed).
fn staged(app: &AppHandle) -> Option<String> {
    let running = app.package_info().version.to_string();
    let mut found = None;
    for entry in std::fs::read_dir(update_dir(app)?).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(version) = name
            .strip_prefix("Ternt-")
            .and_then(|n| n.strip_suffix(".app.tar.gz"))
        else {
            continue;
        };
        if newer(version, &running) {
            found = Some(version.to_string());
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    found
}

/// Whether version `a` comes after `b` (numerically, part by part).
fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) > parts(b)
}

/// Look for an update. One found downloads in the background (installed on
/// quit), or right away with a restart when the manifest says it's required.
/// `manual` checks show the download and report "up to date" too.
pub async fn check(app: &AppHandle, manual: bool) -> Result<(), String> {
    // Don't disturb an install in progress.
    if matches!(status(app), Status::Installing { .. }) || *app.state::<Updates>().downloading.lock().unwrap() {
        return Ok(());
    }
    *app.state::<Updates>().last_check.lock().unwrap() = Some(SystemTime::now());
    let found = updater(app).map_err(|e| e.to_string())?.check().await;
    match found {
        Ok(Some(update)) => {
            let required = update.raw_json.get("required").and_then(|r| r.as_bool()) == Some(true);
            let ready = app
                .state::<Updates>()
                .pending
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|p| p.update.version == update.version && p.bytes.is_some());
            if !ready {
                *app.state::<Updates>().pending.lock().unwrap() = Some(Pending { update, bytes: None });
                download(app, manual || required).await?;
            }
            if required {
                install(app).await?;
            }
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
        Err(err) if manual => set_status(
            app,
            Status::Failed {
                message: err.to_string(),
            },
        ),
        Err(err) => return Err(err.to_string()),
    }
    Ok(())
}

/// Fetch and verify the pending update's package (from disk when an earlier
/// run already did), keep it there for a later launch, and say it's ready.
/// `shown` puts the progress on the card.
async fn download(app: &AppHandle, shown: bool) -> Result<(), String> {
    let Some(update) = app
        .state::<Updates>()
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.update.clone())
    else {
        return Err("no update to download".into());
    };
    let version = update.version.clone();
    let path = package_path(app, &version);
    let on_disk = path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .filter(|bytes| update.verify(bytes).is_ok());
    let bytes = match on_disk {
        Some(bytes) => bytes,
        None => {
            *app.state::<Updates>().downloading.lock().unwrap() = true;
            if shown {
                set_status(
                    app,
                    Status::Downloading {
                        version: version.clone(),
                        progress: Some(0.0),
                    },
                );
            }
            let mut received = 0u64;
            let mut last_percent = 0u64;
            let result = update
                .download(
                    |chunk, total| {
                        received += chunk as u64;
                        if !shown {
                            return;
                        }
                        let progress = total.map(|t| (received as f64 / t.max(1) as f64).min(1.0));
                        // One event per percent is plenty.
                        let percent = progress.map_or(received / 1_000_000, |p| (p * 100.0) as u64);
                        if percent != last_percent {
                            last_percent = percent;
                            set_status(
                                app,
                                Status::Downloading {
                                    version: version.clone(),
                                    progress,
                                },
                            );
                        }
                    },
                    || {},
                )
                .await;
            *app.state::<Updates>().downloading.lock().unwrap() = false;
            match result {
                Ok(bytes) => bytes,
                Err(err) => {
                    if shown {
                        set_status(
                            app,
                            Status::Failed {
                                message: err.to_string(),
                            },
                        );
                    }
                    return Err(err.to_string());
                }
            }
        }
    };
    if let Some(path) = &path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if !path.exists() && let Err(err) = std::fs::write(path, &bytes) {
            eprintln!("couldn't keep the update for the next launch: {err}");
        }
    }
    if let Some(pending) = app.state::<Updates>().pending.lock().unwrap().as_mut() {
        pending.bytes = Some(bytes);
    }
    set_status(app, Status::Ready { version });
    Ok(())
}

/// Swap in the downloaded update (downloading it first if needed), then
/// relaunch.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let ready = app
        .state::<Updates>()
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|p| p.bytes.is_some());
    if !ready {
        download(app, true).await?;
    }
    let Some(pending) = app.state::<Updates>().pending.lock().unwrap().take() else {
        return Err("no update to install".into());
    };
    let version = pending.update.version.clone();
    set_status(app, Status::Installing { version });
    if let Err(err) = apply(app, &pending) {
        // Keep the update so the button can retry.
        *app.state::<Updates>().pending.lock().unwrap() = Some(pending);
        set_status(app, Status::Failed { message: err.clone() });
        return Err(err);
    }

    // Relaunching skips the normal exit path: save first.
    crate::session::save_now(app);
    crate::history::save_now(app);
    app.restart();
}

/// Install a downloaded update into the app bundle and drop its package.
fn apply(app: &AppHandle, pending: &Pending) -> Result<(), String> {
    let bytes = pending.bytes.as_deref().ok_or("the update isn't downloaded")?;
    pending.update.install(bytes).map_err(|e| e.to_string())?;
    if let Some(path) = package_path(app, &pending.update.version) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Ternt is quitting: install a downloaded update, for the next launch.
pub fn install_on_exit(app: &AppHandle) {
    let Some(pending) = app.state::<Updates>().pending.lock().unwrap().take() else {
        return;
    };
    if pending.bytes.is_some()
        && let Err(err) = apply(app, &pending)
    {
        eprintln!("installing the update on quit failed: {err}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.16.1", "0.16.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.16.0", "0.16.0"));
        assert!(!newer("0.15.2", "0.16.0"));
    }
}
