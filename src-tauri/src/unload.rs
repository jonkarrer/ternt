//! Unloading tabs you haven't looked at in a while, to save memory: each
//! loaded page keeps a WebKit process (often 50–150 MB). A web tab off
//! screen for `UNLOAD_AFTER` drops its view; it stays in the sidebar and
//! reloads when shown, like Safari and Chrome do. Tabs playing audio or
//! using the camera or microphone stay. Settings → General turns it off.
//!
//! `TERNT_UNLOAD_AFTER_SECS` shortens the wait (the e2e suite uses it).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager};

const UNLOAD_AFTER: Duration = Duration::from_secs(30 * 60);
/// How often tabs are looked over.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// When each web tab was last on screen (missing: not seen since launch, or
/// on screen now).
#[derive(Default)]
pub struct Seen(Mutex<HashMap<u32, Instant>>);

fn unload_after() -> Duration {
    std::env::var("TERNT_UNLOAD_AFTER_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(UNLOAD_AFTER, Duration::from_secs)
}

/// The tabs on screen changed: those that left it start their wait now.
pub fn on_shown(app: &AppHandle, before: &[u32], now_shown: &[u32]) {
    let seen = app.state::<Seen>();
    let mut seen = seen.0.lock().unwrap();
    let now = Instant::now();
    for id in before.iter().filter(|id| !now_shown.contains(id)) {
        seen.insert(*id, now);
    }
    for id in now_shown {
        seen.remove(id);
    }
}

/// Look the tabs over every so often, for as long as the app runs.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let after = unload_after();
    let every = SWEEP_EVERY.min(after / 2).max(Duration::from_millis(250));
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(every);
            if crate::prefs::get(&app).unload_tabs {
                sweep(&app, after);
            }
        }
    });
}

/// Unload the loaded web tabs that have been off screen for `after`.
fn sweep(app: &AppHandle, after: Duration) {
    let now = Instant::now();
    let candidates = crate::browser::background_pages(app);
    let due: Vec<u32> = {
        let seen = app.state::<Seen>();
        let mut seen = seen.0.lock().unwrap();
        // Forget tabs that closed or unloaded.
        seen.retain(|id, _| candidates.contains(id));
        candidates
            .iter()
            .filter(|id| {
                // Loaded in the background without ever being shown (at
                // launch, a link opened behind): its wait starts now.
                let since = *seen.entry(**id).or_insert(now);
                now.duration_since(since) >= after
            })
            .copied()
            .collect()
    };
    let mut unloaded = false;
    for id in due {
        if crate::browser::page_in_use(app, id) {
            continue;
        }
        match crate::browser::unload_background_tab(app, id) {
            Ok(true) => unloaded = true,
            Ok(false) => {}
            Err(err) => eprintln!("failed to unload tab {id}: {err}"),
        }
    }
    if unloaded {
        crate::browser::state_changed(app);
    }
}
