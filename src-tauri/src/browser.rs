//! Window layout and tab management.
//!
//! One window holds a transparent UI webview (the sidebar, full-window, at the
//! bottom) plus native child webviews stacked over the content area: one per
//! loaded web tab, and one shared webview that renders every terminal tab.
//! Rust owns the layout so content never waits on the UI to resize.
//!
//! Web tabs load lazily: a tab's webview is created the first time it's
//! shown, so restoring a large session costs about as much as one tab, and
//! unloading (closing a pinned tab) frees the page but keeps the tab.
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

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview,
    WebviewUrl, WebviewWindowBuilder, Window,
    webview::{NewWindowFeatures, NewWindowResponse, WebviewBuilder},
    window::WindowBuilder,
};

use crate::session::{self, SavedTab, Session};

pub const MAIN_WINDOW: &str = "main";
pub const UI_WEBVIEW: &str = "ui";
pub const TERM_WEBVIEW: &str = "term";

/// Gap between the content card and the window edges, in logical px.
/// Mirrored in `src/theme.css`.
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

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabKind {
    Web,
    Terminal,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    id: u32,
    kind: TabKind,
    url: String,
    title: String,
    pinned: bool,
    /// Web tabs: whether a webview currently exists for this tab.
    loaded: bool,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
}

impl Tab {
    fn new(id: u32, kind: TabKind, url: String, title: String) -> Self {
        Self {
            id,
            kind,
            url,
            title,
            pinned: false,
            loaded: false,
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tabs: Vec<Tab>,
    active: Option<u32>,
    sidebar_width: f64,
    sidebar_collapsed: bool,
}

/// Navigation state read from a native webview.
pub struct NativeState {
    pub url: Option<String>,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

enum ClosedTab {
    Web(String),
    Terminal,
}

struct State {
    /// Pinned tabs first, then the rest (newest at the top, like Arc).
    tabs: Vec<Tab>,
    active: Option<u32>,
    sidebar_width: f64,
    sidebar_collapsed: bool,
    /// Recently closed tabs, most recent last.
    closed: Vec<ClosedTab>,
}

impl State {
    fn tab(&self, id: u32) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn pinned_count(&self) -> usize {
        self.tabs.iter().take_while(|t| t.pinned).count()
    }

    fn effective_sidebar_width(&self) -> f64 {
        if self.sidebar_collapsed { 0.0 } else { self.sidebar_width }
    }

    /// New tabs go at the top of the unpinned list.
    fn insert_new(&mut self, tab: Tab) {
        let at = self.pinned_count();
        self.tabs.insert(at, tab);
    }
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
                sidebar_collapsed: false,
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
            sidebar_width: s.sidebar_width,
            sidebar_collapsed: s.sidebar_collapsed,
        })
    }

    /// Returns false if the tab no longer exists.
    fn update_tab(&self, id: u32, f: impl FnOnce(&mut Tab)) -> bool {
        self.with(|s| s.tab_mut(id).map(f).is_some())
    }

    fn kind(&self, id: u32) -> Option<TabKind> {
        self.with(|s| s.tab(id).map(|t| t.kind))
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

fn tab_label(id: u32) -> String {
    format!("tab-{id}")
}

fn main_window(app: &AppHandle) -> tauri::Result<Window> {
    app.get_window(MAIN_WINDOW).ok_or(tauri::Error::WindowNotFound)
}

/// The webview that displays a tab: its own for web tabs, the shared
/// terminal webview for terminals.
fn view_for(app: &AppHandle, id: u32) -> Option<Webview> {
    match app.state::<Browser>().kind(id)? {
        TabKind::Web => app.get_webview(&tab_label(id)),
        TabKind::Terminal => app.get_webview(TERM_WEBVIEW),
    }
}

/// The active tab's view, of either kind (for focus).
pub fn active_view(app: &AppHandle) -> Option<Webview> {
    let id = app.state::<Browser>().with(|s| s.active)?;
    view_for(app, id)
}

/// The active tab's webview if it's a loaded web page (for navigation).
pub fn active_webview(app: &AppHandle) -> Option<Webview> {
    let id = app.state::<Browser>().with(|s| s.active)?;
    app.get_webview(&tab_label(id))
}

/// The active web tab's URL; `None` for terminals or no tab.
pub fn active_tab_url(app: &AppHandle) -> Option<String> {
    app.state::<Browser>().with(|s| {
        let tab = s.tab(s.active?)?;
        (tab.kind == TabKind::Web).then(|| tab.url.clone())
    })
}

pub fn is_terminal_tab(app: &AppHandle, id: u32) -> bool {
    app.state::<Browser>().kind(id) == Some(TabKind::Terminal)
}

/// `(kind, pinned, url)` for the tab context menu.
pub fn tab_info(app: &AppHandle, id: u32) -> Option<(TabKind, bool, String)> {
    app.state::<Browser>().with(|s| s.tab(id).map(|t| (t.kind, t.pinned, t.url.clone())))
}

/// Push the full tab state to the app webviews. Used directly after user
/// actions so the sidebar reacts in the same frame.
fn emit_state(app: &AppHandle) {
    let snapshot = app.state::<Browser>().snapshot();
    for label in [UI_WEBVIEW, TERM_WEBVIEW] {
        let _ = app.emit_to(EventTarget::webview(label), "browser:state", snapshot.clone());
    }
    session::schedule_save(app);
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

/// Left edge of the content pane, in logical px (overlays center on it).
pub fn content_left(app: &AppHandle) -> f64 {
    app.state::<Browser>().with(|s| s.effective_sidebar_width()).max(INSET)
}

fn content_bounds(app: &AppHandle) -> tauri::Result<Rect> {
    let window = main_window(app)?;
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    let left = content_left(app);
    Ok(Rect {
        position: LogicalPosition::new(left, INSET).into(),
        size: LogicalSize::new(
            (size.width - left - INSET).max(0.0),
            (size.height - INSET * 2.0).max(0.0),
        )
        .into(),
    })
}

/// Apply the current content bounds to every content view (hidden ones too,
/// so switching tabs never shows a stale size).
fn layout_tabs(app: &AppHandle) -> tauri::Result<()> {
    let ids = app
        .state::<Browser>()
        .with(|s| s.tabs.iter().map(|t| t.id).collect::<Vec<_>>());
    let bounds = content_bounds(app)?;
    let views = ids
        .into_iter()
        .filter_map(|id| app.get_webview(&tab_label(id)))
        .chain(app.get_webview(TERM_WEBVIEW));
    for webview in views {
        webview.set_bounds(bounds)?;
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

    crate::palette::create(&window)?;

    // macOS resizes content webviews natively (see `native::style_content`).
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

/// Create the webview for a web tab at its current URL. Must not be called
/// from the main thread (it blocks on it).
fn load_tab(app: &AppHandle, id: u32) -> tauri::Result<Webview> {
    let browser = app.state::<Browser>();
    let url = browser.with(|s| s.tab(id).map(|t| t.url.clone())).unwrap_or_default();
    let url = Url::parse(&url).unwrap_or_else(|_| "about:blank".parse().unwrap());

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

    let bounds = content_bounds(app)?;
    let webview = main_window(app)?.add_child(builder, bounds.position, bounds.size)?;
    browser.update_tab(id, |t| {
        t.loaded = true;
        t.loading = true;
    });

    #[cfg(target_os = "macos")]
    {
        crate::native::style_content(&webview, CONTENT_RADIUS, true);
        crate::native::observe(&webview, id, app.clone());
    }
    Ok(webview)
}

/// Destroy a web tab's webview but keep the tab (it reloads when shown).
fn unload_tab(app: &AppHandle, id: u32) -> tauri::Result<()> {
    if let Some(webview) = app.get_webview(&tab_label(id)) {
        #[cfg(target_os = "macos")]
        crate::native::unobserve(&webview, id);
        webview.close()?;
    }
    app.state::<Browser>().update_tab(id, |t| {
        t.loaded = false;
        t.loading = false;
    });
    Ok(())
}

/// Open a web tab at the top of the list and show it. Must not be called
/// from the main thread (it blocks on it).
pub fn open_tab(app: &AppHandle, url: Url) -> tauri::Result<u32> {
    let browser = app.state::<Browser>();
    let id = browser.next_id();
    browser.with(|s| s.insert_new(Tab::new(id, TabKind::Web, url.to_string(), String::new())));
    if let Err(err) = activate(app, id) {
        browser.with(|s| s.tabs.retain(|t| t.id != id));
        emit_state(app);
        return Err(err);
    }
    Ok(id)
}

/// Must not be called from the main thread (it blocks on it).
pub fn open_terminal(app: &AppHandle) -> tauri::Result<u32> {
    let browser = app.state::<Browser>();
    let id = browser.next_id();
    browser.with(|s| s.insert_new(Tab::new(id, TabKind::Terminal, String::new(), "Terminal".into())));
    // The terminal webview starts the shell when it first shows the tab.
    activate(app, id)?;
    Ok(id)
}

/// The terminal webview is created on first use: no cost until then.
fn ensure_term_webview(app: &AppHandle) -> tauri::Result<Webview> {
    if let Some(webview) = app.get_webview(TERM_WEBVIEW) {
        return Ok(webview);
    }
    let bounds = content_bounds(app)?;
    let builder = WebviewBuilder::new(TERM_WEBVIEW, WebviewUrl::App("term.html".into())).focused(false);
    let webview = main_window(app)?.add_child(builder, bounds.position, bounds.size)?;
    #[cfg(target_os = "macos")]
    crate::native::style_content(&webview, CONTENT_RADIUS, false);
    Ok(webview)
}

fn activate(app: &AppHandle, id: u32) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    let Some((kind, loaded)) = browser.with(|s| s.tab(id).map(|t| (t.kind, t.loaded))) else {
        return Ok(());
    };

    // Lazily create the view before switching to it.
    let next_view = match kind {
        TabKind::Web if !loaded => load_tab(app, id)?,
        TabKind::Web => app.get_webview(&tab_label(id)).map_or_else(|| load_tab(app, id), Ok)?,
        TabKind::Terminal => ensure_term_webview(app)?,
    };
    let previous = browser.with(|s| s.active.replace(id));

    // Show the new view before hiding the old one so there's no empty frame.
    // Terminals share a view, so terminal-to-terminal switches hide nothing.
    next_view.show()?;
    next_view.set_focus()?;
    if let Some(webview) = previous
        .filter(|&p| p != id)
        .and_then(|p| view_for(app, p))
        .filter(|prev| prev.label() != next_view.label())
    {
        webview.hide()?;
    }
    emit_state(app);
    Ok(())
}

pub fn close_tab_by_id(app: &AppHandle, id: u32) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    // Closing a pinned tab unloads it instead (Arc behavior): it stays in
    // the sidebar and reloads its URL when next opened.
    let Some(pinned) = browser.with(|s| s.tab(id).map(|t| t.pinned)) else {
        return Ok(());
    };
    let Some((kind, was_active, next)) = browser.with(|s| {
        let index = s.tabs.iter().position(|t| t.id == id)?;
        let was_active = s.active == Some(id);
        if was_active {
            s.active = None;
        }
        let tab = if pinned {
            s.tabs[index].clone()
        } else {
            let tab = s.tabs.remove(index);
            s.closed.push(match tab.kind {
                TabKind::Web => ClosedTab::Web(tab.url.clone()),
                TabKind::Terminal => ClosedTab::Terminal,
            });
            if s.closed.len() > MAX_CLOSED_TABS {
                s.closed.remove(0);
            }
            tab
        };
        // Prefer the tab that slid into this slot, else the one above it.
        let next = s
            .tabs
            .iter()
            .skip(index)
            .chain(s.tabs.iter().take(index).rev())
            .find(|t| t.id != id)
            .map(|t| t.id);
        Some((tab.kind, was_active, next))
    }) else {
        return Ok(());
    };

    // `activate` hides the previously active view, but this tab is no longer
    // active, so its view is handled here.
    let next = next.filter(|_| was_active);
    if let Some(next) = next {
        activate(app, next)?;
    }
    match kind {
        TabKind::Web => {
            if pinned {
                unload_tab(app, id)?;
                emit_state(app);
            } else if let Some(webview) = app.get_webview(&tab_label(id)) {
                #[cfg(target_os = "macos")]
                crate::native::unobserve(&webview, id);
                webview.close()?;
            }
        }
        TabKind::Terminal => {
            crate::pty::kill(app, id);
            // The shared terminal view stays up only if a terminal is now showing.
            let showing_terminal = next.is_some_and(|n| is_terminal_tab(app, n));
            if let Some(webview) = app
                .get_webview(TERM_WEBVIEW)
                .filter(|_| was_active && !showing_terminal)
            {
                webview.hide()?;
            }
        }
    }
    if next.is_none() {
        emit_state(app);
    }
    Ok(())
}

pub fn close_active_tab(app: &AppHandle) -> tauri::Result<()> {
    match app.state::<Browser>().with(|s| s.active) {
        Some(id) => close_tab_by_id(app, id),
        None => Ok(()),
    }
}

pub fn reopen_closed_tab(app: &AppHandle) -> tauri::Result<()> {
    match app.state::<Browser>().with(|s| s.closed.pop()) {
        Some(ClosedTab::Web(url)) => match Url::parse(&url) {
            Ok(url) => open_tab(app, url).map(|_| ()),
            Err(_) => Ok(()),
        },
        Some(ClosedTab::Terminal) => open_terminal(app).map(|_| ()),
        None => Ok(()),
    }
}

pub fn duplicate_tab(app: &AppHandle, id: u32) -> tauri::Result<()> {
    match tab_info(app, id) {
        Some((TabKind::Web, _, url)) => match Url::parse(&url) {
            Ok(url) => open_tab(app, url).map(|_| ()),
            Err(_) => Ok(()),
        },
        Some((TabKind::Terminal, ..)) => open_terminal(app).map(|_| ()),
        None => Ok(()),
    }
}

/// Pin moves a web tab to the end of the pinned group; unpin moves it to the
/// top of the regular list.
pub fn set_pinned(app: &AppHandle, id: u32, pinned: bool) {
    let changed = app.state::<Browser>().with(|s| {
        let index = s.tabs.iter().position(|t| t.id == id && t.kind == TabKind::Web)?;
        if s.tabs[index].pinned == pinned {
            return None;
        }
        let mut tab = s.tabs.remove(index);
        tab.pinned = pinned;
        let at = s.pinned_count();
        s.tabs.insert(at, tab);
        Some(())
    });
    if changed.is_some() {
        emit_state(app);
    }
}

/// Move a tab to `index`, kept within its group (pinned or not).
fn move_to(app: &AppHandle, id: u32, index: usize) {
    let moved = app.state::<Browser>().with(|s| {
        let from = s.tabs.iter().position(|t| t.id == id)?;
        let tab = s.tabs.remove(from);
        let pinned = s.pinned_count();
        let range = if tab.pinned { 0..=pinned } else { pinned..=s.tabs.len() };
        let to = index.clamp(*range.start(), *range.end());
        s.tabs.insert(to, tab);
        Some(from != to)
    });
    if moved == Some(true) {
        emit_state(app);
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

pub fn toggle_sidebar(app: &AppHandle) -> tauri::Result<()> {
    let collapsed = app.state::<Browser>().with(|s| {
        s.sidebar_collapsed = !s.sidebar_collapsed;
        s.sidebar_collapsed
    });
    layout_tabs(app)?;
    // With the sidebar gone, the traffic lights would sit on top of the page.
    #[cfg(target_os = "macos")]
    crate::native::set_traffic_lights_hidden(&main_window(app)?, collapsed);
    #[cfg(not(target_os = "macos"))]
    let _ = collapsed;
    emit_state(app);
    Ok(())
}

// --- Session -------------------------------------------------------------

pub fn session(app: &AppHandle) -> Session {
    app.state::<Browser>().with(|s| Session {
        tabs: s
            .tabs
            .iter()
            .map(|t| SavedTab {
                kind: t.kind,
                url: t.url.clone(),
                title: t.title.clone(),
                pinned: t.pinned,
            })
            .collect(),
        active: s.active.and_then(|id| s.tabs.iter().position(|t| t.id == id)),
        sidebar_width: s.sidebar_width,
        sidebar_collapsed: s.sidebar_collapsed,
    })
}

/// Recreate saved tabs, unloaded, then show the active one. Must not be
/// called from the main thread.
pub fn restore(app: &AppHandle, saved: Session) -> tauri::Result<()> {
    let browser = app.state::<Browser>();
    let active = browser.with(|s| {
        s.sidebar_width = saved.sidebar_width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        s.sidebar_collapsed = saved.sidebar_collapsed;
        let mut tabs: Vec<Tab> = saved
            .tabs
            .into_iter()
            .map(|t| {
                let mut tab = Tab::new(browser.next_id(), t.kind, t.url, t.title);
                tab.pinned = t.pinned && t.kind == TabKind::Web;
                tab
            })
            .collect();
        // Keep the pinned-first invariant even if the file was edited by hand.
        tabs.sort_by_key(|t| !t.pinned);
        let active = saved.active.and_then(|i| tabs.get(i)).map(|t| t.id);
        s.tabs = tabs;
        active.or_else(|| s.tabs.first().map(|t| t.id))
    });
    layout_tabs(app)?;
    #[cfg(target_os = "macos")]
    if saved.sidebar_collapsed {
        crate::native::set_traffic_lights_hidden(&main_window(app)?, true);
    }
    match active {
        Some(id) => activate(app, id),
        None => {
            emit_state(app);
            Ok(())
        }
    }
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
pub async fn new_terminal(app: AppHandle) -> Result<u32, String> {
    open_terminal(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_by_id(&app, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    activate(&app, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_tab(app: AppHandle, id: u32, index: usize) {
    move_to(&app, id, index);
}

#[tauri::command]
pub async fn pin_tab(app: AppHandle, id: u32, pinned: bool) {
    set_pinned(&app, id, pinned);
}

/// Terminals set their title from the shell (OSC 0/2 escape sequences).
#[tauri::command]
pub async fn set_tab_title(app: AppHandle, id: u32, title: String) {
    if is_terminal_tab(&app, id) && app.state::<Browser>().update_tab(id, |t| t.title = title) {
        schedule_emit(&app);
    }
}

/// Navigate the active web tab, or open a new one if there is none.
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

/// Called continuously while the user drags the sidebar edge.
#[tauri::command]
pub async fn set_sidebar_width(app: AppHandle, width: f64) -> Result<f64, String> {
    let width = width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
    app.state::<Browser>().with(|s| s.sidebar_width = width);
    layout_tabs(&app).map_err(|e| e.to_string())?;
    session::schedule_save(&app);
    Ok(width)
}
