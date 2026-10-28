//! Window layout, webviews, and the commands that change tabs and spaces.
//!
//! One window holds a transparent UI webview (the sidebar, full-window, at the
//! bottom) plus native child webviews stacked over the content area: one per
//! loaded web tab, and one shared webview that renders every terminal tab.
//! Rust owns the layout so content never waits on the UI to resize. The data
//! model (spaces, zones, folders, ordering) lives in `model.rs`.
//!
//! Web tabs load lazily: a tab's webview is created the first time it's
//! shown, so restoring a large session costs about as much as one tab, and
//! unloading (closing a favorite or pinned tab) frees the page but keeps it.
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

pub use crate::model::{TabKind, Zone};
use crate::{
    model::{ClosedTab, SPACE_COLORS, Saved, Snapshot, State},
    session,
};

pub const MAIN_WINDOW: &str = "main";
pub const UI_WEBVIEW: &str = "ui";
pub const TERM_WEBVIEW: &str = "term";

/// Gap between the content card and the window edges, in logical px.
/// Mirrored in `src/theme.css`.
const INSET: f64 = 8.0;
const CONTENT_RADIUS: f64 = 10.0;
const MIN_SIDEBAR_WIDTH: f64 = 180.0;
const MAX_SIDEBAR_WIDTH: f64 = 480.0;

/// WKWebView's default UA omits the Safari version token, which makes some
/// sites (Google, notably) serve degraded pages.
#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Navigation state read from a native webview.
pub struct NativeState {
    pub url: Option<String>,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Default)]
pub struct Browser {
    state: Mutex<State>,
    emit_pending: AtomicBool,
}

impl Browser {
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }
}

fn with<T>(app: &AppHandle, f: impl FnOnce(&mut State) -> T) -> T {
    app.state::<Browser>().with(f)
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
    match with(app, |s| s.tab(id).map(|t| t.kind))? {
        TabKind::Web => app.get_webview(&tab_label(id)),
        TabKind::Terminal => app.get_webview(TERM_WEBVIEW),
    }
}

/// The active tab's view, of either kind (for focus).
pub fn active_view(app: &AppHandle) -> Option<Webview> {
    view_for(app, with(app, |s| s.active)?)
}

/// The active tab's webview if it's a loaded web page (for navigation).
pub fn active_webview(app: &AppHandle) -> Option<Webview> {
    app.get_webview(&tab_label(with(app, |s| s.active)?))
}

/// The active web tab's URL; `None` for terminals or no tab.
pub fn active_tab_url(app: &AppHandle) -> Option<String> {
    with(app, |s| {
        let tab = s.tab(s.active?)?;
        (tab.kind == TabKind::Web).then(|| tab.url.clone())
    })
}

pub fn is_terminal_tab(app: &AppHandle, id: u32) -> bool {
    with(app, |s| s.tab(id).is_some_and(|t| t.kind == TabKind::Terminal))
}

/// What the tab context menu needs to know.
pub struct TabMenuInfo {
    pub kind: TabKind,
    pub zone: Zone,
    pub url: String,
    pub folder: Option<u32>,
    /// Folders of the active space: `(id, name)`.
    pub folders: Vec<(u32, String)>,
}

pub fn tab_menu_info(app: &AppHandle, id: u32) -> Option<TabMenuInfo> {
    with(app, |s| {
        let tab = s.tab(id)?;
        Some(TabMenuInfo {
            kind: tab.kind,
            zone: tab.zone,
            url: tab.url.clone(),
            folder: tab.folder,
            folders: s
                .folders
                .iter()
                .filter(|f| f.space == s.active_space)
                .map(|f| (f.id, f.name.clone()))
                .collect(),
        })
    })
}

/// `(color, isolated profile, can delete)` for the space context menu.
pub fn space_menu_info(app: &AppHandle, id: u32) -> Option<(String, bool, bool)> {
    with(app, |s| {
        let space = s.space(id)?;
        Some((space.color.clone(), space.profile.is_some(), s.spaces.len() > 1))
    })
}

/// Push the full state to the app webviews. Used directly after user actions
/// so the sidebar reacts in the same frame.
fn emit_state(app: &AppHandle) {
    let snapshot: Snapshot = with(app, |s| s.snapshot());
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
    let changed = with(app, |s| {
        let tab = s.tab_mut(id)?;
        if let Some(url) = native.url {
            tab.url = url;
        }
        tab.title = native.title;
        tab.loading = native.loading;
        tab.can_go_back = native.can_go_back;
        tab.can_go_forward = native.can_go_forward;
        Some(())
    });
    if changed.is_some() {
        schedule_emit(app);
    }
}

/// Left edge of the content pane, in logical px (overlays center on it).
pub fn content_left(app: &AppHandle) -> f64 {
    with(app, |s| s.effective_sidebar_width()).max(INSET)
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
    let ids = with(app, |s| s.tabs.iter().map(|t| t.id).collect::<Vec<_>>());
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

// --- Profiles (separate cookie jars per space) ----------------------------

/// A random 128-bit store id, as hex.
fn new_profile_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    (0..2u64)
        .map(|i| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(i);
            hasher.write_u128(nanos);
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

fn profile_bytes(hex: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Delete a profile's cookies and storage. WebKit refuses while a webview
/// still uses the store, so callers close those first.
fn remove_profile(app: &AppHandle, profile: String) {
    let Some(bytes) = profile_bytes(&profile) else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        // Let the webview closes queued before this land first.
        std::thread::sleep(Duration::from_millis(250));
        if let Err(err) = tauri::async_runtime::block_on(app.remove_data_store(bytes)) {
            eprintln!("failed to remove profile {profile}: {err}");
        }
    });
}

// --- Tabs -------------------------------------------------------------------

/// Create the webview for a web tab at its current URL. Must not be called
/// from the main thread (it blocks on it).
fn load_tab(app: &AppHandle, id: u32) -> tauri::Result<Webview> {
    let (url, _profile) = with(app, |s| {
        let tab = s.tab(id)?;
        Some((tab.url.clone(), s.profile_for(tab)))
    })
    .unwrap_or_default();
    let url = Url::parse(&url).unwrap_or_else(|_| "about:blank".parse().unwrap());

    let popup_app = app.clone();
    let builder = WebviewBuilder::new(tab_label(id), WebviewUrl::External(url))
        .zoom_hotkeys_enabled(true)
        .on_new_window(move |url, features| on_new_window(&popup_app, url, features));

    #[cfg(target_os = "macos")]
    let builder = {
        let builder = builder.user_agent(USER_AGENT);
        match _profile.as_deref().and_then(profile_bytes) {
            Some(store) => builder.data_store_identifier(store),
            None => builder,
        }
    };

    // macOS reads navigation state via KVO (see `native::observe`); elsewhere
    // fall back to load/title callbacks.
    #[cfg(not(target_os = "macos"))]
    let builder = builder
        .on_page_load(move |webview, payload| {
            let loading = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
            let url = payload.url().to_string();
            let app = webview.app_handle();
            let updated = with(app, |s| {
                s.tab_mut(id).map(|t| {
                    t.url = url;
                    t.loading = loading;
                })
            });
            if updated.is_some() {
                schedule_emit(app);
            }
        })
        .on_document_title_changed(move |webview, title| {
            let app = webview.app_handle();
            if with(app, |s| s.tab_mut(id).map(|t| t.title = title)).is_some() {
                schedule_emit(app);
            }
        });

    let bounds = content_bounds(app)?;
    let webview = main_window(app)?.add_child(builder, bounds.position, bounds.size)?;
    with(app, |s| {
        if let Some(tab) = s.tab_mut(id) {
            tab.loaded = true;
            tab.loading = true;
        }
    });

    #[cfg(target_os = "macos")]
    {
        crate::native::style_content(&webview, CONTENT_RADIUS, true);
        crate::native::observe(&webview, id, app.clone());
    }
    Ok(webview)
}

fn close_webview(app: &AppHandle, id: u32) -> tauri::Result<()> {
    if let Some(webview) = app.get_webview(&tab_label(id)) {
        #[cfg(target_os = "macos")]
        crate::native::unobserve(&webview, id);
        webview.close()?;
    }
    Ok(())
}

/// Destroy a web tab's webview but keep the tab (it reloads when shown).
fn unload_tab(app: &AppHandle, id: u32) -> tauri::Result<()> {
    close_webview(app, id)?;
    with(app, |s| {
        if let Some(tab) = s.tab_mut(id) {
            tab.loaded = false;
            tab.loading = false;
        }
    });
    Ok(())
}

/// Open a web tab at the top of Today and show it. Must not be called from
/// the main thread (it blocks on it).
pub fn open_tab(app: &AppHandle, url: Url) -> tauri::Result<u32> {
    let id = with(app, |s| s.add_tab(TabKind::Web, url.to_string(), String::new()));
    if let Err(err) = activate(app, id) {
        with(app, |s| s.tabs.retain(|t| t.id != id));
        emit_state(app);
        return Err(err);
    }
    Ok(id)
}

/// Must not be called from the main thread (it blocks on it).
pub fn open_terminal(app: &AppHandle) -> tauri::Result<u32> {
    let id = with(app, |s| s.add_tab(TabKind::Terminal, String::new(), "Terminal".into()));
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
    let Some((kind, loaded)) = with(app, |s| s.tab(id).map(|t| (t.kind, t.loaded))) else {
        return Ok(());
    };

    // Lazily create the view before switching to it.
    let next_view = match kind {
        TabKind::Web if !loaded => load_tab(app, id)?,
        TabKind::Web => app.get_webview(&tab_label(id)).map_or_else(|| load_tab(app, id), Ok)?,
        TabKind::Terminal => ensure_term_webview(app)?,
    };
    let previous = with(app, |s| s.focus(id)).flatten();

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

/// Show nothing (an empty space): hide the active view.
fn deactivate(app: &AppHandle) -> tauri::Result<()> {
    let previous = with(app, |s| s.active.take());
    if let Some(webview) = previous.and_then(|p| view_for(app, p)) {
        webview.hide()?;
    }
    emit_state(app);
    Ok(())
}

pub fn close_tab_by_id(app: &AppHandle, id: u32) -> tauri::Result<()> {
    // Favorites and pinned tabs unload instead of closing (Arc behavior):
    // they stay in the sidebar and reload their URL when next opened.
    let Some((tab, was_active, next)) = with(app, |s| {
        let next = s.neighbor(id);
        let tab = s.remove_tab(id)?;
        let was_active = s.active == Some(id);
        if was_active {
            s.active = None;
        }
        Some((tab, was_active, next))
    }) else {
        return Ok(());
    };

    // `activate` hides the previously active view, but this tab is no longer
    // active, so its view is handled here.
    let next = next.filter(|_| was_active);
    if let Some(next) = next {
        activate(app, next)?;
    }
    match tab.kind {
        TabKind::Web if tab.is_kept() => unload_tab(app, id)?,
        TabKind::Web => close_webview(app, id)?,
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
    emit_state(app);
    Ok(())
}

pub fn close_active_tab(app: &AppHandle) -> tauri::Result<()> {
    match with(app, |s| s.active) {
        Some(id) => close_tab_by_id(app, id),
        None => Ok(()),
    }
}

/// Reopen the most recently closed tab, in the space it was closed from.
pub fn reopen_closed_tab(app: &AppHandle) -> tauri::Result<()> {
    let Some(closed) = with(app, |s| {
        let closed = s.closed.pop()?;
        let (ClosedTab::Web { space, .. } | ClosedTab::Terminal { space }) = &closed;
        if s.space(*space).is_some() {
            s.active_space = *space;
        }
        Some(closed)
    }) else {
        return Ok(());
    };
    match closed {
        ClosedTab::Web { url, .. } => match Url::parse(&url) {
            Ok(url) => open_tab(app, url).map(|_| ()),
            Err(_) => Ok(()),
        },
        ClosedTab::Terminal { .. } => open_terminal(app).map(|_| ()),
    }
}

pub fn duplicate_tab(app: &AppHandle, id: u32) -> tauri::Result<()> {
    match tab_menu_info(app, id) {
        Some(TabMenuInfo { kind: TabKind::Web, url, .. }) => match Url::parse(&url) {
            Ok(url) => open_tab(app, url).map(|_| ()),
            Err(_) => Ok(()),
        },
        Some(TabMenuInfo { kind: TabKind::Terminal, .. }) => open_terminal(app).map(|_| ()),
        None => Ok(()),
    }
}

/// Move a tab into a zone/folder at a position (sidebar drag and drop, and
/// the tab context menu). `index: None` appends; Today prepends instead.
pub fn move_tab_to(app: &AppHandle, id: u32, zone: Zone, folder: Option<u32>, index: Option<usize>) {
    let index = index.unwrap_or(if zone == Zone::Today { 0 } else { usize::MAX });
    if with(app, |s| s.move_tab(id, zone, folder, index)) {
        emit_state(app);
    }
}

/// Activate the tab `delta` positions away in the sidebar, wrapping.
pub fn cycle_tab(app: &AppHandle, delta: isize) -> tauri::Result<()> {
    let target = with(app, |s| {
        let order = s.visible_order(s.active_space);
        let len = order.len() as isize;
        let current = s
            .active
            .and_then(|a| order.iter().position(|&t| t == a))
            .map_or(-1, |i| i as isize);
        (len > 0).then(|| order[(current + delta).rem_euclid(len) as usize])
    });
    match target {
        Some(id) => activate(app, id),
        None => Ok(()),
    }
}

/// ⌘1–⌘8 select by sidebar position; ⌘9 always selects the last tab.
pub fn select_tab_number(app: &AppHandle, number: usize) -> tauri::Result<()> {
    let target = with(app, |s| {
        let order = s.visible_order(s.active_space);
        if number == 9 {
            order.last().copied()
        } else {
            order.get(number.checked_sub(1)?).copied()
        }
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
    let collapsed = with(app, |s| {
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

// --- Spaces -----------------------------------------------------------------

pub fn switch_space(app: &AppHandle, space: u32) -> tauri::Result<()> {
    if with(app, |s| s.space(space).is_none() || s.active_space == space) {
        return Ok(());
    }
    match with(app, |s| s.switch_space(space)) {
        Some(id) => activate(app, id),
        None => deactivate(app),
    }
}

pub fn cycle_space(app: &AppHandle, delta: isize) -> tauri::Result<()> {
    match with(app, |s| s.space_at_offset(delta)) {
        Some(space) => switch_space(app, space),
        None => Ok(()),
    }
}

/// ⌃1–⌃9 select a space by position.
pub fn select_space_number(app: &AppHandle, number: usize) -> tauri::Result<()> {
    let space = with(app, |s| s.spaces.get(number.checked_sub(1)?).map(|s| s.id));
    match space {
        Some(space) => switch_space(app, space),
        None => Ok(()),
    }
}

pub fn new_space(app: &AppHandle) -> tauri::Result<u32> {
    let id = with(app, |s| s.add_space());
    switch_space(app, id)?;
    emit_state(app);
    Ok(id)
}

pub fn delete_space(app: &AppHandle, id: u32) -> tauri::Result<()> {
    if with(app, |s| s.spaces.len() <= 1 || s.space(id).is_none()) {
        return Ok(());
    }
    // Move off the space first so its views are hidden, not torn down live.
    if with(app, |s| s.active_space == id) {
        let neighbor = with(app, |s| {
            let index = s.spaces.iter().position(|x| x.id == id)?;
            s.spaces
                .get(index + 1)
                .or_else(|| s.spaces.get(index.checked_sub(1)?))
                .map(|x| x.id)
        });
        if let Some(neighbor) = neighbor {
            switch_space(app, neighbor)?;
        }
    }
    let Some((removed, space)) = with(app, |s| s.remove_space(id)) else {
        return Ok(());
    };
    for tab in removed {
        match tab.kind {
            TabKind::Web => close_webview(app, tab.id)?,
            TabKind::Terminal => crate::pty::kill(app, tab.id),
        }
    }
    if let Some(profile) = space.profile {
        remove_profile(app, profile);
    }
    emit_state(app);
    Ok(())
}

pub fn set_space_color(app: &AppHandle, id: u32, color: &str) {
    if !SPACE_COLORS.contains(&color) {
        return;
    }
    if with(app, |s| s.space_mut(id).map(|x| x.color = color.into())).is_some() {
        emit_state(app);
    }
}

/// Give a space its own cookie jar and storage, or return it to the shared
/// one. Its loaded tabs reload under the new store; a dropped store's data is
/// deleted.
pub fn set_space_profile(app: &AppHandle, id: u32, isolated: bool) -> tauri::Result<()> {
    let Some((old, affected, active)) = with(app, |s| {
        let space = s.space_mut(id)?;
        if space.profile.is_some() == isolated {
            return None;
        }
        let old = space.profile.take();
        if isolated {
            space.profile = Some(new_profile_id());
        }
        let affected: Vec<u32> = s
            .tabs
            .iter()
            .filter(|t| t.loaded && t.zone != Zone::Favorite && t.space == id)
            .map(|t| t.id)
            .collect();
        Some((old, affected, s.active))
    }) else {
        return Ok(());
    };
    for tab in &affected {
        unload_tab(app, *tab)?;
    }
    if let Some(active) = active.filter(|a| affected.contains(a)) {
        activate(app, active)?;
    }
    if let Some(old) = old {
        remove_profile(app, old);
    }
    emit_state(app);
    Ok(())
}

/// Ask the sidebar to start inline-renaming a space or folder.
pub fn request_rename(app: &AppHandle, kind: &str, id: u32) -> tauri::Result<()> {
    #[derive(Clone, Serialize)]
    struct Rename<'a> {
        kind: &'a str,
        id: u32,
    }
    if let Some(ui) = app.get_webview(UI_WEBVIEW) {
        ui.set_focus()?;
    }
    app.emit_to(EventTarget::webview(UI_WEBVIEW), "browser:rename", Rename { kind, id })
}

// --- Folders ----------------------------------------------------------------

pub fn new_folder(app: &AppHandle) -> Option<u32> {
    let id = with(app, |s| s.add_folder(s.active_space, "New Folder".into()))?;
    emit_state(app);
    Some(id)
}

pub fn delete_folder(app: &AppHandle, id: u32) {
    if with(app, |s| s.remove_folder(id)) {
        emit_state(app);
    }
}

// --- Session ----------------------------------------------------------------

pub fn session(app: &AppHandle) -> Saved {
    with(app, |s| s.save())
}

/// Recreate a saved session (tabs unloaded), then show the active tab. Must
/// not be called from the main thread.
pub fn restore(app: &AppHandle, saved: Saved) -> tauri::Result<()> {
    let (active, collapsed) = with(app, |s| {
        *s = State::restore(saved);
        s.sidebar_width = s.sidebar_width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        (s.active.take(), s.sidebar_collapsed)
    });
    layout_tabs(app)?;
    #[cfg(target_os = "macos")]
    if collapsed {
        crate::native::set_traffic_lights_hidden(&main_window(app)?, true);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = collapsed;
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

// --- Commands ---------------------------------------------------------------
// Async so they run off the main thread (see `open_tab`).

fn err(e: tauri::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn get_state(app: AppHandle) -> Snapshot {
    with(&app, |s| s.snapshot())
}

#[tauri::command]
pub async fn new_tab(app: AppHandle, url: String) -> Result<u32, String> {
    open_tab(&app, parse_url(&url)?).map_err(err)
}

#[tauri::command]
pub async fn new_terminal(app: AppHandle) -> Result<u32, String> {
    open_terminal(&app).map_err(err)
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_by_id(&app, id).map_err(err)
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    activate(&app, id).map_err(err)
}

/// `index` is the position within the target group after removing the tab.
#[tauri::command]
pub async fn move_tab(app: AppHandle, id: u32, zone: Zone, folder: Option<u32>, index: Option<usize>) {
    move_tab_to(&app, id, zone, folder, index);
}

/// Terminals set their title from the shell (OSC 0/2 escape sequences).
#[tauri::command]
pub async fn set_tab_title(app: AppHandle, id: u32, title: String) {
    let updated = with(&app, |s| {
        let tab = s.tab_mut(id).filter(|t| t.kind == TabKind::Terminal)?;
        tab.title = title;
        Some(())
    });
    if updated.is_some() {
        schedule_emit(&app);
    }
}

/// Navigate the active web tab, or open a new one if there is none.
#[tauri::command]
pub async fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let url = parse_url(&url)?;
    let Some(webview) = active_webview(&app) else {
        return open_tab(&app, url).map(|_| ()).map_err(err);
    };
    webview.navigate(url).map_err(err)?;
    webview.set_focus().map_err(err)
}

#[tauri::command]
pub async fn go_back(app: AppHandle) -> Result<(), String> {
    back_active(&app).map_err(err)
}

#[tauri::command]
pub async fn go_forward(app: AppHandle) -> Result<(), String> {
    forward_active(&app).map_err(err)
}

#[tauri::command]
pub async fn reload(app: AppHandle) -> Result<(), String> {
    reload_active(&app).map_err(err)
}

/// Called continuously while the user drags the sidebar edge.
#[tauri::command]
pub async fn set_sidebar_width(app: AppHandle, width: f64) -> Result<f64, String> {
    let width = width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
    with(&app, |s| s.sidebar_width = width);
    layout_tabs(&app).map_err(err)?;
    session::schedule_save(&app);
    Ok(width)
}

#[tauri::command]
pub async fn create_space(app: AppHandle) -> Result<u32, String> {
    new_space(&app).map_err(err)
}

#[tauri::command]
pub async fn activate_space(app: AppHandle, id: u32) -> Result<(), String> {
    switch_space(&app, id).map_err(err)
}

#[tauri::command]
pub async fn rename_space(app: AppHandle, id: u32, name: String) {
    let name = name.trim().to_string();
    if !name.is_empty() && with(&app, |s| s.space_mut(id).map(|x| x.name = name)).is_some() {
        emit_state(&app);
    }
}

#[tauri::command]
pub async fn remove_space(app: AppHandle, id: u32) -> Result<(), String> {
    delete_space(&app, id).map_err(err)
}

#[tauri::command]
pub async fn space_color(app: AppHandle, id: u32, color: String) {
    set_space_color(&app, id, &color);
}

#[tauri::command]
pub async fn space_profile(app: AppHandle, id: u32, isolated: bool) -> Result<(), String> {
    set_space_profile(&app, id, isolated).map_err(err)
}

#[tauri::command]
pub async fn create_folder(app: AppHandle) -> Option<u32> {
    new_folder(&app)
}

#[tauri::command]
pub async fn rename_folder(app: AppHandle, id: u32, name: String) {
    let name = name.trim().to_string();
    if !name.is_empty() && with(&app, |s| s.folder_mut(id).map(|f| f.name = name)).is_some() {
        emit_state(&app);
    }
}

#[tauri::command]
pub async fn set_folder_open(app: AppHandle, id: u32, open: bool) {
    if with(&app, |s| s.folder_mut(id).map(|f| f.open = open)).is_some() {
        emit_state(&app);
    }
}

#[tauri::command]
pub async fn remove_folder(app: AppHandle, id: u32) {
    delete_folder(&app, id);
}
