//! Native menu. Browser shortcuts live here as accelerators so they work no
//! matter which webview (UI or page) has keyboard focus.

use tauri::{
    AppHandle, Wry,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::browser;

pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let app_menu = Submenu::with_items(
        app,
        "moove",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, "new-tab", "New Tab", true, Some("CmdOrCtrl+T"))?,
            &MenuItem::with_id(app, "open-location", "Open Location…", true, Some("CmdOrCtrl+L"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "close-tab", "Close Tab", true, Some("CmdOrCtrl+W"))?,
        ],
    )?;

    // Without these predefined items, copy/paste don't work in macOS webviews.
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &MenuItem::with_id(app, "reload", "Reload Page", true, Some("CmdOrCtrl+R"))?,
            &MenuItem::with_id(app, "back", "Back", true, Some("CmdOrCtrl+["))?,
            &MenuItem::with_id(app, "forward", "Forward", true, Some("CmdOrCtrl+]"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;

    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;

    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window])
}

pub fn handle(app: &AppHandle, event: MenuEvent) {
    let app = app.clone();
    let id = event.id().0.clone();
    // Menu events arrive on the main thread; webview operations block on it,
    // so run them from a worker.
    tauri::async_runtime::spawn(async move {
        let result = match id.as_str() {
            "new-tab" => browser::focus_omnibox(&app, true),
            "open-location" => browser::focus_omnibox(&app, false),
            "close-tab" => browser::close_active_tab(&app),
            "reload" => browser::reload_active(&app),
            "back" => browser::eval_active(&app, "history.back()"),
            "forward" => browser::eval_active(&app, "history.forward()"),
            _ => Ok(()),
        };
        if let Err(err) = result {
            eprintln!("menu action {id} failed: {err}");
        }
    });
}
