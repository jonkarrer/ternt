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

/// Who uses this Ternt (Settings → Account). Kept on this Mac only; the
/// profiles hold the actual sign-ins.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub name: String,
    pub email: String,
}

/// Longest name or email kept, in characters.
const MAX_ACCOUNT_FIELD: usize = 200;

impl Account {
    fn tidy(self) -> Self {
        let tidy = |s: String| s.trim().chars().take(MAX_ACCOUNT_FIELD).collect();
        Self {
            name: tidy(self.name),
            email: tidy(self.email),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub external_links: LinkTarget,
    pub account: Account,
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
    let Some(data) = path(app).and_then(|p| std::fs::read(p).ok()) else {
        return;
    };
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
pub fn set_prefs(app: AppHandle, mut prefs: Prefs) {
    prefs.account = prefs.account.tidy();
    *app.state::<Store>().0.lock().unwrap() = prefs.clone();
    save(&app, &prefs);
    let _ = app.emit_to(
        EventTarget::webview(crate::settings::SETTINGS_WINDOW),
        "prefs:changed",
        prefs,
    );
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
        assert_eq!(prefs.account, Account::default());
    }

    #[test]
    fn account_fields_are_trimmed_and_capped() {
        let account = Account {
            name: "  Jo  ".into(),
            email: "x".repeat(500),
        }
        .tidy();
        assert_eq!(account.name, "Jo");
        assert_eq!(account.email.len(), MAX_ACCOUNT_FIELD);
    }
}
