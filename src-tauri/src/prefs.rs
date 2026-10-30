//! App preferences (Settings), saved as `prefs.json` next to the session.
//! Small and read rarely: loaded at startup, written on each change.

use std::{path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager};

/// Where links opened from other apps go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkTarget {
    /// A small window of its own (Little Ternt), to glance at and close,
    /// or move into a tab.
    #[default]
    Little,
    /// A new tab in the active space.
    Tab,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub external_links: LinkTarget,
}

#[derive(Default)]
pub struct Store(Mutex<Prefs>);

/// `TERNT_PREFS_FILE` overrides the location (used by the e2e suite).
fn path(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TERNT_PREFS_FILE") {
        return Some(path.into());
    }
    Some(crate::session::data_dir(app)?.join("prefs.json"))
}

pub fn load(app: &AppHandle) {
    let Some(data) = path(app).and_then(|p| std::fs::read(p).ok()) else { return };
    match serde_json::from_slice(&data) {
        Ok(prefs) => *app.state::<Store>().0.lock().unwrap() = prefs,
        Err(err) => eprintln!("ignoring unreadable prefs: {err}"),
    }
}

pub fn get(app: &AppHandle) -> Prefs {
    app.state::<Store>().0.lock().unwrap().clone()
}

fn save(app: &AppHandle, prefs: &Prefs) {
    let Some(path) = path(app) else { return };
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(prefs)?)?;
        std::fs::rename(tmp, &path)
    })();
    if let Err(err) = result {
        eprintln!("failed to save prefs: {err}");
    }
}

#[tauri::command]
pub fn get_prefs(app: AppHandle) -> Prefs {
    get(&app)
}

#[tauri::command]
pub fn set_prefs(app: AppHandle, prefs: Prefs) {
    *app.state::<Store>().0.lock().unwrap() = prefs.clone();
    save(&app, &prefs);
    let _ = app.emit_to(EventTarget::webview(crate::settings::SETTINGS_WINDOW), "prefs:changed", prefs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let prefs: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.external_links, LinkTarget::Little);
        let prefs: Prefs = serde_json::from_str(r#"{"externalLinks":"tab","future":1}"#).unwrap();
        assert_eq!(prefs.external_links, LinkTarget::Tab);
    }
}
