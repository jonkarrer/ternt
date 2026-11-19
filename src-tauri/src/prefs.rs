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

/// Terminal text size, in px (⌘+ / ⌘− / ⌘0 in a terminal).
pub const TERMINAL_FONT_SIZE: f64 = 13.0;
const MIN_TERMINAL_FONT_SIZE: f64 = 8.0;
const MAX_TERMINAL_FONT_SIZE: f64 = 36.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub external_links: LinkTarget,
    pub account: Account,
    pub terminal_font_size: f64,
    /// Unload tabs that have been off screen a while (see `unload`).
    pub unload_tabs: bool,
    /// Air Traffic Control: links from other apps that go to a given space
    /// (see `routing`).
    pub link_rules: Vec<crate::routing::LinkRule>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            external_links: LinkTarget::default(),
            account: Account::default(),
            terminal_font_size: TERMINAL_FONT_SIZE,
            unload_tabs: true,
            link_rules: Vec::new(),
        }
    }
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
    let Some(path) = path(app) else { return };
    if let crate::persist::Loaded::Read(prefs) = crate::persist::load(&path) {
        *app.state::<Store>().0.lock().unwrap() = prefs;
    }
}

pub fn get(app: &AppHandle) -> Prefs {
    app.state::<Store>().0.lock().unwrap().clone()
}

fn save(app: &AppHandle, prefs: &Prefs) {
    let Some(path) = path(app) else { return };
    let result = serde_json::to_vec_pretty(prefs)
        .map_err(std::io::Error::from)
        .and_then(|data| crate::persist::write(&path, &data, true));
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
    prefs.link_rules = prefs.link_rules.into_iter().filter_map(|r| r.tidy()).collect();
    prefs.terminal_font_size = if prefs.terminal_font_size.is_finite() {
        prefs
            .terminal_font_size
            .clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE)
    } else {
        TERMINAL_FONT_SIZE
    };
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
        assert_eq!(prefs.terminal_font_size, TERMINAL_FONT_SIZE);
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
