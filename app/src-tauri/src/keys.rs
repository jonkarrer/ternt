//! The user's own API keys. They live in the macOS Keychain, never in a file; a key in the
//! environment wins over it (`ANTHROPIC_API_KEY`, `BASETEN_API_KEY`: development, where the
//! repo's `.env` already has them).

use anyhow::{Context as _, Result};
use keyring::Entry;

/// The Keychain service the keys are filed under. Development builds keep their own, so
/// trying a key in `just app` never replaces the one the installed app uses.
const SERVICE: &str = if cfg!(debug_assertions) {
    "com.jokar.tokidoki.dev"
} else {
    "com.jokar.tokidoki"
};
const ANTHROPIC: &str = "anthropic";
const BASETEN: &str = "baseten";

#[derive(Debug, Clone, Default)]
pub struct Keys {
    /// Answers and memory. Required.
    pub anthropic: String,
    /// The router's last resort, a small fast model. Empty when the user has none: inputs
    /// the router can't place in code then go to the thread it would have guessed.
    pub baseten: String,
}

/// The keys, or `None` until there is an Anthropic key.
pub fn load() -> Result<Option<Keys>> {
    let Some(anthropic) = read("ANTHROPIC_API_KEY", ANTHROPIC)? else {
        return Ok(None);
    };
    Ok(Some(Keys {
        anthropic,
        baseten: read("BASETEN_API_KEY", BASETEN)?.unwrap_or_default(),
    }))
}

/// Stores the keys. An empty Baseten key removes the stored one.
pub fn save(keys: &Keys) -> Result<()> {
    write(ANTHROPIC, &keys.anthropic)?;
    write(BASETEN, &keys.baseten)
}

fn read(variable: &str, account: &str) -> Result<Option<String>> {
    if let Some(key) = std::env::var(variable).ok().filter(|key| !key.is_empty()) {
        return Ok(Some(key));
    }
    match Entry::new(SERVICE, account)?.get_password() {
        Ok(key) if !key.is_empty() => Ok(Some(key)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context("reading the Keychain"),
    }
}

fn write(account: &str, key: &str) -> Result<()> {
    let entry = Entry::new(SERVICE, account)?;
    let written = match key {
        "" => match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        },
        key => entry.set_password(key),
    };
    written.context("writing to the Keychain")
}
