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
    /// Set while exploring tabs (⌘F).
    explore: Mutex<Option<Explore>>,
}

/// Where explore mode started, to return there on cancel.
struct Explore {
    origin: Option<u32>,
    origin_space: u32,
    /// The sidebar was collapsed and opened for exploring.
    reopened_sidebar: bool,
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

/// The active space's profile (for its history).
pub fn active_profile(app: &AppHandle) -> Option<String> {
    with(app, |s| s.profile_of(s.active_space).map(String::from))
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

/// What the space context menu needs to know.
pub struct SpaceMenuInfo {
    pub color: String,
    pub profile: Option<String>,
    pub deletable: bool,
    /// The profiles it could use.
    pub groups: Vec<crate::model::ProfileGroup>,
}

pub fn space_menu_info(app: &AppHandle, id: u32) -> Option<SpaceMenuInfo> {
    with(app, |s| {
        let space = s.space(id)?;
        Some(SpaceMenuInfo {
            color: space.color.clone(),
            profile: space.profile.clone(),
            deletable: s.spaces.len() > 1,
            groups: s.profile_groups(),
        })
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
        let profile = s.profile_for(s.tab(id)?);
        let tab = s.tab_mut(id)?;
        if let Some(url) = native.url {
            tab.url = url;
        }
        tab.title = native.title;
        tab.loading = native.loading;
        tab.can_go_back = native.can_go_back;
        tab.can_go_forward = native.can_go_forward;
        // A finished load is a visit (history ignores repeats of the same URL).
        let visit = (!tab.loading).then(|| (tab.url.clone(), tab.title.clone()));
        Some((profile, visit))
    });
    let Some((profile, visit)) = changed else { return };
    if let Some((url, title)) = visit {
        crate::history::record(app, id, profile.as_deref(), &url, &title);
    }
    schedule_emit(app);
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
    if exploring(app) {
        update_dim(app)?;
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
    crate::history::clear(app, Some(&profile));
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
    crate::history::forget_tab(app, id);
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
    open_terminal_running(app, None)
}

/// A new terminal, optionally running `command` once its shell starts.
/// Must not be called from the main thread.
pub fn open_terminal_running(app: &AppHandle, command: Option<String>) -> tauri::Result<u32> {
    let title = command.clone().unwrap_or_else(|| "Terminal".into());
    let id = with(app, |s| s.add_tab(TabKind::Terminal, String::new(), title));
    if let Some(command) = command {
        crate::pty::queue_startup_input(id, format!("{command}\n"));
    }
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
    if exploring(app) {
        // Previewing: the sidebar keeps the keyboard, the dim stays on top.
        update_dim(app)?;
    } else {
        next_view.set_focus()?;
    }
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

/// Close the active space's Today web tabs (Arc's "Clear"). Terminals stay:
/// closing one kills its shell, which ⇧⌘T can't bring back.
pub fn clear_today(app: &AppHandle) -> tauri::Result<()> {
    let (mut ids, active) = with(app, |s| {
        let ids: Vec<u32> = s
            .tabs
            .iter()
            .filter(|t| t.zone == Zone::Today && t.space == s.active_space && t.kind == TabKind::Web)
            .map(|t| t.id)
            .collect();
        (ids, s.active)
    });
    // Close the active tab last, so focus never lands on (and lazily loads)
    // a tab that is about to close.
    ids.sort_by_key(|&id| Some(id) == active);
    for id in ids {
        close_tab_by_id(app, id)?;
    }
    Ok(())
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
    // Each new space gets its own logins; it can share another's via the
    // space menu.
    let id = with(app, |s| s.add_space(Some(new_profile_id())));
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
    if let Some(profile) = space.profile.filter(|p| !with(app, |s| s.profile_in_use(p))) {
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

/// Which cookie jar a space uses (the space menu's Profile submenu).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileChoice {
    /// The default store, shared by every space that picks it.
    Shared,
    /// A fresh, empty store.
    New,
    /// Whatever store another space uses.
    SameAs(u32),
}

impl ProfileChoice {
    /// `shared`, `new` or `space-<id>`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "shared" => Some(Self::Shared),
            "new" => Some(Self::New),
            _ => value.strip_prefix("space-")?.parse().ok().map(Self::SameAs),
        }
    }
}

/// Switch a space's cookie jar and storage. Its loaded tabs (favorites
/// included) reload under the new store; a store no space uses any more is
/// deleted.
pub fn set_space_profile(app: &AppHandle, id: u32, choice: ProfileChoice) -> tauri::Result<()> {
    let Some((old, affected, active)) = with(app, |s| {
        let profile = match choice {
            ProfileChoice::Shared => None,
            ProfileChoice::New => Some(new_profile_id()),
            ProfileChoice::SameAs(other) => s.space(other)?.profile.clone(),
        };
        let (old, affected) = s.set_space_profile(id, profile)?;
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
    if let Some(old) = old.filter(|p| !with(app, |s| s.profile_in_use(p))) {
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

// --- Explore mode (⌘F) --------------------------------------------------------
//
// The content dims and the sidebar takes the keyboard: the UI moves a cursor
// over the tabs (no IPC per keypress), previews with Space and opens with
// Enter. Rust owns the mode so focus and the dim stay consistent.

#[derive(Clone, Copy, Debug)]
pub enum ExploreEnd {
    /// Open this tab.
    Open(u32),
    /// Stay on whatever is showing.
    Commit,
    /// Go back to the tab shown when exploring started.
    Cancel,
}

pub fn exploring(app: &AppHandle) -> bool {
    app.state::<Browser>().explore.lock().unwrap().is_some()
}

/// Cover the content area with the dim (or refresh its frame and z-order).
fn update_dim(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(ui) = app.get_webview(UI_WEBVIEW) {
        let bounds = content_bounds(app)?;
        let (position, size) = (bounds.position.to_logical::<f64>(1.0), bounds.size.to_logical::<f64>(1.0));
        let frame = exploring(app).then_some((position.x, position.y, size.width, size.height));
        crate::native::set_dim(&ui, frame, CONTENT_RADIUS);
    }
    Ok(())
}

fn emit_explore(app: &AppHandle, on: bool) {
    let _ = app.emit_to(EventTarget::webview(UI_WEBVIEW), "browser:explore", on);
}

/// ⌘F: start exploring, or (pressed again) stop on what's showing.
pub fn toggle_explore(app: &AppHandle) -> tauri::Result<()> {
    if exploring(app) {
        end_explore(app, ExploreEnd::Commit);
        return Ok(());
    }
    let (origin, origin_space, collapsed) = with(app, |s| (s.active, s.active_space, s.sidebar_collapsed));
    *app.state::<Browser>().explore.lock().unwrap() = Some(Explore {
        origin,
        origin_space,
        reopened_sidebar: collapsed,
    });
    if collapsed {
        toggle_sidebar(app)?;
    }
    update_dim(app)?;
    if let Some(ui) = app.get_webview(UI_WEBVIEW) {
        ui.set_focus()?;
    }
    emit_explore(app, true);
    Ok(())
}

/// Leave explore mode. Must not be called from the main thread.
pub fn end_explore(app: &AppHandle, end: ExploreEnd) {
    let Some(explore) = app.state::<Browser>().explore.lock().unwrap().take() else {
        return;
    };
    // The mode is off, so activating focuses the page as usual.
    let result = (|| -> tauri::Result<()> {
        match end {
            ExploreEnd::Open(id) => activate(app, id)?,
            ExploreEnd::Commit => {
                if let Some(view) = active_view(app) {
                    view.set_focus()?;
                }
            }
            ExploreEnd::Cancel => {
                if with(app, |s| s.space(explore.origin_space).is_some()) {
                    with(app, |s| s.active_space = explore.origin_space);
                }
                match explore.origin.filter(|&id| with(app, |s| s.tab(id).is_some())) {
                    Some(id) => activate(app, id)?,
                    None => deactivate(app)?,
                }
            }
        }
        update_dim(app)?;
        if explore.reopened_sidebar && !with(app, |s| s.sidebar_collapsed) {
            toggle_sidebar(app)?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        eprintln!("failed to end explore mode: {err}");
    }
    emit_explore(app, false);
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
/// `command`, if given, runs in the new terminal's shell.
pub async fn new_terminal(app: AppHandle, command: Option<String>) -> Result<u32, String> {
    let command = command.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    open_terminal_running(&app, command).map_err(err)
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, id: u32) -> Result<(), String> {
    close_tab_by_id(&app, id).map_err(err)
}

#[tauri::command]
pub async fn activate_tab(app: AppHandle, id: u32) -> Result<(), String> {
    // Clicking a tab while exploring opens it for real.
    if exploring(&app) {
        end_explore(&app, ExploreEnd::Open(id));
        return Ok(());
    }
    activate(&app, id).map_err(err)
}

/// Show a tab while exploring, without leaving explore mode.
#[tauri::command]
pub async fn explore_preview(app: AppHandle, id: u32) -> Result<(), String> {
    activate(&app, id).map_err(err)
}

/// Leave explore mode: open `open`, or with `cancel` return to where it
/// started, else stay on what's showing.
#[tauri::command]
pub async fn explore_end(app: AppHandle, open: Option<u32>, cancel: bool) {
    let end = match (open, cancel) {
        (_, true) => ExploreEnd::Cancel,
        (Some(id), _) => ExploreEnd::Open(id),
        (None, _) => ExploreEnd::Commit,
    };
    end_explore(&app, end);
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
pub async fn space_profile(app: AppHandle, id: u32, profile: String) -> Result<(), String> {
    let choice = ProfileChoice::parse(&profile).ok_or("invalid profile choice")?;
    set_space_profile(&app, id, choice).map_err(err)
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

#[cfg(test)]
mod tests {
    use super::ProfileChoice;

    #[test]
    fn profile_choices_parse() {
        assert_eq!(ProfileChoice::parse("shared"), Some(ProfileChoice::Shared));
        assert_eq!(ProfileChoice::parse("new"), Some(ProfileChoice::New));
        assert_eq!(ProfileChoice::parse("space-12"), Some(ProfileChoice::SameAs(12)));
        assert_eq!(ProfileChoice::parse("space-"), None);
        assert_eq!(ProfileChoice::parse("space-x"), None);
        assert_eq!(ProfileChoice::parse(""), None);
    }

    #[test]
    fn profile_ids_round_trip_to_store_bytes() {
        let id = super::new_profile_id();
        assert_eq!(id.len(), 32);
        let bytes = super::profile_bytes(&id).unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, id);
        assert_ne!(super::new_profile_id(), id, "ids are unique");
        assert_eq!(super::profile_bytes("zz"), None);
    }
}
