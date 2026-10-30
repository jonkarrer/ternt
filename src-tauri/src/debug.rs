//! Debug-build-only test harness.
//!
//! `TERNT_TEST=path/to/script.js` runs the script inside the UI webview once
//! it has mounted. The script drives the app through the same commands the UI
//! uses (`window.__ternt`, see src/lib/debug.ts) plus the helpers here.

use std::process::Command;

use tauri::{AppHandle, LogicalSize, Manager};

use crate::browser::{MAIN_WINDOW, UI_WEBVIEW};

/// Called by the UI on mount; evaluates the test script if one was given.
#[tauri::command]
pub async fn debug_ready(app: AppHandle) -> Result<(), String> {
    let Ok(path) = std::env::var("TERNT_TEST") else {
        return Ok(());
    };
    let script = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    // Start from a clean slate: no command bar over the window.
    crate::palette::close(&app).map_err(|e| e.to_string())?;
    let ui = app.get_webview(UI_WEBVIEW).ok_or("no ui webview")?;
    ui.eval(format!(
        "(async () => {{ const t = window.__ternt; try {{ {script}\n }} catch (e) {{ await t.log('FAIL ' + (e?.message ?? e) + '\\n' + (e?.stack ?? '')); }} finally {{ await t.log('TEST DONE'); }} }})()"
    ))
    .map_err(|e| e.to_string())
}

/// Test settings can arrive as `--ternt-test-env=<file>` (lines of
/// `KEY=VALUE`) instead of the environment: relaunching after an update goes
/// through LaunchServices, which keeps arguments but not the environment.
pub fn load_env_from_args() {
    let Some(path) = std::env::args().find_map(|a| a.strip_prefix("--ternt-test-env=").map(String::from)) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
        // SAFETY: called first thing in `run`, before any other thread exists.
        unsafe { std::env::set_var(key.trim(), value.trim()) };
    }
}

/// Prints `[test] …`, and appends it to `$TERNT_TEST_LOG` if set (a relaunched
/// app's stdout goes nowhere).
#[tauri::command]
pub fn debug_log(message: String) {
    println!("[test] {message}");
    if let Some(path) = std::env::var_os("TERNT_TEST_LOG") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[test] {message}");
        }
    }
}

/// Evaluate JS in another webview (palette, terminal, a tab).
#[tauri::command]
pub async fn debug_eval(app: AppHandle, label: String, js: String) -> Result<(), String> {
    app.get_webview(&label)
        .ok_or_else(|| format!("no webview {label}"))?
        .eval(js)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn debug_resize(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    app.get_window(MAIN_WINDOW)
        .ok_or("no window")?
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())
}

/// Capture just one window (the main one unless `window` names another) to
/// `$TERNT_SHOTS/<name>.png`.
#[tauri::command]
pub async fn debug_screenshot(app: AppHandle, name: String, window: Option<String>) -> Result<String, String> {
    let window = app.get_window(window.as_deref().unwrap_or(MAIN_WINDOW)).ok_or("no window")?;
    let ns_window = window.ns_window().map_err(|e| e.to_string())? as *const objc2::runtime::AnyObject;
    let number: isize = unsafe { objc2::msg_send![&*ns_window, windowNumber] };
    let dir = std::env::var("TERNT_SHOTS").unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let path = format!("{dir}/{name}.png");
    let status = Command::new("screencapture")
        .args(["-x", "-o", "-l", &number.to_string(), &path])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("screencapture exited with {status}"));
    }
    Ok(path)
}

/// Let scripts open windows without a user gesture, so tests can exercise
/// the popup path (WebKit blocks gesture-less `window.open` by default).
#[tauri::command]
pub async fn debug_allow_popups(app: AppHandle, label: String) -> Result<(), String> {
    let webview = app.get_webview(&label).ok_or_else(|| format!("no webview {label}"))?;
    webview
        .with_webview(|platform| unsafe {
            let view = &*(platform.inner() as *const objc2::runtime::AnyObject);
            let config: *mut objc2::runtime::AnyObject = objc2::msg_send![view, configuration];
            let prefs: *mut objc2::runtime::AnyObject = objc2::msg_send![&*config, preferences];
            let _: () = objc2::msg_send![&*prefs, setJavaScriptCanOpenWindowsAutomatically: true];
        })
        .map_err(|e| e.to_string())
}

/// Labels of all open windows (popups, Settings, Little Ternt).
#[tauri::command]
pub fn debug_windows(app: AppHandle) -> Vec<String> {
    // Windows holding several webviews (main, Little Ternt) aren't
    // webview windows.
    app.windows().into_keys().collect()
}

#[tauri::command]
pub fn debug_quit(app: AppHandle) {
    app.exit(0);
}

/// Scratch values other webviews hand back to the test script (it can only
/// eval into them, not read results).
static VALUES: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

#[tauri::command]
pub fn debug_set(key: String, value: String) {
    let mut values = VALUES.lock().unwrap();
    values.retain(|(k, _)| *k != key);
    values.push((key, value));
}

#[tauri::command]
pub fn debug_get(key: String) -> Option<String> {
    VALUES.lock().unwrap().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
}

/// The clipboard's text, to verify Copy URL / Copy Link.
#[tauri::command]
pub fn debug_clipboard() -> Result<String, String> {
    let out = Command::new("pbpaste").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Which webview has keyboard focus (holds the window's first responder),
/// independent of whether the app is frontmost.
#[tauri::command]
pub async fn debug_focused(app: AppHandle) -> Option<String> {
    let window = app.get_window(MAIN_WINDOW)?;
    let ns_window = window.ns_window().ok()? as usize;
    let (tx, rx) = std::sync::mpsc::channel();
    let webviews: Vec<(String, usize)> = app
        .webviews()
        .into_iter()
        .filter_map(|(label, webview)| {
            let (vtx, vrx) = std::sync::mpsc::channel();
            webview.with_webview(move |p| vtx.send(p.inner() as usize).unwrap()).ok()?;
            Some((label, vrx.recv().ok()?))
        })
        .collect();
    window
        .run_on_main_thread(move || unsafe {
            use objc2::runtime::AnyObject;
            let ns_window = &*(ns_window as *const AnyObject);
            let responder: *mut AnyObject = objc2::msg_send![ns_window, firstResponder];
            if responder.is_null() {
                let _ = tx.send(None);
                return;
            }
            let found = webviews.into_iter().find(|(_, view)| {
                let view = *view as *const AnyObject;
                let is_view: bool = objc2::msg_send![&*responder, isKindOfClass: objc2::class!(NSView)];
                is_view && objc2::msg_send![&*responder, isDescendantOf: &*view]
            });
            let _ = tx.send(found.map(|(label, _)| label));
        })
        .ok()?;
    rx.recv().ok().flatten()
}

/// A webview's frame (logical px, window coordinates) and whether it's
/// hidden, to check split view layouts.
#[derive(serde::Serialize)]
pub struct ViewFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    hidden: bool,
}

#[tauri::command]
pub async fn debug_view(app: AppHandle, label: String) -> Result<ViewFrame, String> {
    let webview = app.get_webview(&label).ok_or_else(|| format!("no webview {label}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    webview
        .with_webview(move |p| unsafe {
            let view = &*(p.inner() as *const objc2::runtime::AnyObject);
            let hidden: bool = objc2::msg_send![view, isHidden];
            let frame: objc2_foundation::NSRect = objc2::msg_send![view, frame];
            let _ = tx.send((frame, hidden));
        })
        .map_err(|e| e.to_string())?;
    let (frame, hidden) = rx.recv().map_err(|e| e.to_string())?;
    Ok(ViewFrame {
        x: frame.origin.x,
        y: frame.origin.y,
        width: frame.size.width,
        height: frame.size.height,
        hidden,
    })
}

/// Give a webview the keyboard, as a click into it would.
#[tauri::command]
pub async fn debug_focus(app: AppHandle, label: String) -> Result<(), String> {
    app.get_webview(&label)
        .ok_or_else(|| format!("no webview {label}"))?
        .set_focus()
        .map_err(|e| e.to_string())
}

/// Open a link the way macOS hands one over from another app.
#[tauri::command]
pub async fn debug_open_external(app: AppHandle, url: String) -> Result<(), String> {
    let url = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    crate::little::open_external(&app, url).map_err(|e| e.to_string())
}
