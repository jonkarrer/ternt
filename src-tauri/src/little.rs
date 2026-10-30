//! Little Ternt: links opened from other apps (Ternt as the default browser)
//! open in a small window of their own, to read and close, or move into a
//! tab with "Open in Ternt". Settings → Links can send them to tabs instead.
//!
//! Each window holds two webviews, like the main one: a bar on top
//! (`little.html` → `src/little/`, with IPC) and the page below (none). The
//! page signs in with the active space's profile.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Url, Webview,
    WebviewUrl,
    webview::{PageLoadEvent, WebviewBuilder},
    window::WindowBuilder,
};

use crate::prefs::LinkTarget;

pub const PREFIX: &str = "little-";
/// Height of the bar above the page, in logical px. Mirrored in
/// `src/little/Little.svelte`.
const BAR_HEIGHT: f64 = 40.0;

/// What a window's bar shows.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LittleState {
    url: String,
    title: String,
    loading: bool,
    /// The space "Open in Ternt" moves the page to.
    space: String,
}

/// Open windows by label.
#[derive(Default)]
pub struct Littles(Mutex<HashMap<String, LittleState>>);

fn bar_label(window: &str) -> String {
    format!("{window}-bar")
}

fn page_label(window: &str) -> String {
    format!("{window}-page")
}

/// The window a bar or page webview belongs to.
fn window_of(webview: &str) -> &str {
    webview.trim_end_matches("-bar").trim_end_matches("-page")
}

/// A link from another app: a Little Ternt window or a new tab, per
/// Settings. Must not be called from the main thread.
pub fn open_external(app: &AppHandle, url: Url) -> tauri::Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Ok(());
    }
    match crate::prefs::get(app).external_links {
        LinkTarget::Little => open(app, url),
        LinkTarget::Tab => {
            // At launch the link can arrive before the session is back.
            crate::browser::wait_until_restored();
            crate::browser::open_tab(app, url)?;
            crate::browser::focus_main_window(app)
        }
    }
}

/// Open `url` in a new Little Ternt window. Must not be called from the
/// main thread.
pub fn open(app: &AppHandle, url: Url) -> tauri::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let label = format!("{PREFIX}{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let (profile, space) = crate::browser::active_space_profile(app);
    app.state::<Littles>().0.lock().unwrap().insert(
        label.clone(),
        LittleState {
            url: url.to_string(),
            title: String::new(),
            loading: true,
            space,
        },
    );

    let builder = WindowBuilder::new(app, &label)
        .title(url.host_str().unwrap_or("Little Ternt"))
        .inner_size(800.0, 600.0)
        .min_inner_size(420.0, 300.0)
        .center();
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    let window = builder.build()?;
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);

    let bar = WebviewBuilder::new(bar_label(&label), WebviewUrl::App("little.html".into()));
    window.add_child(
        bar,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, BAR_HEIGHT),
    )?;

    let popup_app = app.clone();
    let page = WebviewBuilder::new(page_label(&label), WebviewUrl::External(url))
        .zoom_hotkeys_enabled(true)
        .on_new_window(move |url, features| {
            crate::browser::on_new_window(&popup_app, url, features)
        })
        .on_page_load(|webview, payload| {
            let loading = matches!(payload.event(), PageLoadEvent::Started);
            let url = payload.url().to_string();
            update(webview.app_handle(), webview.label(), |s| {
                s.url = url;
                s.loading = loading;
            });
        })
        .on_document_title_changed(|webview, title| {
            let window = window_of(webview.label()).to_string();
            if let Some(w) = webview.app_handle().get_window(&window) {
                let _ = w.set_title(&title);
            }
            update(webview.app_handle(), webview.label(), |s| s.title = title);
        });
    #[cfg(target_os = "macos")]
    let page = {
        let page = page.user_agent(crate::browser::USER_AGENT);
        match profile.as_deref().and_then(crate::browser::profile_bytes) {
            Some(store) => page.data_store_identifier(store),
            None => page,
        }
    };
    #[cfg(not(target_os = "macos"))]
    let _ = profile;
    let page = window.add_child(
        page,
        LogicalPosition::new(0.0, BAR_HEIGHT),
        LogicalSize::new(size.width, (size.height - BAR_HEIGHT).max(0.0)),
    )?;

    let app = app.clone();
    let window_label = label.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(_) => {
            let (app, label) = (app.clone(), window_label.clone());
            std::thread::spawn(move || layout(&app, &label));
        }
        tauri::WindowEvent::Destroyed => {
            app.state::<Littles>()
                .0
                .lock()
                .unwrap()
                .remove(&window_label);
        }
        _ => {}
    });
    page.set_focus()
}

/// Keep the bar on top and the page filling the rest.
fn layout(app: &AppHandle, label: &str) {
    let Some(window) = app.get_window(label) else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let size = size.to_logical::<f64>(window.scale_factor().unwrap_or(1.0));
    if let Some(bar) = app.get_webview(&bar_label(label)) {
        let _ = bar.set_bounds(tauri::Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width, BAR_HEIGHT).into(),
        });
    }
    if let Some(page) = app.get_webview(&page_label(label)) {
        let _ = page.set_bounds(tauri::Rect {
            position: LogicalPosition::new(0.0, BAR_HEIGHT).into(),
            size: LogicalSize::new(size.width, (size.height - BAR_HEIGHT).max(0.0)).into(),
        });
    }
}

/// Change a window's state and tell its bar.
fn update(app: &AppHandle, webview: &str, change: impl FnOnce(&mut LittleState)) {
    let window = window_of(webview).to_string();
    let littles = app.state::<Littles>();
    let state = {
        let mut littles = littles.0.lock().unwrap();
        let Some(state) = littles.get_mut(&window) else {
            return;
        };
        change(state);
        state.clone()
    };
    let _ = app.emit_to(
        EventTarget::webview(bar_label(&window)),
        "little:state",
        state,
    );
}

/// ⌘W closes a Little Ternt window when it's in front. Returns whether it did.
pub fn close_if_focused(app: &AppHandle) -> bool {
    let focused = app
        .windows()
        .into_iter()
        .find(|(label, w)| label.starts_with(PREFIX) && w.is_focused().unwrap_or(false));
    match focused {
        Some((_, window)) => window.close().is_ok(),
        None => false,
    }
}

/// Move the page into a tab of the active space (it reloads there) and
/// close the window.
pub fn open_in_ternt(app: &AppHandle, window: &str) -> tauri::Result<()> {
    let Some(url) = app
        .state::<Littles>()
        .0
        .lock()
        .unwrap()
        .get(window)
        .map(|s| s.url.clone())
    else {
        return Ok(());
    };
    if let Some(w) = app.get_window(window) {
        w.close()?;
    }
    // Let the window go before the main one comes forward.
    std::thread::sleep(Duration::from_millis(50));
    if let Ok(url) = Url::parse(&url) {
        crate::browser::open_tab(app, url)?;
    }
    crate::browser::focus_main_window(app)
}

#[tauri::command]
pub fn little_state(app: AppHandle, webview: Webview) -> Option<LittleState> {
    app.state::<Littles>()
        .0
        .lock()
        .unwrap()
        .get(window_of(webview.label()))
        .cloned()
}

#[tauri::command]
pub async fn little_open_in_ternt(app: AppHandle, webview: Webview) -> Result<(), String> {
    open_in_ternt(&app, window_of(webview.label())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview_labels_map_to_their_window() {
        assert_eq!(window_of(&bar_label("little-3")), "little-3");
        assert_eq!(window_of(&page_label("little-3")), "little-3");
        assert_eq!(window_of("little-3"), "little-3");
    }
}
