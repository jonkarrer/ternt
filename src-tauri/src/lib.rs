mod browser;
#[cfg(all(debug_assertions, target_os = "macos"))]
mod debug;
mod history;
mod little;
mod menu;
mod model;
#[cfg(target_os = "macos")]
mod native;
mod palette;
mod prefs;
mod pty;
mod session;
mod settings;
mod updater;
mod welcome;

use browser::Browser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(all(debug_assertions, target_os = "macos"))]
    debug::load_env_from_args();
    let builder = tauri::Builder::default();
    // Test runs launch the app over and over; don't pull it in front of
    // whatever the user is working in each time.
    #[cfg(all(debug_assertions, target_os = "macos"))]
    let builder = builder.activate_ignoring_other_apps(std::env::var_os("TERNT_TEST").is_none());
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Browser::default())
        .manage(palette::Palette::default())
        .manage(pty::Terminals::default())
        .manage(history::Store::default())
        .manage(updater::Updates::default())
        .manage(prefs::Store::default())
        .manage(little::Littles::default())
        .register_asynchronous_uri_scheme_protocol(welcome::SCHEME, welcome::respond)
        .setup(|app| {
            browser::create_main_window(app.handle())?;
            app.set_menu(menu::build(app.handle())?)?;
            history::load(app.handle());
            prefs::load(app.handle());
            updater::start(app.handle());

            // Restore the last session, then open any `ternt <url>...` args.
            // No session at all means a first launch: show the welcome page.
            // A session with no tabs just opens the command bar.
            let saved = session::load(app.handle());
            let first_launch = saved.is_none();
            let urls: Vec<_> = std::env::args()
                .skip(1)
                .filter_map(|arg| tauri::Url::parse(&arg).ok())
                .collect();
            if urls.is_empty() && !first_launch && saved.as_ref().is_some_and(|s| s.tabs.is_empty()) {
                palette::open(app.handle(), palette::Mode::NewTab)?;
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(Err(err)) = saved.map(|saved| browser::restore(&handle, saved)) {
                    eprintln!("failed to restore session: {err}");
                }
                browser::mark_restored();
                // Welcome first, so a URL given on the command line ends up active.
                if first_launch && let Err(err) = welcome::open(&handle) {
                    eprintln!("failed to open the welcome page: {err}");
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
            browser::space_icon,
            browser::rename_profile,
            browser::split_view,
            browser::set_split_ratio,
            browser::reopen_closed,
            browser::clear_profile_history,
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
            history::history_search,
            updater::update_status,
            updater::check_for_updates,
            updater::install_update,
            updater::app_version,
            settings::reveal_data_dir,
            settings::default_browser,
            settings::make_default_browser,
            prefs::get_prefs,
            prefs::set_prefs,
            little::little_state,
            little::little_open_in_ternt,
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
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_clipboard,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_focused,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_view,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_focus,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            debug::debug_open_external,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            match event {
                // Saves are debounced; make sure the last change lands.
                tauri::RunEvent::Exit => {
                    session::save_now(app);
                    history::save_now(app);
                }
                // Links from other apps (Ternt as the default browser).
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Opened { urls } => {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        for url in urls {
                            if let Err(err) = little::open_external(&app, url) {
                                eprintln!("failed to open a link: {err}");
                            }
                        }
                    });
                }
                _ => {}
            }
        });
}
