mod browser;
mod menu;
#[cfg(target_os = "macos")]
mod native;

use browser::Browser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Browser::default())
        .setup(|app| {
            browser::create_main_window(app.handle())?;
            app.set_menu(menu::build(app.handle())?)?;

            // `moove <url>...` opens each URL as a tab.
            let urls: Vec<_> = std::env::args()
                .skip(1)
                .filter_map(|arg| tauri::Url::parse(&arg).ok())
                .collect();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
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
            browser::close_tab,
            browser::activate_tab,
            browser::navigate,
            browser::go_back,
            browser::go_forward,
            browser::reload,
            browser::set_sidebar_width,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
