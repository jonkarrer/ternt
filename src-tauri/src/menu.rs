//! Native menu. Browser shortcuts live here as accelerators so they work no
//! matter which webview (UI or page) has keyboard focus.

use tauri::{
    AppHandle, Wry,
    menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
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
            &MenuItem::with_id(app, "reopen-tab", "Reopen Closed Tab", true, Some("CmdOrCtrl+Shift+T"))?,
        ],
    )?;

    let tab_numbers = (1..=9)
        .map(|n| {
            let label = if n == 9 { "Select Last Tab".into() } else { format!("Select Tab {n}") };
            MenuItem::with_id(app, format!("select-tab-{n}"), label, true, Some(&format!("CmdOrCtrl+{n}")))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let next = MenuItem::with_id(app, "next-tab", "Next Tab", true, Some("Ctrl+Tab"))?;
    let previous = MenuItem::with_id(app, "previous-tab", "Previous Tab", true, Some("Ctrl+Shift+Tab"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let mut tab_items: Vec<&dyn IsMenuItem<Wry>> = vec![&next, &previous, &separator];
    tab_items.extend(tab_numbers.iter().map(|item| item as &dyn IsMenuItem<Wry>));
    let tabs = Submenu::with_items(app, "Tabs", true, &tab_items)?;

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

    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &tabs, &window])
}

pub fn handle(app: &AppHandle, event: MenuEvent) {
    run_action(app, event.id().0.clone());
}

pub fn run_action(app: &AppHandle, id: String) {
    let app = app.clone();
    // Menu events arrive on the main thread; webview operations block on it,
    // so run them from a worker.
    tauri::async_runtime::spawn(async move {
        let result = match id.as_str() {
            "new-tab" => browser::focus_omnibox(&app, true),
            "open-location" => browser::focus_omnibox(&app, false),
            "close-tab" => browser::close_active_tab(&app),
            "reload" => browser::reload_active(&app),
            "back" => browser::back_active(&app),
            "forward" => browser::forward_active(&app),
            "reopen-tab" => browser::reopen_closed_tab(&app),
            "next-tab" => browser::cycle_tab(&app, 1),
            "previous-tab" => browser::cycle_tab(&app, -1),
            id if id.starts_with("select-tab-") => match id["select-tab-".len()..].parse() {
                Ok(n) => browser::select_tab_number(&app, n),
                Err(_) => Ok(()),
            },
            _ => Ok(()),
        };
        if let Err(err) = result {
            eprintln!("menu action {id} failed: {err}");
        }
    });
}
