//! The Settings window (Ternt → Settings…, ⌘,): its own window and webview
//! (`settings.html` → `src/settings/`), one instance at a time. It mirrors
//! the browser state like the sidebar does (`browser:state`) and changes it
//! through the same commands.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const SETTINGS_WINDOW: &str = "settings";

/// Open the window, or bring it forward if it's already open. Must not be
/// called from the main thread (window creation blocks on it).
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        window.unminimize()?;
        // Focusing a window activates the app; test runs stay in the
        // background (see `lib.rs`).
        if cfg!(debug_assertions) && std::env::var_os("TERNT_TEST").is_some() {
            return window.show();
        }
        return window.set_focus();
    }
    let builder = WebviewWindowBuilder::new(app, SETTINGS_WINDOW, WebviewUrl::App("settings.html".into()))
        .title("Settings")
        .inner_size(720.0, 540.0)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .center();
    // The tab bar sits in the title bar, as in Arc and macOS settings.
    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    builder.build()?;
    Ok(())
}

/// ⌘W is a menu accelerator, so it reaches the menu whichever window is in
/// front: close Settings when it's the one in front. Returns whether it was.
pub fn close_if_focused(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window(SETTINGS_WINDOW) else {
        return false;
    };
    if !window.is_focused().unwrap_or(false) {
        return false;
    }
    let _ = window.close();
    true
}

/// Show the app's data folder (sessions, history) in Finder.
#[tauri::command]
pub async fn reveal_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = crate::session::data_dir(&app).ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::process::Command::new("open").arg(&dir).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Whether Ternt opens web links from other apps.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultBrowser {
    is_default: bool,
    /// Something else is (or nothing is known): the button can ask.
    can_ask: bool,
}

#[tauri::command]
pub fn default_browser(app: AppHandle) -> DefaultBrowser {
    #[cfg(target_os = "macos")]
    {
        let is_default = crate::native::is_default_browser(&app.config().identifier);
        DefaultBrowser { is_default, can_ask: !is_default }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        DefaultBrowser { is_default: false, can_ask: false }
    }
}

/// Ask macOS to make Ternt the default browser (it confirms with the user).
#[tauri::command]
pub async fn make_default_browser(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let id = app.config().identifier.clone();
        app.run_on_main_thread(move || crate::native::make_default_browser(&id))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("not supported on this platform".into())
    }
}
