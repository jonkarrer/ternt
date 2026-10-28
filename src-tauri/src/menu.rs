//! Native menu. Browser shortcuts live here as accelerators so they work no
//! matter which webview (UI or page) has keyboard focus.

use tauri::{
    AppHandle, Manager, Wry,
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

use crate::{
    browser::{self, TabKind, Zone},
    model::SPACE_COLORS,
    palette,
};

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

    let space_numbers = (1..=9)
        .map(|n| MenuItem::with_id(app, format!("select-space-{n}"), format!("Space {n}"), true, Some(&format!("Ctrl+{n}"))))
        .collect::<tauri::Result<Vec<_>>>()?;
    let new_space = MenuItem::with_id(app, "new-space", "New Space", true, None::<&str>)?;
    let new_folder = MenuItem::with_id(app, "new-folder", "New Folder", true, None::<&str>)?;
    let next_space = MenuItem::with_id(app, "next-space", "Next Space", true, Some("CmdOrCtrl+Alt+Right"))?;
    let previous_space = MenuItem::with_id(app, "previous-space", "Previous Space", true, Some("CmdOrCtrl+Alt+Left"))?;
    let space_separator = PredefinedMenuItem::separator(app)?;
    let mut space_items: Vec<&dyn IsMenuItem<Wry>> =
        vec![&new_space, &new_folder, &space_separator, &next_space, &previous_space, &separator];
    space_items.extend(space_numbers.iter().map(|item| item as &dyn IsMenuItem<Wry>));
    let spaces = Submenu::with_items(app, "Spaces", true, &space_items)?;

    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &tabs, &spaces, &window])
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
            "new-space" => browser::new_space(&app).map(|_| ()),
            "new-folder" => browser::new_folder(&app).map_or(Ok(()), |id| browser::request_rename(&app, "folder", id)),
            "next-space" => browser::cycle_space(&app, 1),
            "previous-space" => browser::cycle_space(&app, -1),
            id if id.starts_with("select-space-") => match id["select-space-".len()..].parse() {
                Ok(n) => browser::select_space_number(&app, n),
                Err(_) => Ok(()),
            },
            id if id.starts_with("select-tab-") => match id["select-tab-".len()..].parse() {
                Ok(n) => browser::select_tab_number(&app, n),
                Err(_) => Ok(()),
            },
            id if id.starts_with("ctx-") => run_context_action(&app, id),
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

// --- Context menus -----------------------------------------------------------
//
// Item ids carry their target and argument: `ctx-<action>:<id>[:<arg>]`, and
// are dispatched by `run_context_action` through the normal menu event path.

fn item(app: &AppHandle, id: String, label: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, label, true, None::<&str>)
}

fn popup(app: &AppHandle, menu: tauri::Result<Menu<Wry>>) -> Result<(), String> {
    let menu = menu.map_err(|e| e.to_string())?;
    let window = app.get_window(browser::MAIN_WINDOW).ok_or("no window")?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

/// Right-click menu for a tab in the sidebar.
#[tauri::command]
pub async fn tab_context_menu(app: AppHandle, id: u32) -> Result<(), String> {
    let info = browser::tab_menu_info(&app, id).ok_or("no such tab")?;
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        if info.kind == TabKind::Web {
            let (fav_action, fav_label) = match info.zone {
                Zone::Favorite => ("ctx-today", "Remove from Favorites"),
                _ => ("ctx-favorite", "Add to Favorites"),
            };
            menu.append(&item(&app, format!("{fav_action}:{id}"), fav_label)?)?;
            let (pin_action, pin_label) = match info.zone {
                Zone::Pinned => ("ctx-today", "Unpin Tab"),
                _ => ("ctx-pin", "Pin Tab"),
            };
            menu.append(&item(&app, format!("{pin_action}:{id}"), pin_label)?)?;

            let folders = Submenu::new(&app, "Move to Folder", true)?;
            for (folder, name) in &info.folders {
                let enabled = info.folder != Some(*folder);
                folders.append(&MenuItem::with_id(&app, format!("ctx-move-folder:{id}:{folder}"), name, enabled, None::<&str>)?)?;
            }
            if !info.folders.is_empty() {
                folders.append(&PredefinedMenuItem::separator(&app)?)?;
            }
            folders.append(&item(&app, format!("ctx-new-folder:{id}"), "New Folder")?)?;
            menu.append(&folders)?;
            if info.folder.is_some() {
                menu.append(&item(&app, format!("ctx-pin:{id}"), "Remove from Folder")?)?;
            }
            menu.append(&PredefinedMenuItem::separator(&app)?)?;
            menu.append(&item(&app, format!("ctx-duplicate:{id}"), "Duplicate Tab")?)?;
            menu.append(&item(&app, format!("ctx-copy-link:{id}"), "Copy Link")?)?;
            menu.append(&PredefinedMenuItem::separator(&app)?)?;
        }
        let close_label = if info.zone == Zone::Today { "Close Tab" } else { "Unload Tab" };
        menu.append(&item(&app, format!("ctx-close:{id}"), close_label)?)?;
        Ok(menu)
    };
    popup(&app, build())
}

/// Right-click menu for a space in the switcher.
#[tauri::command]
pub async fn space_context_menu(app: AppHandle, id: u32) -> Result<(), String> {
    let (color, isolated, deletable) = browser::space_menu_info(&app, id).ok_or("no such space")?;
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        menu.append(&item(&app, format!("ctx-rename-space:{id}"), "Rename Space…")?)?;
        let colors = Submenu::new(&app, "Color", true)?;
        for option in SPACE_COLORS {
            let label = format!("{}{}", option[..1].to_uppercase(), &option[1..]);
            colors.append(&CheckMenuItem::with_id(&app, format!("ctx-space-color:{id}:{option}"), label, true, option == color, None::<&str>)?)?;
        }
        menu.append(&colors)?;
        menu.append(&CheckMenuItem::with_id(
            &app,
            format!("ctx-space-profile:{id}"),
            "Separate Profile (own cookies and logins)",
            true,
            isolated,
            None::<&str>,
        )?)?;
        menu.append(&PredefinedMenuItem::separator(&app)?)?;
        menu.append(&MenuItem::with_id(&app, format!("ctx-delete-space:{id}"), "Delete Space", deletable, None::<&str>)?)?;
        Ok(menu)
    };
    popup(&app, build())
}

/// Right-click menu for a pinned folder.
#[tauri::command]
pub async fn folder_context_menu(app: AppHandle, id: u32) -> Result<(), String> {
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        menu.append(&item(&app, format!("ctx-rename-folder:{id}"), "Rename Folder…")?)?;
        menu.append(&item(&app, format!("ctx-delete-folder:{id}"), "Delete Folder (keep tabs)")?)?;
        Ok(menu)
    };
    popup(&app, build())
}

fn run_context_action(app: &AppHandle, id: &str) -> tauri::Result<()> {
    let mut parts = id.split(':');
    let action = parts.next().unwrap_or_default();
    let Some(target) = parts.next().and_then(|n| n.parse::<u32>().ok()) else {
        return Ok(());
    };
    let arg = parts.next();
    match action {
        // Tabs
        "ctx-favorite" => browser::move_tab_to(app, target, Zone::Favorite, None, None),
        "ctx-pin" => browser::move_tab_to(app, target, Zone::Pinned, None, None),
        "ctx-today" => browser::move_tab_to(app, target, Zone::Today, None, None),
        "ctx-move-folder" => {
            let folder = arg.and_then(|f| f.parse().ok());
            browser::move_tab_to(app, target, Zone::Pinned, folder, None);
        }
        "ctx-new-folder" => {
            if let Some(folder) = browser::new_folder(app) {
                browser::move_tab_to(app, target, Zone::Pinned, Some(folder), None);
                browser::request_rename(app, "folder", folder)?;
            }
        }
        "ctx-duplicate" => browser::duplicate_tab(app, target)?,
        #[cfg(target_os = "macos")]
        "ctx-copy-link" => {
            if let Some(info) = browser::tab_menu_info(app, target) {
                crate::native::copy_to_clipboard(app, info.url);
            }
        }
        "ctx-close" => browser::close_tab_by_id(app, target)?,
        // Spaces
        "ctx-rename-space" => browser::request_rename(app, "space", target)?,
        "ctx-space-color" => browser::set_space_color(app, target, arg.unwrap_or_default()),
        "ctx-space-profile" => {
            let isolated = browser::space_menu_info(app, target).is_some_and(|(_, isolated, _)| isolated);
            browser::set_space_profile(app, target, !isolated)?;
        }
        "ctx-delete-space" => browser::delete_space(app, target)?,
        // Folders
        "ctx-rename-folder" => browser::request_rename(app, "folder", target)?,
        "ctx-delete-folder" => browser::delete_folder(app, target),
        _ => {}
    }
    Ok(())
}
