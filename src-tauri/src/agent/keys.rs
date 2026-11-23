//! The user's API keys for the agents: bring your own. They live in the macOS Keychain, never
//! in a file or the repo, and reach the agent only through the sidecar's environment (set on
//! the process in `sidecar.rs`; see docs/AGENT-SIDECAR.md). The process gets every key the
//! user has; which one a space's agent uses follows from that space's provider
//! (`setup.rs`), and a tool's key (Exa, for web search) serves every space with that tool on. A variable already in Ternt's own environment wins: development
//! (`doppler run -- just dev tokidoki`), where a rebuilt debug binary would otherwise be asked
//! to confirm Keychain access at every launch.

use keyring::Entry;
use serde::Serialize;

/// The Keychain service the keys are filed under. Debug builds keep their own, so trying a
/// key in `just dev` never replaces the one the installed app uses.
const SERVICE: &str = if cfg!(debug_assertions) {
    "com.jokar.ternt.dev"
} else {
    "com.jokar.ternt"
};

/// One key: the variable the sidecar reads it from, and its Keychain account.
pub struct Key {
    pub variable: &'static str,
    account: &'static str,
}

/// Answers and memory, through Fireworks AI (the default provider).
pub const FIREWORKS: Key = Key {
    variable: "FIREWORKS_API_KEY",
    account: "fireworks",
};
/// Answers and memory, through Anthropic directly.
pub const ANTHROPIC: Key = Key {
    variable: "ANTHROPIC_API_KEY",
    account: "anthropic",
};
/// Web search, for the spaces that have that tool on (Exa).
pub const EXA: Key = Key {
    variable: "EXA_API_KEY",
    account: "exa",
};

/// Where a key is, if anywhere.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    #[default]
    None,
    Keychain,
    /// Ternt's own environment (development); Settings can't remove it.
    Environment,
}

impl Key {
    /// The key and where it came from; `None` when neither place has it.
    pub fn read(&self) -> Result<Option<(String, Source)>, String> {
        if let Some(value) = std::env::var(self.variable).ok().filter(|v| !v.is_empty()) {
            return Ok(Some((value, Source::Environment)));
        }
        match Entry::new(SERVICE, self.account)
            .map_err(|e| e.to_string())?
            .get_password()
        {
            Ok(value) if !value.is_empty() => Ok(Some((value, Source::Keychain))),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("reading the Keychain: {error}")),
        }
    }

    /// Store the key; an empty value removes the stored one.
    pub fn write(&self, value: &str) -> Result<(), String> {
        let entry = Entry::new(SERVICE, self.account).map_err(|e| e.to_string())?;
        let written = match value {
            "" => match entry.delete_credential() {
                Err(keyring::Error::NoEntry) => Ok(()),
                other => other,
            },
            value => entry.set_password(value),
        };
        written.map_err(|error| format!("writing to the Keychain: {error}"))
    }
}

/// The sidecar's environment: every key the user has, as (variable, value). A key that can't
/// be read is logged and left out, so the process still starts.
pub fn load() -> Vec<(&'static str, String)> {
    [&FIREWORKS, &ANTHROPIC, &EXA]
        .into_iter()
        .filter_map(|key| match key.read() {
            Ok(value) => value.map(|(value, _)| (key.variable, value)),
            Err(error) => {
                eprintln!("ternt-agent: {}: {error}", key.variable);
                None
            }
        })
        .collect()
}

/// Where each key is, never its value: what Settings shows.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Present {
    pub fireworks: Source,
    pub anthropic: Source,
    pub exa: Source,
}

pub fn present() -> Present {
    let source = |key: &Key| {
        key.read()
            .ok()
            .flatten()
            .map(|(_, source)| source)
            .unwrap_or_default()
    };
    Present {
        fireworks: source(&FIREWORKS),
        anthropic: source(&ANTHROPIC),
        exa: source(&EXA),
    }
}
