//! Native menu. Browser shortcuts live here as accelerators so they work no
//! matter which webview (UI or page) has keyboard focus.

use tauri::{
    AppHandle, Manager, Wry,
    menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{browser, palette};

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
            &MenuItem::with_id(app, "new-terminal", "New Terminal Tab", true, Some("CmdOrCtrl+Alt+T"))?,
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
            &MenuItem::with_id(app, "toggle-sidebar", "Toggle Sidebar", true, Some("CmdOrCtrl+S"))?,
            &PredefinedMenuItem::separator(app)?,
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
            "new-tab" => palette::open(&app, palette::Mode::NewTab),
            "open-location" => palette::open(&app, palette::Mode::Edit),
            "new-terminal" => browser::open_terminal(&app).map(|_| ()),
            "close-tab" => browser::close_active_tab(&app),
            "reload" => browser::reload_active(&app),
            "back" => browser::back_active(&app),
            "forward" => browser::forward_active(&app),
            "reopen-tab" => browser::reopen_closed_tab(&app),
            "toggle-sidebar" => browser::toggle_sidebar(&app),
            "next-tab" => browser::cycle_tab(&app, 1),
            "previous-tab" => browser::cycle_tab(&app, -1),
            id if id.starts_with("select-tab-") => match id["select-tab-".len()..].parse() {
                Ok(n) => browser::select_tab_number(&app, n),
                Err(_) => Ok(()),
            },
            id if id.starts_with("ctx-") => match id.split_once(':').and_then(|(a, n)| Some((a, n.parse().ok()?))) {
                Some((action, tab)) => run_tab_action(&app, action, tab),
                None => Ok(()),
            },
            _ => Ok(()),
        };
        if let Err(err) = result {
            eprintln!("menu action {id} failed: {err}");
        }
    });
}

/// Run a menu action from the UI (sidebar buttons, command bar entries), so
/// every action has one implementation whether clicked, typed or pressed.
#[tauri::command]
pub fn menu_action(app: AppHandle, id: String) {
    run_action(&app, id);
}

/// Right-click menu for a tab in the sidebar. Item ids carry the tab id
/// (`ctx-pin:12`) and are handled in `run_action`.
#[tauri::command]
pub async fn tab_context_menu(app: AppHandle, id: u32) -> Result<(), String> {
    let (kind, pinned, _) = browser::tab_info(&app, id).ok_or("no such tab")?;
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        if kind == browser::TabKind::Web {
            let pin_label = if pinned { "Unpin Tab" } else { "Pin Tab" };
            menu.append(&MenuItem::with_id(&app, format!("ctx-pin:{id}"), pin_label, true, None::<&str>)?)?;
            menu.append(&MenuItem::with_id(&app, format!("ctx-duplicate:{id}"), "Duplicate Tab", true, None::<&str>)?)?;
            menu.append(&MenuItem::with_id(&app, format!("ctx-copy-link:{id}"), "Copy Link", true, None::<&str>)?)?;
            menu.append(&PredefinedMenuItem::separator(&app)?)?;
        }
        let close_label = if pinned { "Unload Tab" } else { "Close Tab" };
        menu.append(&MenuItem::with_id(&app, format!("ctx-close:{id}"), close_label, true, None::<&str>)?)?;
        Ok(menu)
    };
    let menu = build().map_err(|e| e.to_string())?;
    let window = app.get_window(browser::MAIN_WINDOW).ok_or("no window")?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

fn run_tab_action(app: &AppHandle, action: &str, id: u32) -> tauri::Result<()> {
    match action {
        "ctx-pin" => {
            let pinned = browser::tab_info(app, id).is_some_and(|(_, pinned, _)| pinned);
            browser::set_pinned(app, id, !pinned);
        }
        "ctx-duplicate" => browser::duplicate_tab(app, id)?,
        #[cfg(target_os = "macos")]
        "ctx-copy-link" => {
            if let Some((_, _, url)) = browser::tab_info(app, id) {
                crate::native::copy_to_clipboard(app, url);
            }
        }
        "ctx-close" => browser::close_tab_by_id(app, id)?,
        _ => {}
    }
    Ok(())
}
