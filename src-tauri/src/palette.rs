//! Command bar overlay.
//!
//! Page webviews are native views stacked above the UI webview, so the UI
//! can't draw over them. The palette is its own transparent webview covering
//! the window. It's created hidden at startup so opening it is instant, and
//! raised above the tabs each time it's shown.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, Manager, Window, WebviewUrl,
    webview::WebviewBuilder,
};

use crate::browser::{self, UI_WEBVIEW};

pub const PALETTE_WEBVIEW: &str = "palette";

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Results open in a new tab.
    NewTab,
    /// Prefilled with the active tab's URL; results replace it.
    Edit,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    mode: Mode,
    url: Option<String>,
    /// Where the content pane starts, so the bar centers over the page
    /// rather than the whole window.
    content_left: f64,
}

/// The open request, if the palette is showing. Read by the palette on load
/// so a request made before it finished loading isn't lost.
#[derive(Default)]
pub struct Palette(Mutex<Option<Request>>);

pub fn create(window: &Window) -> tauri::Result<()> {
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    let builder = WebviewBuilder::new(PALETTE_WEBVIEW, WebviewUrl::App("palette.html".into()))
        .transparent(true)
        .focused(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.auto_resize();
    let webview = window.add_child(builder, LogicalPosition::new(0.0, 0.0), size)?;
    webview.hide()?;
    #[cfg(target_os = "macos")]
    crate::native::track_window_size(&webview);
    Ok(())
}

pub fn open(app: &AppHandle, mode: Mode) -> tauri::Result<()> {
    // The command bar takes over from explore mode (keeping what it showed).
    if browser::exploring(app) {
        browser::end_explore(app, browser::ExploreEnd::Commit);
    }
    let url = browser::active_tab_url(app);
    let mode = if url.is_none() { Mode::NewTab } else { mode };
    let request = Request {
        mode,
        url: if mode == Mode::Edit { url } else { None },
        content_left: browser::content_left(app),
    };
    *app.state::<Palette>().0.lock().unwrap() = Some(request.clone());

    let Some(webview) = app.get_webview(PALETTE_WEBVIEW) else {
        return Ok(());
    };
    // Render the new request before the view becomes visible.
    app.emit_to(EventTarget::webview(PALETTE_WEBVIEW), "palette:open", request)?;
    #[cfg(target_os = "macos")]
    crate::native::bring_to_front(&webview);
    webview.show()?;
    webview.set_focus()
}

pub fn close(app: &AppHandle) -> tauri::Result<()> {
    if app.state::<Palette>().0.lock().unwrap().take().is_none() {
        return Ok(());
    }
    if let Some(webview) = app.get_webview(PALETTE_WEBVIEW) {
        webview.hide()?;
    }
    // Hand keyboard focus back to the page, or the sidebar if there is none.
    match browser::active_view(app).or_else(|| app.get_webview(UI_WEBVIEW)) {
        Some(webview) => webview.set_focus(),
        None => Ok(()),
    }
}

#[tauri::command]
pub async fn palette_request(app: AppHandle) -> Option<Request> {
    app.state::<Palette>().0.lock().unwrap().clone()
}

#[tauri::command]
pub async fn open_palette(app: AppHandle, mode: Mode) -> Result<(), String> {
    open(&app, mode).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_palette(app: AppHandle) -> Result<(), String> {
    close(&app).map_err(|e| e.to_string())
}
