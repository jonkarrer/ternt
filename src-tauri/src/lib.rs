mod browser;
#[cfg(all(debug_assertions, target_os = "macos"))]
mod debug;
mod menu;
mod model;
#[cfg(target_os = "macos")]
mod native;
mod palette;
mod pty;
mod session;

use browser::Browser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Browser::default())
        .manage(palette::Palette::default())
        .manage(pty::Terminals::default())
        .setup(|app| {
            browser::create_main_window(app.handle())?;
            app.set_menu(menu::build(app.handle())?)?;

            // Restore the last session, then open any `moove <url>...` args.
            let saved = session::load(app.handle());
            let urls: Vec<_> = std::env::args()
                .skip(1)
                .filter_map(|arg| tauri::Url::parse(&arg).ok())
                .collect();
            if urls.is_empty() && saved.as_ref().is_none_or(|s| s.tabs.is_empty()) {
                palette::open(app.handle(), palette::Mode::NewTab)?;
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(Err(err)) = saved.map(|saved| browser::restore(&handle, saved)) {
                    eprintln!("failed to restore session: {err}");
                }
                for url in urls {
                    if let Err(err) = browser::open_tab(&handle, url) {
                        eprintln!("failed to open tab: {err}");
                    }
                }
            });
            Ok(())
        })
        .on_menu_event(menu::handle)
        .invoke_handler(tauri::generate_handler![
            browser::get_state,
            browser::new_tab,
            browser::new_terminal,
            browser::close_tab,
            browser::activate_tab,
            browser::explore_preview,
            browser::explore_end,
            browser::move_tab,
            browser::create_space,
            browser::activate_space,
            browser::rename_space,
            browser::remove_space,
            browser::space_color,
            browser::space_profile,
            browser::create_folder,
            browser::rename_folder,
            browser::set_folder_open,
            browser::remove_folder,
            browser::set_tab_title,
            browser::navigate,
            browser::go_back,
            browser::go_forward,
            browser::reload,
            browser::set_sidebar_width,
            menu::menu_action,
            menu::tab_context_menu,
            menu::space_context_menu,
            menu::folder_context_menu,
            palette::palette_request,
            palette::open_palette,
            palette::close_palette,
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_ack,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_ready,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_log,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_eval,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_resize,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_screenshot,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_allow_popups,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_windows,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_set,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_get,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Saves are debounced; make sure the last change lands.
            if let tauri::RunEvent::Exit = event {
                session::save_now(app);
            }
        });
}
