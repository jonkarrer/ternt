//! The user's API keys for the agents. They live in the macOS Keychain, never in a file or
//! the repo, and reach the agent only through the sidecar's environment (set on the process
//! in `sidecar.rs`; see docs/AGENT-SIDECAR.md). A variable already in Ternt's own
//! environment wins: development (`ANTHROPIC_API_KEY=… just dev`), where a rebuilt debug
//! binary would otherwise be asked to confirm Keychain access at every launch.

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

/// Answers and memory, through Fireworks AI. One of this and `ANTHROPIC` is required.
pub const FIREWORKS: Key = Key {
    variable: "FIREWORKS_API_KEY",
    account: "fireworks",
};
/// Answers and memory, through Anthropic directly.
pub const ANTHROPIC: Key = Key {
    variable: "ANTHROPIC_API_KEY",
    account: "anthropic",
};
/// The router's fallback model. Optional.
pub const BASETEN: Key = Key {
    variable: "BASETEN_API_KEY",
    account: "baseten",
};
pub const ALL: [Key; 3] = [FIREWORKS, ANTHROPIC, BASETEN];
/// The keys that let the agent answer at all; one is enough.
pub const ANSWERING: [&str; 2] = [FIREWORKS.variable, ANTHROPIC.variable];

impl Key {
    /// The key, from the environment or the Keychain; `None` when neither has it.
    pub fn read(&self) -> Result<Option<String>, String> {
        if let Some(value) = std::env::var(self.variable).ok().filter(|v| !v.is_empty()) {
            return Ok(Some(value));
        }
        match Entry::new(SERVICE, self.account)
            .map_err(|e| e.to_string())?
            .get_password()
        {
            Ok(value) if !value.is_empty() => Ok(Some(value)),
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

/// Every key that is set, as (variable, value): the sidecar's environment. A key that
/// can't be read is logged and left out, so the process still starts.
pub fn load() -> Vec<(&'static str, String)> {
    ALL.iter()
        .filter_map(|key| match key.read() {
            Ok(value) => value.map(|value| (key.variable, value)),
            Err(error) => {
                eprintln!("ternt-agent: {}: {error}", key.variable);
                None
            }
        })
        .collect()
}

/// Which keys are set, never their values: what the panel shows.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Present {
    pub fireworks: bool,
    pub anthropic: bool,
    pub baseten: bool,
}

pub fn present() -> Present {
    let set = |key: &Key| key.read().ok().flatten().is_some();
    Present {
        fireworks: set(&FIREWORKS),
        anthropic: set(&ANTHROPIC),
        baseten: set(&BASETEN),
    }
}
