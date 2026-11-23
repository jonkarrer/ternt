//! Tokidoki in Ternt: an AI agent per space, behind the Ask button and ⌘J.
//!
//! The agents run in their own process, the sidecar (`sidecar.rs`; protocol
//! in `wire.rs` and docs/AGENT-SIDECAR.md): tokidoki-serve, or the stub from
//! `src-tauri/agent` in development. A panic, an abort or running out of
//! memory in an agent ends that process, never Ternt; the supervisor starts
//! it again and the panel says so. This module owns:
//!
//! - **The requests**: each command names a space, and the sidecar opens that
//!   space's agent on first use, in its own folder (`folder_name`). The
//!   space's name goes along with every message, so a rename reaches the
//!   agent by itself.
//! - **The outbox**: every agent's events, tagged with their space, go
//!   through one bounded channel to one forwarder task, which coalesces
//!   streamed text (one IPC message per ~30ms instead of one per token) and
//!   emits nothing while the panel is hidden. The page reloads a thread's
//!   history when it opens, so dropped events cost nothing; a full queue
//!   sends `Lagged`, and the page catches up the same way.
//! - **The panel**: a transparent webview over the window (like the command
//!   bar), created on first open and kept hidden afterwards.
//! - **The keys** (`keys.rs`): the user's, in the Keychain, handed to the
//!   process through its environment when it starts.

mod keys;
pub mod protocol;
mod setup;
mod sidecar;
pub mod wire;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, Manager, Webview, WebviewUrl, Window,
    webview::WebviewBuilder,
};
use tokio::sync::mpsc;

use crate::browser::{self, UI_WEBVIEW};
use protocol::*;
pub use setup::{Setup, SpaceStatus};
pub use sidecar::Status;

pub const AGENT_WEBVIEW: &str = "agent";
/// How long streamed text is held back to be sent as one piece.
const FLUSH: Duration = Duration::from_millis(30);
/// Events waiting for the forwarder before some are dropped (`Lagged`).
const QUEUE: usize = 2048;

/// A space as the panel shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceInfo {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    space: SpaceInfo,
    /// Where the content pane starts, so the panel centers over the page.
    content_left: f64,
    /// The agent's data folder under the app data dir (`agents/<folder>`).
    folder: String,
}

/// An event on its way to the page, with the space it belongs to.
#[derive(Clone, Serialize)]
pub struct Tagged {
    pub space: u32,
    pub event: Event,
}

/// Where agents put their events.
#[derive(Clone)]
pub struct Outbox {
    tx: mpsc::Sender<Tagged>,
    /// Spaces whose events were dropped because the queue was full.
    lagged: Arc<Mutex<Vec<u32>>>,
}

impl Outbox {
    pub fn push(&self, space: u32, event: Event) {
        if self.tx.try_send(Tagged { space, event }).is_err() {
            let mut lagged = self.lagged.lock().unwrap();
            if !lagged.contains(&space) {
                lagged.push(space);
            }
        }
    }
}

#[derive(Default)]
pub struct Host {
    sidecar: OnceLock<Arc<sidecar::Sidecar>>,
    /// The open request while the panel is showing (read by the page on
    /// load, and by the forwarder to know whether anyone is looking).
    request: Mutex<Option<Request>>,
    outbox: OnceLock<Outbox>,
}

/// The folder a space's agent keeps its data in (tokidoki allows letters,
/// digits, `-` and `_`; the id, not the name, so renames keep the agent).
pub fn folder_name(space: u32) -> String {
    format!("space-{space}")
}

fn sidecar(app: &AppHandle) -> Result<Arc<sidecar::Sidecar>, String> {
    app.state::<Host>()
        .sidecar
        .get()
        .cloned()
        .ok_or_else(|| "the agent host isn't running".into())
}

/// Create the outbox, start forwarding events to the page, start the process.
pub fn start(app: &AppHandle) {
    let (tx, rx) = mpsc::channel(QUEUE);
    let outbox = Outbox {
        tx,
        lagged: Arc::default(),
    };
    let host = app.state::<Host>();
    if host.outbox.set(outbox.clone()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(forward(app.clone(), rx, outbox.lagged.clone()));
    // The e2e runner gives each suite its own folder: tokidoki locks the folder it
    // serves, so a test and a `just dev` on the same one would fight over it.
    let data = std::env::var_os("TERNT_AGENTS_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::session::data_dir(app).map(|dir| dir.join("agents")))
        .unwrap_or_else(|| std::env::temp_dir().join("ternt-agents"));
    let sidecar = sidecar::Sidecar::new(app.clone(), outbox, data);
    sidecar.start();
    let _ = host.sidecar.set(sidecar);
}

/// Deliver events to the panel: text deltas for one thread merge into one
/// message, sent at most `FLUSH` after the first of them arrived (a steady
/// stream must not hold them back); everything else goes through in order.
/// Nothing is sent, or kept, while the panel is hidden.
async fn forward(app: AppHandle, mut rx: mpsc::Receiver<Tagged>, lagged: Arc<Mutex<Vec<u32>>>) {
    let mut pending: Vec<(u32, String, String)> = Vec::new();
    /// With nothing pending, how long to wait before looking at `lagged`.
    const IDLE: Duration = Duration::from_secs(1);
    let mut since: Option<Instant> = None;
    loop {
        let wait = match since {
            Some(first) => FLUSH.saturating_sub(first.elapsed()),
            None => IDLE,
        };
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(None) => return,
            Ok(Some(Tagged { space, event })) => {
                if !showing(&app) {
                    pending.clear();
                    since = None;
                    continue;
                }
                match event {
                    Event::Delta {
                        thread,
                        delta: Delta::Text { text },
                    } => {
                        let first = *since.get_or_insert_with(Instant::now);
                        match pending
                            .iter_mut()
                            .find(|(s, t, _)| *s == space && *t == thread)
                        {
                            Some((_, _, held)) => held.push_str(&text),
                            None => pending.push((space, thread, text)),
                        }
                        // A stream that never pauses must not hold its text back.
                        if first.elapsed() >= FLUSH {
                            flush(&app, &mut pending);
                            since = None;
                        }
                    }
                    event => {
                        flush(&app, &mut pending);
                        since = None;
                        emit(&app, space, event);
                    }
                }
            }
            Err(_) => {
                flush(&app, &mut pending);
                since = None;
            }
        }
        let dropped: Vec<u32> = std::mem::take(&mut *lagged.lock().unwrap());
        for space in dropped {
            flush(&app, &mut pending);
            since = None;
            emit(&app, space, Event::Lagged);
        }
    }
}

fn flush(app: &AppHandle, pending: &mut Vec<(u32, String, String)>) {
    for (space, thread, text) in pending.drain(..) {
        emit(
            app,
            space,
            Event::Delta {
                thread,
                delta: Delta::Text { text },
            },
        );
    }
}

fn emit(app: &AppHandle, space: u32, event: Event) {
    let _ = app.emit_to(
        EventTarget::webview(AGENT_WEBVIEW),
        "agent:event",
        Tagged { space, event },
    );
}

/// A space was deleted: have the sidecar remove its agent's files
/// (conversations, memory, journal). If the process is down just then the
/// folder stays behind, which only costs disk.
pub fn forget(app: &AppHandle, space: u32) {
    let Ok(sidecar) = sidecar(app) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        if let Err(err) = sidecar.delete(space).await {
            eprintln!("ternt-agent: deleting {}: {err}", folder_name(space));
        }
    });
}

/// Start the agent process over (a key changed in Settings).
pub fn restart(app: &AppHandle) {
    if let Ok(sidecar) = sidecar(app) {
        sidecar.restart();
    }
}

/// Stop the agent process (before exit).
pub fn shutdown(app: &AppHandle) {
    if let Ok(sidecar) = sidecar(app) {
        sidecar.shutdown();
    }
}

fn create(window: &Window) -> tauri::Result<Webview> {
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    let builder = WebviewBuilder::new(AGENT_WEBVIEW, WebviewUrl::App("agent.html".into()))
        .transparent(true)
        .focused(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.auto_resize();
    let webview = window.add_child(builder, LogicalPosition::new(0.0, 0.0), size)?;
    #[cfg(target_os = "macos")]
    crate::native::track_window_size(&webview);
    Ok(webview)
}

pub fn showing(app: &AppHandle) -> bool {
    app.state::<Host>().request.lock().unwrap().is_some()
}

/// Show the panel for the active space.
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    // One overlay at a time.
    crate::palette::close(app)?;
    let Some(space) = browser::active_space_info(app) else {
        return Ok(());
    };
    let request = Request {
        folder: folder_name(space.id),
        space,
        content_left: browser::content_left(app),
    };
    *app.state::<Host>().request.lock().unwrap() = Some(request.clone());
    let webview = match app.get_webview(AGENT_WEBVIEW) {
        Some(webview) => webview,
        None => create(&browser::main_window(app)?)?,
    };
    // A page still loading misses this and asks with `agent_request`.
    app.emit_to(EventTarget::webview(AGENT_WEBVIEW), "agent:open", request)?;
    #[cfg(target_os = "macos")]
    crate::native::bring_to_front(&webview);
    webview.show()?;
    webview.set_focus()
}

/// Hide the panel, however it was asked (Esc, ⌘J, the command bar opening):
/// the page hears `agent:close` and forgets its request the same way each time.
pub fn close(app: &AppHandle) -> tauri::Result<()> {
    if app.state::<Host>().request.lock().unwrap().take().is_none() {
        return Ok(());
    }
    if let Some(webview) = app.get_webview(AGENT_WEBVIEW) {
        webview.hide()?;
        app.emit_to(EventTarget::webview(AGENT_WEBVIEW), "agent:close", ())?;
    }
    match browser::active_view(app).or_else(|| app.get_webview(UI_WEBVIEW)) {
        Some(webview) => webview.set_focus(),
        None => Ok(()),
    }
}

/// ⌘J: show the panel, or hide it if it's up.
pub fn toggle(app: &AppHandle) -> tauri::Result<()> {
    if showing(app) { close(app) } else { open(app) }
}

#[tauri::command]
pub async fn agent_request(app: AppHandle) -> Option<Request> {
    app.state::<Host>().request.lock().unwrap().clone()
}

#[tauri::command]
pub async fn open_agent(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_agent(app: AppHandle) -> Result<(), String> {
    close(&app).map_err(|e| e.to_string())
}

/// What the user typed, to the space's agent; `thread` continues one. The
/// page holds input while a turn is under way, so `when_busy` is the default.
#[tauri::command]
pub async fn agent_send(
    app: AppHandle,
    space: u32,
    text: String,
    thread: Option<String>,
) -> Result<Sent, String> {
    let label = browser::space_info(&app, space)
        .ok_or_else(|| format!("no space {space}"))?
        .name;
    let mut input = Input::text(Speaker::owner(), text);
    input.thread = thread;
    sidecar(&app)?.send(space, &label, input).await
}

#[tauri::command]
pub async fn agent_threads(app: AppHandle, space: u32) -> Result<Vec<ThreadSummary>, String> {
    sidecar(&app)?.threads(space).await
}

#[tauri::command]
pub async fn agent_history(
    app: AppHandle,
    space: u32,
    thread: String,
    since: usize,
) -> Result<Vec<Event>, String> {
    sidecar(&app)?.history(space, thread, since).await
}

#[tauri::command]
pub async fn agent_abort(app: AppHandle, space: u32, thread: String) -> Result<bool, String> {
    sidecar(&app)?.abort(space, thread).await
}

/// How a space's agent runs now (the panel and Settings ask).
#[tauri::command]
pub async fn agent_space_status(app: AppHandle, space: u32) -> Result<SpaceStatus, String> {
    let setup = browser::space_setup(&app, space).ok_or_else(|| format!("no space {space}"))?;
    sidecar(&app)?.configure(space, setup).await
}

/// Settings → Agent: how a space's agent should run from now on. Kept with
/// the space, and the agent runs so at once.
#[tauri::command]
pub async fn agent_set_space(
    app: AppHandle,
    space: u32,
    setup: Setup,
) -> Result<SpaceStatus, String> {
    if browser::space_setup(&app, space).is_none() {
        return Err(format!("no space {space}"));
    }
    browser::set_space_agent(&app, space, setup.clone());
    sidecar(&app)?.configure(space, setup).await
}

/// What a space's agent remembers (Settings → Agent → Memory).
#[tauri::command]
pub async fn agent_memories(app: AppHandle, space: u32) -> Result<Vec<Memory>, String> {
    sidecar(&app)?.memories(space).await
}

/// The user edited a memory: the one that replaces it comes back.
#[tauri::command]
pub async fn agent_revise(
    app: AppHandle,
    space: u32,
    id: i64,
    text: String,
) -> Result<Memory, String> {
    sidecar(&app)?.revise(space, id, text).await
}

#[tauri::command]
pub async fn agent_forget(app: AppHandle, space: u32, id: i64) -> Result<(), String> {
    sidecar(&app)?.forget(space, id).await
}

/// Everything the space's agent has learned, gone; its conversations stay.
#[tauri::command]
pub async fn agent_wipe(app: AppHandle, space: u32) -> Result<(), String> {
    sidecar(&app)?.wipe(space).await
}

/// Where the agent process stands (the panel asks on open; `agent:status`
/// carries changes).
#[tauri::command]
pub async fn agent_status(app: AppHandle) -> Status {
    match sidecar(&app) {
        Ok(sidecar) => sidecar.status(),
        Err(reason) => Status::Down { reason },
    }
}

/// The panel's Restart button.
#[tauri::command]
pub async fn agent_restart(app: AppHandle) -> Result<(), String> {
    sidecar(&app)?.restart();
    Ok(())
}

/// Which API keys the agents have and where from (never their values).
#[tauri::command]
pub async fn agent_keys() -> keys::Present {
    keys::present()
}

/// Store the keys and start the agent over with them. `None` leaves a key
/// as it is; an empty string removes it.
#[tauri::command]
pub async fn agent_set_keys(
    app: AppHandle,
    fireworks: Option<String>,
    anthropic: Option<String>,
    exa: Option<String>,
) -> Result<(), String> {
    if let Some(key) = fireworks {
        keys::FIREWORKS.write(key.trim())?;
    }
    if let Some(key) = anthropic {
        keys::ANTHROPIC.write(key.trim())?;
    }
    if let Some(key) = exa {
        keys::EXA.write(key.trim())?;
    }
    restart(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_what_tokidoki_accepts() {
        let name = folder_name(7);
        assert_eq!(name, "space-7");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn a_full_outbox_marks_the_space_lagged_instead_of_blocking() {
        let (tx, mut rx) = mpsc::channel(1);
        let outbox = Outbox {
            tx,
            lagged: Arc::default(),
        };
        outbox.push(1, Event::Lagged);
        outbox.push(1, Event::Lagged);
        outbox.push(2, Event::Lagged);
        outbox.push(1, Event::Lagged);
        assert_eq!(*outbox.lagged.lock().unwrap(), [1, 2]);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }
}
