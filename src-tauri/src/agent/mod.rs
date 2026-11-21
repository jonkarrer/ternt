//! Tokidoki in Ternt: an AI agent per space, behind the Ask button and ⌘J.
//!
//! This module is the host side of the embedding described in tokidoki's
//! `docs/EMBEDDING.md`. It owns:
//!
//! - **The boundary** (`Agent`): what a space's agent must do. `stub::Stub`
//!   implements it today; tokidoki's `Spaces::open` will, one agent per
//!   space folder (`folder_name`), opened lazily on first use.
//! - **The outbox**: every agent's events, tagged with their space, go
//!   through one bounded channel to one forwarder task, which coalesces
//!   streamed text (one IPC message per ~30ms instead of one per token) and
//!   emits nothing while the panel is hidden. The page reloads a thread's
//!   history when it opens, so dropped events cost nothing; a full queue
//!   sends `Lagged`, and the page catches up the same way.
//! - **The panel**: a transparent webview over the window (like the command
//!   bar), created on first open and kept hidden afterwards.
//!
//! Keeping a crash in an agent from taking the browser down is a build
//! setting as much as code: see "Agent" in README.md.

pub mod protocol;
mod stub;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, Manager, Webview, WebviewUrl, Window,
    webview::WebviewBuilder,
};
use tokio::sync::mpsc;

use crate::browser::{self, UI_WEBVIEW};
use protocol::*;

pub const AGENT_WEBVIEW: &str = "agent";
/// How long streamed text is held back to be sent as one piece.
const FLUSH: Duration = Duration::from_millis(30);
/// Events waiting for the forwarder before some are dropped (`Lagged`).
const QUEUE: usize = 2048;

/// A space's agent. Mirrors the methods tokidoki's `Tokidoki` offers, so the
/// embedding is a one-line swap per method.
pub trait Agent: Send + Sync {
    /// Returns at once with where the input went; the answer arrives as events.
    fn send(&self, input: Input) -> Result<Sent, String>;
    fn threads(&self) -> Vec<ThreadSummary>;
    /// A thread's checkpoints from `since` on.
    fn history(&self, thread: &str, since: usize) -> Vec<Event>;
    /// Stop the turn under way; `false` if there was none.
    fn abort(&self, thread: &str) -> bool;
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
    /// Which implementation answers (`stub` until tokidoki is embedded).
    backend: &'static str,
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
    agents: Mutex<HashMap<u32, Arc<dyn Agent>>>,
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
    let app = app.clone();
    tauri::async_runtime::spawn(forward(app, rx, outbox.lagged));
}

/// Deliver events to the panel: text deltas for one thread merge into one
/// message per `FLUSH`; everything else goes through in order.
async fn forward(app: AppHandle, mut rx: mpsc::Receiver<Tagged>, lagged: Arc<Mutex<Vec<u32>>>) {
    let mut pending: Vec<(u32, String, String)> = Vec::new();
    loop {
        match tokio::time::timeout(FLUSH, rx.recv()).await {
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
                        emit(&app, space, event);
                    }
                }
            }
            Err(_) => flush(&app, &mut pending),
        }
        let dropped: Vec<u32> = std::mem::take(&mut *lagged.lock().unwrap());
        for space in dropped {
            flush(&app, &mut pending);
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

/// The space's agent, opened on first use.
fn agent_for(app: &AppHandle, space: u32) -> Result<Arc<dyn Agent>, String> {
    let host = app.state::<Host>();
    let mut agents = host.agents.lock().unwrap();
    if let Some(agent) = agents.get(&space) {
        return Ok(agent.clone());
    }
    let info = browser::space_info(app, space).ok_or_else(|| format!("no space {space}"))?;
    let outbox = host
        .outbox
        .get()
        .ok_or("the agent host isn't running")?
        .clone();
    // tokidoki: `spaces.open(&folder_name(space), |data| Config::new(data, keys…), tools)`.
    let agent: Arc<dyn Agent> = Arc::new(stub::Stub::new(space, info.name, outbox));
    agents.insert(space, agent.clone());
    Ok(agent)
}

/// Stop every agent (before exit; tokidoki's `close_all`).
pub fn shutdown(app: &AppHandle) {
    app.state::<Host>().agents.lock().unwrap().clear();
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
    let folder = folder_name(space.id);
    let request = Request {
        space,
        folder,
        content_left: browser::content_left(app),
        backend: "stub",
    };
    let host = app.state::<Host>();
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
    agent_for(&app, space)?.send(input)
}

#[tauri::command]
pub async fn agent_threads(app: AppHandle, space: u32) -> Result<Vec<ThreadSummary>, String> {
    Ok(agent_for(&app, space)?.threads())
}

#[tauri::command]
pub async fn agent_history(
    app: AppHandle,
    space: u32,
    thread: String,
    since: usize,
) -> Result<Vec<Event>, String> {
    Ok(agent_for(&app, space)?.history(&thread, since))
}

#[tauri::command]
pub async fn agent_abort(app: AppHandle, space: u32, thread: String) -> Result<bool, String> {
    Ok(agent_for(&app, space)?.abort(&thread))
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
