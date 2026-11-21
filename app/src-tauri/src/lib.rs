//! Tokidoki, the app: the `tokidoki` library in a window.
//!
//! The core runs in this process. The window's page calls the commands below, one per
//! `Tokidoki` method, and follows the `tokidoki:event` events, which are `interface::Event`s
//! as JSON: the page is a client of the same protocol anything else would speak.
//!
//! The core opens once there is an Anthropic key (`keys`). It keeps running while the window
//! is closed, so idle threads still dream and unfinished work still finishes; quitting stops
//! it, and the next launch resumes what was cut off.

mod keys;
mod updater;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tokidoki::interface::{Event, Feedback, Input, Memory, Sent, Speaker, ThreadSummary, WhenBusy};
use tokidoki::{Config, Tokidoki, Tools};

use keys::Keys;

/// The running core, once it has opened.
#[derive(Default)]
struct Core {
    /// Held while opening, so two callers never open the same files twice.
    running: Mutex<Option<Tokidoki>>,
}

/// Where the app is, for the page to choose what to show.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Status {
    /// No Anthropic key yet: ask for one.
    NeedsKeys,
    Ready,
    /// The core didn't open.
    Failed {
        message: String,
    },
}

type Said<T> = Result<T, String>;

/// An error as the page shows it: the whole chain, on one line.
fn say(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn core(app: &AppHandle) -> Said<Tokidoki> {
    (app.state::<Core>().running.lock().unwrap().clone())
        .ok_or_else(|| "Tokidoki isn't running yet.".to_string())
}

/// Where the databases live: `TOKIDOKI_DATA` if set (a scratch run), else the app's data
/// folder. Development builds keep theirs apart, so `just app` never touches what the
/// installed app remembers.
fn data_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TOKIDOKI_DATA").filter(|dir| !dir.is_empty()) {
        return Ok(dir.into());
    }
    let dir = app.path().app_data_dir()?;
    Ok(match cfg!(debug_assertions) {
        true => dir.join("dev"),
        false => dir,
    })
}

/// Opens the core with `keys`, unless it is open, and passes its events on to the page from
/// then on.
fn open(app: &AppHandle, keys: Keys) -> anyhow::Result<()> {
    let core = app.state::<Core>();
    let mut running = core.running.lock().unwrap();
    if running.is_some() {
        return Ok(());
    }
    let config = Config::new(data_dir(app)?, tokidoki::Vendor::Anthropic, keys.anthropic);
    let tokidoki = Tokidoki::open(config, Tools::new())?;
    let mut events = tokidoki.events();
    *running = Some(tokidoki);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let _ = app.emit("tokidoki:event", event);
        }
    });
    Ok(())
}

/// Stops the core: running work resumes at the next launch.
pub(crate) async fn close_core(app: &AppHandle) {
    let running = app.state::<Core>().running.lock().unwrap().take();
    if let Some(tokidoki) = running
        && let Err(error) = tokidoki.close().await
    {
        eprintln!("closing tokidoki: {error:#}");
    }
}

/// Opens the core if it can be and isn't, and says where things stand.
#[tauri::command]
async fn status(app: AppHandle) -> Status {
    if app.state::<Core>().running.lock().unwrap().is_some() {
        return Status::Ready;
    }
    let opened = match keys::load() {
        Ok(Some(keys)) => open(&app, keys),
        Ok(None) => return Status::NeedsKeys,
        Err(error) => Err(error),
    };
    match opened {
        Ok(()) => Status::Ready,
        Err(error) => Status::Failed {
            message: say(error),
        },
    }
}

/// Which keys are set, for the form: never the keys themselves.
#[derive(Serialize)]
struct KeysSet {
    anthropic: bool,
    baseten: bool,
}

#[tauri::command]
fn keys_set() -> Said<KeysSet> {
    let keys = keys::load().map_err(say)?.unwrap_or_default();
    Ok(KeysSet {
        anthropic: !keys.anthropic.is_empty(),
        baseten: !keys.baseten.is_empty(),
    })
}

/// Checks the Anthropic key with Anthropic, stores the keys in the Keychain, and starts the
/// core with them. `None` keeps the key there is; an empty Baseten key removes it. When the
/// core was already running, the app relaunches to use them.
#[tauri::command]
async fn set_keys(app: AppHandle, anthropic: Option<String>, baseten: Option<String>) -> Said<()> {
    let mut keys = keys::load().map_err(say)?.unwrap_or_default();
    if let Some(key) = anthropic.map(|key| key.trim().to_string()) {
        check_anthropic(&key).await?;
        keys.anthropic = key;
    }
    if let Some(key) = baseten {
        keys.baseten = key.trim().to_string();
    }
    if keys.anthropic.is_empty() {
        return Err("Tokidoki needs an Anthropic API key.".into());
    }
    keys::save(&keys).map_err(say)?;
    if app.state::<Core>().running.lock().unwrap().is_some() {
        close_core(&app).await;
        app.restart();
    }
    open(&app, keys).map_err(say)
}

async fn check_anthropic(key: &str) -> Said<()> {
    let response = reqwest::Client::new()
        .get("https://api.anthropic.com/v1/models?limit=1")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|_| "Couldn't reach Anthropic to check the key. Check your connection.")?;
    match response.status().as_u16() {
        200..300 => Ok(()),
        401 | 403 => Err("Anthropic didn't accept that key. Check it and paste it again.".into()),
        status => Err(format!(
            "Anthropic answered {status} while checking the key."
        )),
    }
}

/// Sends what the user wrote: to `thread`, or wherever the router decides. `steer` joins the
/// answer under way instead of waiting for it.
#[tauri::command]
async fn send(
    app: AppHandle,
    text: String,
    thread: Option<String>,
    steer: bool,
    request_id: Option<String>,
) -> Said<Sent> {
    let mut input = Input::text(Speaker::owner(), text);
    input.thread = thread;
    input.request_id = request_id;
    if steer {
        input.when_busy = WhenBusy::Steer;
    }
    core(&app)?.send(input).await.map_err(say)
}

#[tauri::command]
fn threads(app: AppHandle) -> Said<Vec<ThreadSummary>> {
    core(&app)?.threads().map_err(say)
}

#[tauri::command]
fn history(app: AppHandle, thread: String, since: usize) -> Said<Vec<Event>> {
    core(&app)?.history(&thread, since).map_err(say)
}

#[tauri::command]
fn abort(app: AppHandle, thread: String) -> Said<bool> {
    core(&app)?.abort(&thread).map_err(say)
}

#[tauri::command]
fn feedback(app: AppHandle, thread: String, turn: Option<u32>, text: String) -> Said<()> {
    let feedback = Feedback {
        from: Speaker::owner(),
        thread,
        turn,
        text,
    };
    core(&app)?.feedback(feedback).map_err(say)
}

/// What it remembers, newest first.
#[tauri::command]
fn memories(app: AppHandle) -> Said<Vec<Memory>> {
    core(&app)?.memories(&Speaker::owner()).map_err(say)
}

/// Keeps something the user tells it outright.
#[tauri::command]
fn remember(app: AppHandle, text: String) -> Said<Memory> {
    core(&app)?.remember(&Speaker::owner(), &text).map_err(say)
}

/// The user's correction of the memory `id`.
#[tauri::command]
fn revise_memory(app: AppHandle, id: i64, text: String) -> Said<Memory> {
    core(&app)?
        .revise(&Speaker::owner(), id, &text)
        .map_err(say)
}

/// Removes the memory `id`, for good.
#[tauri::command]
fn forget_memory(app: AppHandle, id: i64) -> Said<()> {
    core(&app)?.forget(&Speaker::owner(), id).map_err(say)
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn run() {
    let builder = tauri::Builder::default();
    // One core per data folder: a second launch brings the first forward instead. Not in
    // development, whose data is its own, so `just app` runs beside the installed app.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        show_window(app)
    }));
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Core::default())
        .manage(updater::Updates::default())
        .setup(|app| {
            updater::start(app.handle());
            Ok(())
        })
        // Closing the window leaves Tokidoki running; the Dock icon brings it back.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            status,
            keys_set,
            set_keys,
            send,
            threads,
            history,
            abort,
            feedback,
            memories,
            remember,
            revise_memory,
            forget_memory,
            updater::update_status,
            updater::check_for_updates,
            updater::install_update,
            updater::app_version,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tokidoki")
        .run(|app, event| match event {
            RunEvent::Reopen { .. } => show_window(app),
            RunEvent::Exit => {
                tauri::async_runtime::block_on(close_core(app));
                updater::install_on_exit(app);
            }
            _ => {}
        });
}
