//! Window layout and tab management.
//!
//! One window holds a transparent UI webview (the sidebar, full-window, at the
//! bottom) plus one native child webview per tab, stacked over the content
//! area. Rust owns the layout so page webviews never wait on the UI to resize.
//!
//! Locking rule: never hold `Browser::state` across a webview call. Webview
//! calls block on the main thread, and main-thread callbacks (navigation
//! state changes) take the lock, so holding it would deadlock.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview,
    WebviewUrl, WebviewWindowBuilder, Window,
    webview::{NewWindowFeatures, NewWindowResponse, WebviewBuilder},
    window::WindowBuilder,
};

pub const MAIN_WINDOW: &str = "main";
pub const UI_WEBVIEW: &str = "ui";

/// Gap between the content card and the window edges, in logical px.
/// Mirrored in `src/app.css`.
const INSET: f64 = 8.0;
const CONTENT_RADIUS: f64 = 10.0;
const DEFAULT_SIDEBAR_WIDTH: f64 = 240.0;
const MIN_SIDEBAR_WIDTH: f64 = 180.0;
const MAX_SIDEBAR_WIDTH: f64 = 480.0;
const MAX_CLOSED_TABS: usize = 25;

/// WKWebView's default UA omits the Safari version token, which makes some
/// sites (Google, notably) serve degraded pages.
#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    id: u32,
    url: String,
    title: String,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
}

#[derive(Clone, Serialize)]
pub struct Snapshot {
    tabs: Vec<Tab>,
    active: Option<u32>,
}

/// Navigation state read from a native webview.
pub struct NativeState {
    pub url: Option<String>,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

struct State {
    tabs: Vec<Tab>,
    active: Option<u32>,
    sidebar_width: f64,
    /// URLs of recently closed tabs, most recent last.
    closed: Vec<String>,
}

pub struct Browser {
    state: Mutex<State>,
    next_id: AtomicU32,
    emit_pending: AtomicBool,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                tabs: Vec::new(),
                active: None,
                sidebar_width: DEFAULT_SIDEBAR_WIDTH,
                closed: Vec::new(),
            }),
            next_id: AtomicU32::new(1),
            emit_pending: AtomicBool::new(false),
        }
    }
}

impl Browser {
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    fn snapshot(&self) -> Snapshot {
        self.with(|s| Snapshot {
            tabs: s.tabs.clone(),
            active: s.active,
        })
    }

    /// Returns false if the tab no longer exists.
    fn update_tab(&self, id: u32, f: impl FnOnce(&mut Tab)) -> bool {
        self.with(|s| s.tabs.iter_mut().find(|t| t.id == id).map(f).is_some())
    }
}

fn tab_label(id: u32) -> String {
    format!("tab-{id}")
}

fn main_window(app: &AppHandle) -> tauri::Result<Window> {
    app.get_window(MAIN_WINDOW).ok_or(tauri::Error::WindowNotFound)
}

fn tab_webview(app: &AppHandle, id: u32) -> Option<Webview> {
    app.get_webview(&tab_label(id))
}

fn active_webview(app: &AppHandle) -> Option<Webview> {
    let id = app.state::<Browser>().with(|s| s.active)?;
    tab_webview(app, id)
}

/// Push the full tab state to the UI. Used directly after user actions so the
/// sidebar reacts in the same frame.
fn emit_state(app: &AppHandle) {
    let snapshot = app.state::<Browser>().snapshot();
    let _ = app.emit_to(EventTarget::webview(UI_WEBVIEW), "browser:state", snapshot);
}

/// Coalesce bursts of page-driven changes (a navigation fires URL, title,
/// loading and history updates back to back) into one emit per frame.
fn schedule_emit(app: &AppHandle) {
    if app.state::<Browser>().emit_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(16));
        app.state::<Browser>().emit_pending.store(false, Ordering::Release);
        emit_state(&app);
    });
}

/// Called on the main thread whenever a tab's native navigation state changes.
pub fn on_native_change(app: &AppHandle, id: u32, native: NativeState) {
    let changed = app.state::<Browser>().update_tab(id, |t| {
        if let Some(url) = native.url {
            t.url = url;
        }
        t.title = native.title;
        t.loading = native.loading;
        t.can_go_back = native.can_go_back;
        t.can_go_forward = native.can_go_forward;
    });
    if changed {
        schedule_emit(app);
    }
}

fn content_bounds(window: &Window, sidebar_width: f64) -> tauri::Result<Rect> {
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    let left = sidebar_width.max(INSET);
    Ok(Rect {
        position: LogicalPosition::new(left, INSET).into(),
        size: LogicalSize::new(
            (size.width - left - INSET).max(0.0),
            (size.height - INSET * 2.0).max(0.0),
        )
        .into(),
    })
}

/// Apply the current content bounds to every tab (hidden ones too, so
/// switching tabs never shows a stale size).
fn layout_tabs(app: &AppHandle) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    let (ids, sidebar_width) = browser.with(|s| {
        (
            s.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
            s.sidebar_width,
        )
    });
    let bounds = content_bounds(&main_window(app)?, sidebar_width)?;
    for id in ids {
        if let Some(webview) = tab_webview(app, id) {
            webview.set_bounds(bounds)?;
        }
    }
    Ok(())
}

pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let builder = WindowBuilder::new(app, MAIN_WINDOW)
        .title("moove")
        .inner_size(1280.0, 800.0)
        .min_inner_size(640.0, 400.0);

    #[cfg(target_os = "macos")]
    let builder = {
        use tauri::window::{Effect, EffectState, EffectsBuilder};
        builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .transparent(true)
            .effects(
                EffectsBuilder::new()
                    .effect(Effect::Sidebar)
                    .state(EffectState::FollowsWindowActiveState)
                    .build(),
            )
    };

    let window = builder.build()?;
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);

    let ui = WebviewBuilder::new(UI_WEBVIEW, WebviewUrl::App("index.html".into())).transparent(true);
    #[cfg(not(target_os = "macos"))]
    let ui = ui.auto_resize();
    let _ui = window.add_child(ui, LogicalPosition::new(0.0, 0.0), size)?;

    #[cfg(target_os = "macos")]
    crate::native::track_window_size(&_ui);

    // macOS resizes page webviews natively (see `native::style_content`).
    #[cfg(not(target_os = "macos"))]
    {
        let app = app.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Resized(_) = event {
                let _ = layout_tabs(&app);
            }
        });
    }

    Ok(())
}

/// Handle `window.open()` / `target=_blank`. Scripted popups that ask for a
/// size (OAuth sign-in, payment flows) get a real window that shares the
/// opener's configuration, so `window.opener` messaging works. Everything
/// else opens as a tab.
fn on_new_window(app: &AppHandle, url: Url, features: NewWindowFeatures) -> NewWindowResponse<tauri::Wry> {
    if features.size().is_some() {
        match open_popup(app, &url, features) {
            Ok(window) => return NewWindowResponse::Create { window },
            Err(err) => eprintln!("failed to open popup for {url}: {err}"),
        }
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = open_tab(&app, url);
    });
    NewWindowResponse::Deny
}

fn open_popup(
    app: &AppHandle,
    url: &Url,
    features: NewWindowFeatures,
) -> tauri::Result<tauri::WebviewWindow> {
    static NEXT_POPUP: AtomicU32 = AtomicU32::new(1);
    let label = format!("popup-{}", NEXT_POPUP.fetch_add(1, Ordering::Relaxed));
    let popup_app = app.clone();
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External("about:blank".parse().unwrap()))
        .window_features(features)
        .title(url.host_str().unwrap_or("moove"))
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(&title);
        })
        .on_new_window(move |url, features| on_new_window(&popup_app, url, features));
    #[cfg(target_os = "macos")]
    let builder = builder.user_agent(USER_AGENT);
    builder.build()
}

/// Must not be called from the main thread (it blocks on it).
pub fn open_tab(app: &AppHandle, url: Url) -> tauri::Result<u32> {
    let browser = app.state::<Browser>();
    let id = browser.next_id.fetch_add(1, Ordering::Relaxed);
    let window = main_window(app)?;
    let bounds = content_bounds(&window, browser.with(|s| s.sidebar_width))?;

    // Register before creating the webview so its first load events find the tab.
    browser.with(|s| {
        s.tabs.push(Tab {
            id,
            url: url.to_string(),
            title: String::new(),
            loading: true,
            can_go_back: false,
            can_go_forward: false,
        })
    });

    let popup_app = app.clone();
    let builder = WebviewBuilder::new(tab_label(id), WebviewUrl::External(url))
        .zoom_hotkeys_enabled(true)
        .on_new_window(move |url, features| on_new_window(&popup_app, url, features));

    #[cfg(target_os = "macos")]
    let builder = builder.user_agent(USER_AGENT);

    // macOS reads navigation state via KVO (see `native::observe`); elsewhere
    // fall back to load/title callbacks.
    #[cfg(not(target_os = "macos"))]
    let builder = builder
        .on_page_load(move |webview, payload| {
            let loading = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
            let url = payload.url().to_string();
            let app = webview.app_handle();
            if app.state::<Browser>().update_tab(id, |t| {
                t.url = url;
                t.loading = loading;
            }) {
                schedule_emit(app);
            }
        })
        .on_document_title_changed(move |webview, title| {
            let app = webview.app_handle();
            if app.state::<Browser>().update_tab(id, |t| t.title = title) {
                schedule_emit(app);
            }
        });

    let webview = match window.add_child(builder, bounds.position, bounds.size) {
        Ok(webview) => webview,
        Err(err) => {
            browser.with(|s| s.tabs.retain(|t| t.id != id));
            return Err(err);
        }
    };

    #[cfg(target_os = "macos")]
    {
        crate::native::style_content(&webview, CONTENT_RADIUS);
        crate::native::observe(&webview, id, app.clone());
    }

    activate(app, id)?;
    Ok(id)
}

fn activate(app: &AppHandle, id: u32) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    let Some(previous) = browser.with(|s| {
        s.tabs
            .iter()
            .any(|t| t.id == id)
            .then(|| s.active.replace(id))
    }) else {
        return Ok(());
    };

    // Show the new tab before hiding the old one so there's no empty frame.
    if let Some(webview) = tab_webview(app, id) {
        webview.show()?;
        webview.set_focus()?;
    }
    if let Some(webview) = previous
        .filter(|&p| p != id)
        .and_then(|p| tab_webview(app, p))
    {
        webview.hide()?;
    }
    emit_state(app);
    Ok(())
}

fn close(app: &AppHandle, id: u32) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    let next = browser.with(|s| {
        let index = s.tabs.iter().position(|t| t.id == id)?;
        let tab = s.tabs.remove(index);
        s.closed.push(tab.url);
        if s.closed.len() > MAX_CLOSED_TABS {
            s.closed.remove(0);
        }
        if s.active != Some(id) {
            return Some(None);
        }
        s.active = None;
        // Prefer the tab that slid into this slot, else the one above it.
        let next = s.tabs.get(index).or_else(|| s.tabs.last()).map(|t| t.id);
        Some(next)
    });
    let Some(next) = next else { return Ok(()) };

    if let Some(next) = next {
        activate(app, next)?;
    }
    if let Some(webview) = tab_webview(app, id) {
        #[cfg(target_os = "macos")]
        crate::native::unobserve(&webview, id);
        webview.close()?;
    }
    if next.is_none() {
        emit_state(app);
    }
    Ok(())
}

pub fn close_active_tab(app: &AppHandle) -> tauri::Result<()> {
    match app.state::<Browser>().with(|s| s.active) {
        Some(id) => close(app, id),
        None => Ok(()),
    }
}

pub fn reopen_closed_tab(app: &AppHandle) -> tauri::Result<()> {
    let Some(url) = app.state::<Browser>().with(|s| s.closed.pop()) else {
        return Ok(());
    };
    match Url::parse(&url) {
        Ok(url) => open_tab(app, url).map(|_| ()),
        Err(_) => Ok(()),
    }
}

/// Activate the tab `delta` positions away from the active one, wrapping.
pub fn cycle_tab(app: &AppHandle, delta: isize) -> tauri::Result<()> {
    let target = app.state::<Browser>().with(|s| {
        let len = s.tabs.len() as isize;
        let current = s.tabs.iter().position(|t| Some(t.id) == s.active)? as isize;
        Some(s.tabs[(current + delta).rem_euclid(len) as usize].id)
    });
    match target {
        Some(id) => activate(app, id),
        None => Ok(()),
    }
}

/// ⌘1–⌘8 select by position; ⌘9 always selects the last tab.
pub fn select_tab_number(app: &AppHandle, number: usize) -> tauri::Result<()> {
    let target = app.state::<Browser>().with(|s| {
        let tab = if number == 9 {
            s.tabs.last()
        } else {
            s.tabs.get(number.checked_sub(1)?)
        };
        tab.map(|t| t.id)
    });
    match target {
        Some(id) => activate(app, id),
        None => Ok(()),
    }
}

pub fn reload_active(app: &AppHandle) -> tauri::Result<()> {
    match active_webview(app) {
        Some(webview) => webview.reload(),
        None => Ok(()),
    }
}

pub fn back_active(app: &AppHandle) -> tauri::Result<()> {
    let Some(webview) = active_webview(app) else { return Ok(()) };
    #[cfg(target_os = "macos")]
    crate::native::go_back(&webview);
    #[cfg(not(target_os = "macos"))]
    webview.eval("history.back()")?;
    Ok(())
}

pub fn forward_active(app: &AppHandle) -> tauri::Result<()> {
    let Some(webview) = active_webview(app) else { return Ok(()) };
    #[cfg(target_os = "macos")]
    crate::native::go_forward(&webview);
    #[cfg(not(target_os = "macos"))]
    webview.eval("history.forward()")?;
    Ok(())
}

/// Move keyboard focus to the sidebar's address field.
pub fn focus_omnibox(app: &AppHandle, new_tab: bool) -> tauri::Result<()> {
    if let Some(ui) = app.get_webview(UI_WEBVIEW) {
        ui.set_focus()?;
    }
    app.emit_to(EventTarget::webview(UI_WEBVIEW), "browser:omnibox", new_tab)
}

fn parse_url(url: &str) -> Result<Url, String> {
    Url::parse(url).map_err(|e| format!("invalid url {url:?}: {e}"))
}

// Commands are async so they run off the main thread (see `open_tab`).

#[tauri::command]
pub async fn get_state(app: AppHandle) -> Snapshot {
    app.state::<Browser>().snapshot()
}

#[tauri::command]
pub async fn new_tab(app: AppHandle, url: String) -> Result<u32, String> {
    open_tab(&app, parse_url(&url)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close(&app, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    activate(&app, id).map_err(|e| e.to_string())
}

/// Navigate the active tab, or open a new one if there is none.
#[tauri::command]
pub async fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let url = parse_url(&url)?;
    let Some(webview) = active_webview(&app) else {
        return open_tab(&app, url).map(|_| ()).map_err(|e| e.to_string());
    };
    webview.navigate(url).map_err(|e| e.to_string())?;
    webview.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn go_back(app: AppHandle) -> Result<(), String> {
    back_active(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn go_forward(app: AppHandle) -> Result<(), String> {
    forward_active(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reload(app: AppHandle) -> Result<(), String> {
    reload_active(&app).map_err(|e| e.to_string())
}

/// Called continuously while the user drags the sidebar edge. A width of 0
/// collapses the sidebar.
#[tauri::command]
pub async fn set_sidebar_width(app: AppHandle, width: f64) -> Result<f64, String> {
    let width = if width <= 0.0 {
        0.0
    } else {
        width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
    };
    app.state::<Browser>().with(|s| s.sidebar_width = width);
    layout_tabs(&app).map_err(|e| e.to_string())?;
    Ok(width)
}
