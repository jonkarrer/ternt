//! Tokidoki in Ternt: an AI agent per space, behind the Ask button and ⌘J.
//!
//! The agents run in their own process, the sidecar (`sidecar.rs`; protocol
//! in `wire.rs` and docs/AGENT-SIDECAR.md): tokidoki's `serve` binary once it
//! exists, the stub in `src/bin/ternt-agent` until then. A panic, an abort or
//! running out of memory in an agent ends that process, never Ternt; the
//! supervisor starts it again and the panel says so. This module owns:
//!
//! - **The boundary** (`Agent`): what a space's agent does, the same four
//!   methods tokidoki offers. `sidecar::Remote` implements it over the
//!   protocol; agents open lazily on first use, one per space folder
//!   (`folder_name`).
//! - **The outbox**: every agent's events, tagged with their space, go
//!   through one bounded channel to one forwarder task, which coalesces
//!   streamed text (one IPC message per ~30ms instead of one per token) and
//!   emits nothing while the panel is hidden. The page reloads a thread's
//!   history when it opens, so dropped events cost nothing; a full queue
//!   sends `Lagged`, and the page catches up the same way.
//! - **The panel**: a transparent webview over the window (like the command
//!   bar), created on first open and kept hidden afterwards.

mod keys;
pub mod protocol;
mod sidecar;
pub mod wire;

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
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
pub use sidecar::Status;

pub const AGENT_WEBVIEW: &str = "agent";
/// How long streamed text is held back to be sent as one piece.
const FLUSH: Duration = Duration::from_millis(30);
/// Events waiting for the forwarder before some are dropped (`Lagged`).
const QUEUE: usize = 2048;

/// What a method answers, once the sidecar has.
pub type Reply<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// A space's agent: the methods tokidoki's `Tokidoki` offers.
pub trait Agent: Send + Sync {
    /// Returns as soon as the input is placed; the answer arrives as events.
    fn send(&self, input: Input) -> Reply<Sent>;
    fn threads(&self) -> Reply<Vec<ThreadSummary>>;
    /// A thread's checkpoints from `since` on.
    fn history(&self, thread: String, since: usize) -> Reply<Vec<Event>>;
    /// Stop the turn under way; `false` if there was none.
    fn abort(&self, thread: String) -> Reply<bool>;
}

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
    /// Which implementation answers (`stub` until tokidoki's `serve` does),
    /// or `starting` while the process comes up.
    backend: String,
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

/// An open agent and the space name it was opened with.
type Opened = (String, Arc<dyn Agent>);

#[derive(Default)]
pub struct Host {
    /// Per space: the agent and the space name it was opened with.
    agents: Mutex<HashMap<u32, Opened>>,
    sidecar: OnceLock<Arc<sidecar::Sidecar>>,
    /// The open request, if the panel is showing (read by the page on load).
    request: Mutex<Option<Request>>,
    outbox: OnceLock<Outbox>,
    visible: AtomicBool,
}

/// The folder a space's agent keeps its data in (tokidoki allows letters,
/// digits, `-` and `_`; the id, not the name, so renames keep the agent).
pub fn folder_name(space: u32) -> String {
    format!("space-{space}")
}

/// Create the outbox and start forwarding events to the page.
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
    let data = crate::session::data_dir(app)
        .map(|dir| dir.join("agents"))
        .unwrap_or_else(|| std::env::temp_dir().join("ternt-agents"));
    let sidecar = sidecar::Sidecar::new(app.clone(), outbox, data);
    sidecar.start();
    let _ = host.sidecar.set(sidecar);
}

/// Deliver events to the panel: text deltas for one thread merge into one
/// message, sent at most `FLUSH` after the first of them arrived (a steady
/// stream must not hold them back); everything else goes through in order.
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
                if !app.state::<Host>().visible.load(Ordering::Relaxed) {
                    continue;
                }
                match event {
                    Event::Delta {
                        thread,
                        delta: Delta::Text { text },
                    } => {
                        since.get_or_insert_with(Instant::now);
                        match pending
                            .iter_mut()
                            .find(|(s, t, _)| *s == space && *t == thread)
                        {
                            Some((_, _, held)) => held.push_str(&text),
                            None => pending.push((space, thread, text)),
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

/// The space's agent, opened on first use (again if the space was renamed,
/// so the agent hears the new name).
fn agent_for(app: &AppHandle, space: u32) -> Result<Arc<dyn Agent>, String> {
    let host = app.state::<Host>();
    let info = browser::space_info(app, space).ok_or_else(|| format!("no space {space}"))?;
    let mut agents = host.agents.lock().unwrap();
    if let Some((label, agent)) = agents.get(&space)
        && *label == info.name
    {
        return Ok(agent.clone());
    }
    let sidecar = host
        .sidecar
        .get()
        .ok_or("the agent host isn't running")?
        .clone();
    let agent: Arc<dyn Agent> = Arc::new(sidecar::Remote::new(
        sidecar,
        folder_name(space),
        info.name.clone(),
    ));
    agents.insert(space, (info.name, agent.clone()));
    Ok(agent)
}

/// A space was deleted: forget its agent, and have the sidecar remove its
/// files (conversations, memory, journal). If the process is down just then
/// the folder stays behind, which only costs disk.
pub fn forget(app: &AppHandle, space: u32) {
    let host = app.state::<Host>();
    host.agents.lock().unwrap().remove(&space);
    let Some(sidecar) = host.sidecar.get().cloned() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let folder = folder_name(space);
        if let Err(err) = sidecar
            .call(Some(folder.clone()), "delete", serde_json::Value::Null)
            .await
        {
            eprintln!("ternt-agent: deleting {folder}: {err}");
        }
    });
}

/// Stop the agent process (before exit).
pub fn shutdown(app: &AppHandle) {
    let host = app.state::<Host>();
    host.agents.lock().unwrap().clear();
    if let Some(sidecar) = host.sidecar.get() {
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
    let host = app.state::<Host>();
    let folder = folder_name(space.id);
    let backend = host
        .sidecar
        .get()
        .and_then(|s| s.backend())
        .unwrap_or_else(|| "starting".into());
    let request = Request {
        space,
        folder,
        content_left: browser::content_left(app),
        backend,
    };
    *host.request.lock().unwrap() = Some(request.clone());
    host.visible.store(true, Ordering::Relaxed);
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

pub fn close(app: &AppHandle) -> tauri::Result<()> {
    let host = app.state::<Host>();
    host.visible.store(false, Ordering::Relaxed);
    if host.request.lock().unwrap().take().is_none() {
        return Ok(());
    }
    if let Some(webview) = app.get_webview(AGENT_WEBVIEW) {
        webview.hide()?;
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

/// What the user typed, to the space's agent; `thread` continues one.
#[tauri::command]
pub async fn agent_send(
    app: AppHandle,
    space: u32,
    text: String,
    thread: Option<String>,
) -> Result<Sent, String> {
    let input = Input {
        from: Speaker {
            id: "owner".into(),
            name: None,
        },
        parts: vec![Part::Text { text }],
        thread,
        request_id: None,
        when_busy: WhenBusy::Steer,
    };
    agent_for(&app, space)?.send(input).await
}

#[tauri::command]
pub async fn agent_threads(app: AppHandle, space: u32) -> Result<Vec<ThreadSummary>, String> {
    agent_for(&app, space)?.threads().await
}

#[tauri::command]
pub async fn agent_history(
    app: AppHandle,
    space: u32,
    thread: String,
    since: usize,
) -> Result<Vec<Event>, String> {
    agent_for(&app, space)?.history(thread, since).await
}

#[tauri::command]
pub async fn agent_abort(app: AppHandle, space: u32, thread: String) -> Result<bool, String> {
    agent_for(&app, space)?.abort(thread).await
}

/// Where the agent process stands (the panel asks on open; `agent:status`
/// carries changes).
#[tauri::command]
pub async fn agent_status(app: AppHandle) -> Status {
    app.state::<Host>()
        .sidecar
        .get()
        .map(|s| s.status())
        .unwrap_or(Status::Down {
            reason: "The agent host isn't running.".into(),
        })
}

/// The panel's Restart button.
#[tauri::command]
pub async fn agent_restart(app: AppHandle) -> Result<(), String> {
    app.state::<Host>()
        .sidecar
        .get()
        .ok_or("the agent host isn't running")?
        .restart();
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

/// Which API keys the user has given the agents (never their values).
#[tauri::command]
pub async fn agent_keys() -> keys::Present {
    keys::present()
}

/// Store the keys and start the agent over with them. `None` leaves a key
/// as it is; an empty string removes it.
#[tauri::command]
pub async fn agent_set_keys(
    app: AppHandle,
    anthropic: Option<String>,
    baseten: Option<String>,
) -> Result<(), String> {
    if let Some(key) = anthropic {
        keys::ANTHROPIC.write(key.trim())?;
    }
    if let Some(key) = baseten {
        keys::BASETEN.write(key.trim())?;
    }
    if let Some(sidecar) = app.state::<Host>().sidecar.get() {
        sidecar.restart();
    }
    Ok(())
}
