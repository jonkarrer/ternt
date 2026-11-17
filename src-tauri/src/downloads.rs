//! Downloads: the list in the sidebar's Downloads box (src/lib/DownloadsBox.svelte),
//! like Arc's. Files go to ~/Downloads (named as the site suggests,
//! numbered rather than overwriting); each shows its progress while it
//! downloads, then opens with a click. Finished ones are kept in
//! `downloads.json` across launches.
//!
//! WebKit downloads the file; wry tells us when one starts and ends, and
//! `native` hands us the `WKDownload` alongside, for progress and Cancel.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Url, Webview};

use crate::browser::{PEEK_WEBVIEW, UI_WEBVIEW};

/// Finished downloads kept in the list.
const KEEP: usize = 50;
/// How often progress is read while something downloads.
const POLL_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Active,
    Done,
    Failed,
    Canceled,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: u32,
    pub url: String,
    pub path: PathBuf,
    pub name: String,
    pub state: State,
    pub received: u64,
    /// The size, when the server said (and once it's done).
    pub total: Option<u64>,
    /// When it started (ms since 1970).
    pub started: u64,
    /// The file is gone (moved or deleted since). Not saved: checked when sent.
    #[serde(skip_deserializing, default)]
    pub missing: bool,
}

#[derive(Default)]
pub struct Downloads {
    list: Mutex<Vec<Download>>,
    next: AtomicU32,
    polling: AtomicBool,
}

#[cfg(target_os = "macos")]
thread_local! {
    /// Each download in progress's `WKDownload`, by id (main thread only).
    static NATIVE: std::cell::RefCell<HashMap<u32, objc2::rc::Retained<objc2::runtime::AnyObject>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// `TERNT_DOWNLOADS_FILE` overrides the location (used by the e2e suite).
fn path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TERNT_DOWNLOADS_FILE") {
        return Some(path.into());
    }
    Some(crate::session::data_dir(app)?.join("downloads.json"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Load the saved list (once, at launch).
pub fn load(app: &AppHandle) {
    let list: Vec<Download> = match path(app).map(|p| crate::persist::load(&p)) {
        Some(crate::persist::Loaded::Read(list)) => list,
        _ => Vec::new(),
    };
    let state = app.state::<Downloads>();
    state.next.store(list.iter().map(|d| d.id + 1).max().unwrap_or(1), Ordering::Relaxed);
    *state.list.lock().unwrap() = list;
}

fn save(app: &AppHandle) {
    let Some(path) = path(app) else {
        return;
    };
    let finished: Vec<Download> = app
        .state::<Downloads>()
        .list
        .lock()
        .unwrap()
        .iter()
        .filter(|d| d.state != State::Active)
        .cloned()
        .collect();
    let data = serde_json::to_vec(&finished).unwrap_or_default();
    std::thread::spawn(move || {
        if let Err(err) = crate::persist::write(&path, &data, false) {
            eprintln!("can't save downloads: {err}");
        }
    });
}

/// The list as the sidebar shows it: newest first.
fn snapshot(app: &AppHandle) -> Vec<Download> {
    let mut list = app.state::<Downloads>().list.lock().unwrap().clone();
    for d in &mut list {
        d.missing = d.state == State::Done && !d.path.exists();
    }
    list.reverse();
    list
}

fn changed(app: &AppHandle) {
    let list = snapshot(app);
    for label in [UI_WEBVIEW, PEEK_WEBVIEW] {
        let _ = app.emit_to(EventTarget::webview(label), "downloads", &list);
    }
}

/// `path`, numbered ("name (1).ext") past files there and downloads on their
/// way to one: wry numbers only past files, so two downloads of one file at
/// once were both sent to the same path, and WebKit canceled one.
fn unique(path: &Path, taken: &[Download]) -> PathBuf {
    let free = |p: &Path| !p.exists() && !taken.iter().any(|d| d.state == State::Active && d.path == p);
    if free(path) {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    // wry's own numbering may already be there: count on from it.
    let stem = match stem.rsplit_once(" (") {
        Some((base, n)) if n.strip_suffix(')').is_some_and(|n| n.parse::<u32>().is_ok()) => base,
        _ => &stem,
    };
    (1..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| free(p))
        .unwrap()
}

/// A page's download is starting, to `path` (wry's download hook, which may
/// renumber it). Runs on the main thread, inside WebKit's delegate call.
pub fn started(webview: &Webview, url: &Url, path: &mut PathBuf) {
    let app = webview.app_handle();
    let state = app.state::<Downloads>();
    *path = unique(path, &state.list.lock().unwrap());
    let id = state.next.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| url.to_string());
    state.list.lock().unwrap().push(Download {
        id,
        url: url.to_string(),
        path: path.to_path_buf(),
        name,
        state: State::Active,
        received: 0,
        total: None,
        started: now_ms(),
        missing: false,
    });
    #[cfg(target_os = "macos")]
    if let Some(native) = crate::native::current_download() {
        NATIVE.with(|n| n.borrow_mut().insert(id, native.download));
        // A link opened in a new tab that was only ever a file: close the
        // empty tab (or window), as Safari and Arc do.
        if native.nothing_shown {
            close_empty(app, webview.label().to_string());
        }
    }
    changed(app);
    poll(app);
}

/// Close the page view a download left with nothing to show.
#[cfg(target_os = "macos")]
fn close_empty(app: &AppHandle, label: String) {
    let app = app.clone();
    // Closing views blocks on the main thread.
    std::thread::spawn(move || {
        if let Some(id) = label.strip_prefix("tab-").and_then(|id| id.parse().ok()) {
            if let Err(err) = crate::browser::close_tab_by_id(&app, id) {
                eprintln!("can't close the download's empty tab: {err}");
            }
        } else if label.starts_with("popup-")
            && let Some(window) = app.get_webview_window(&label)
        {
            let _ = window.close();
        }
    });
}

/// A download ended (wry's download hook, on the main thread).
pub fn finished(app: &AppHandle, url: &Url, success: bool) {
    let state = app.state::<Downloads>();
    let mut list = state.list.lock().unwrap();
    // The `WKDownload` tells two downloads of one URL apart.
    #[cfg(target_os = "macos")]
    let native_id = match crate::native::current_download() {
        Some(current) => {
            let id = NATIVE.with(|n| {
                let mut n = n.borrow_mut();
                let id = n
                    .iter()
                    .find(|(_, d)| objc2::rc::Retained::as_ptr(d) == objc2::rc::Retained::as_ptr(&current.download))
                    .map(|(id, _)| *id)?;
                n.remove(&id);
                Some(id)
            });
            // Not ours any more: canceled (WebKit may report it as failed).
            let Some(id) = id else {
                return;
            };
            Some(id)
        }
        None => None,
    };
    #[cfg(not(target_os = "macos"))]
    let native_id = None;
    let url = url.to_string();
    let Some(d) = list.iter_mut().find(|d| match native_id {
        Some(id) => d.id == id,
        None => d.state == State::Active && d.url == url,
    }) else {
        return;
    };
    // Canceled already (WebKit may report the cancel as a failure).
    if d.state != State::Active {
        let _ = std::fs::remove_file(&d.path);
        return;
    }
    if success {
        d.state = State::Done;
        if let Ok(meta) = std::fs::metadata(&d.path) {
            d.received = meta.len();
            d.total = Some(meta.len());
        }
        #[cfg(target_os = "macos")]
        crate::native::download_finished(&d.path);
    } else {
        d.state = State::Failed;
        // Don't leave half a file in Downloads.
        let _ = std::fs::remove_file(&d.path);
    }
    #[cfg(all(debug_assertions, target_os = "macos"))]
    crate::debug::record_download(&d.path, success);
    let finished = list.len() - list.iter().filter(|d| d.state == State::Active).count();
    if finished > KEEP {
        let mut extra = finished - KEEP;
        list.retain(|d| {
            let drop = extra > 0 && d.state != State::Active;
            extra -= drop as usize;
            !drop
        });
    }
    drop(list);
    changed(app);
    save(app);
}

/// Read progress every so often while anything downloads.
fn poll(app: &AppHandle) {
    let state = app.state::<Downloads>();
    if state.polling.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL_EVERY);
            let tick_app = app.clone();
            let _ = app.run_on_main_thread(move || read_progress(&tick_app));
            let state = app.state::<Downloads>();
            let active = || state.list.lock().unwrap().iter().any(|d| d.state == State::Active);
            if !active() {
                state.polling.store(false, Ordering::Release);
                // One may have started just now, and seen us still polling.
                if !active() || state.polling.swap(true, Ordering::AcqRel) {
                    return;
                }
            }
        }
    });
}

/// Main thread.
fn read_progress(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let progress: Vec<(u32, i64, i64)> = NATIVE.with(|n| {
            n.borrow()
                .iter()
                .map(|(id, d)| {
                    let (done, total) = crate::native::download_progress(d);
                    (*id, done, total)
                })
                .collect()
        });
        let mut moved = false;
        {
            let state = app.state::<Downloads>();
            let mut list = state.list.lock().unwrap();
            for (id, done, total) in progress {
                if let Some(d) = list.iter_mut().find(|d| d.id == id && d.state == State::Active) {
                    let received = done.max(0) as u64;
                    let total = (total > 0).then_some(total as u64);
                    if d.received != received || d.total != total {
                        d.received = received;
                        d.total = total;
                        moved = true;
                    }
                }
            }
        }
        if moved {
            changed(app);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

fn find(app: &AppHandle, id: u32) -> Result<Download, String> {
    app.state::<Downloads>()
        .list
        .lock()
        .unwrap()
        .iter()
        .find(|d| d.id == id)
        .cloned()
        .ok_or_else(|| "no such download".into())
}

/// Hand a file to Finder: open it, or show it in its folder (`reveal`).
fn open(path: &Path, reveal: bool) -> Result<(), String> {
    #[cfg(all(debug_assertions, target_os = "macos"))]
    if crate::debug::testing() {
        crate::debug::record_opened(path, reveal);
        return Ok(());
    }
    let mut command = std::process::Command::new("open");
    if reveal {
        command.arg("-R");
    }
    command.arg(path).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn downloads(app: AppHandle) -> Vec<Download> {
    snapshot(&app)
}

#[tauri::command]
pub async fn open_download(app: AppHandle, id: u32) -> Result<(), String> {
    let d = find(&app, id)?;
    if d.state != State::Done || !d.path.exists() {
        return Err("the file isn't there".into());
    }
    open(&d.path, false)
}

#[tauri::command]
pub async fn reveal_download(app: AppHandle, id: u32) -> Result<(), String> {
    let d = find(&app, id)?;
    if d.path.exists() {
        open(&d.path, true)
    } else {
        // Gone: show the folder it was in.
        open(d.path.parent().ok_or("no folder")?, false)
    }
}

#[tauri::command]
pub async fn cancel_download(app: AppHandle, id: u32) -> Result<(), String> {
    {
        let state = app.state::<Downloads>();
        let mut list = state.list.lock().unwrap();
        let Some(d) = list.iter_mut().find(|d| d.id == id && d.state == State::Active) else {
            return Ok(());
        };
        d.state = State::Canceled;
        let _ = std::fs::remove_file(&d.path);
    }
    #[cfg(target_os = "macos")]
    app.run_on_main_thread(move || {
        if let Some(d) = NATIVE.with(|n| n.borrow_mut().remove(&id)) {
            crate::native::cancel_download(&d);
        }
    })
    .map_err(|e| e.to_string())?;
    changed(&app);
    save(&app);
    Ok(())
}

/// Take finished downloads off the list (all of them with no `id`). The
/// files stay.
#[tauri::command]
pub async fn clear_downloads(app: AppHandle, id: Option<u32>) {
    app.state::<Downloads>()
        .list
        .lock()
        .unwrap()
        .retain(|d| d.state == State::Active || id.is_some_and(|id| d.id != id));
    changed(&app);
    save(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_in_use_is_numbered() {
        let dir = std::env::temp_dir().join(format!("ternt-downloads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.tar.gz");
        assert_eq!(unique(&path, &[]), path);
        std::fs::write(&path, b"").unwrap();
        assert_eq!(unique(&path, &[]), dir.join("a.tar (1).gz"));
        let active = Download {
            id: 1,
            url: String::new(),
            path: dir.join("a.tar (1).gz"),
            name: String::new(),
            state: State::Active,
            received: 0,
            total: None,
            started: 0,
            missing: false,
        };
        // Taken by a download on its way, not on disk yet.
        assert_eq!(unique(&path, std::slice::from_ref(&active)), dir.join("a.tar (2).gz"));
        assert_eq!(unique(&dir.join("a.tar (1).gz"), &[active]), dir.join("a.tar (2).gz"));
    }
}
