//! The setup window (`onboarding.html` → `src/onboarding/`): shown once, at the very first
//! launch, and from Help → Set Up Ternt… after that. It asks whether to use the AI agent
//! and, if so, takes the keys the agent needs: a provider's is required, Jev's highly
//! recommended. The welcome page can't do this: it is a plain web tab without IPC, and a
//! key must go from the page that takes it to the Keychain and nowhere else.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const ONBOARDING_WINDOW: &str = "onboarding";

/// Open the window, or bring it forward if it's already open. Must not be called from the
/// main thread (window creation blocks on it).
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    // Focusing a window activates the app; test runs stay in the background (see `lib.rs`).
    let testing = cfg!(debug_assertions) && std::env::var_os("TERNT_TEST").is_some();
    if let Some(window) = app.get_webview_window(ONBOARDING_WINDOW) {
        window.unminimize()?;
        return if testing {
            window.show()
        } else {
            window.set_focus()
        };
    }
    let builder = WebviewWindowBuilder::new(
        app,
        ONBOARDING_WINDOW,
        WebviewUrl::App("onboarding.html".into()),
    )
    .title("Set Up Ternt")
    .inner_size(620.0, 700.0)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .focused(!testing)
    .center();
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    builder.build()?;
    Ok(())
}

/// The page is done (Continue): close its window.
#[tauri::command]
pub async fn close_onboarding(app: AppHandle) -> Result<(), String> {
    match app.get_webview_window(ONBOARDING_WINDOW) {
        Some(window) => window.close().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}
