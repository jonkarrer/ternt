//! What a feature that is gone left on this Mac, cleaned up at launch.
//!
//! Ternt had an AI agent until v0.23: its API keys sat in the Keychain and each space's
//! conversations and memories in `agents/` under the data folder. Nothing in Ternt shows
//! or uses either any more, so both are deleted (the owner's decision, 2026-11-25).
//! Once every copy has run a version with this, the module and `keyring` can go.

use tauri::AppHandle;

const SERVICE: &str = if cfg!(debug_assertions) {
    "com.jokar.ternt.dev"
} else {
    "com.jokar.ternt"
};
/// The Keychain accounts the agent's keys were kept under.
const ACCOUNTS: [&str; 4] = ["fireworks", "anthropic", "exa", "typesafe"];
const FOLDER: &str = "agents";

/// Delete the agent's keys and files, off the main thread: the Keychain can be slow.
pub fn clean(app: &AppHandle) {
    let folder = crate::session::data_dir(app).map(|dir| dir.join(FOLDER));
    std::thread::spawn(move || {
        for account in ACCOUNTS {
            match keyring::Entry::new(SERVICE, account).and_then(|entry| entry.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(err) => eprintln!("couldn't delete the old {account} key: {err}"),
            }
        }
        if let Some(folder) = folder {
            remove_folder(&folder);
        }
    });
}

/// A folder that isn't there is already removed.
fn remove_folder(folder: &std::path::Path) {
    match std::fs::remove_dir_all(folder) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => eprintln!("couldn't remove {}: {err}", folder.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_goes_with_what_is_in_it_and_a_missing_one_is_fine() {
        let folder = std::env::temp_dir().join(format!("ternt-retired-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("space-1")).unwrap();
        std::fs::write(folder.join("space-1/memory.db"), b"x").unwrap();
        remove_folder(&folder);
        assert!(!folder.exists());
        remove_folder(&folder);
    }
}
