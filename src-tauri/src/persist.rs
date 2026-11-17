//! Saving and loading the JSON files Ternt keeps (session, history, prefs).
//!
//! Writes go to a temp file of their own, flushed to disk, then renamed over
//! the file: a crash or power loss mid-save leaves the old file whole, and
//! two saves at once (a debounced one and the one on quit) can't mix. A file
//! that can't be read is moved aside rather than overwritten by the next
//! save, so nothing is lost for good to a bad write or a bug.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::de::DeserializeOwned;

/// One save at a time, so the last one to start is the one that lands.
static SAVING: Mutex<()> = Mutex::new(());

/// Where `path`'s previous version is kept (see [`write`] with `backup`).
pub fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Write `data` to `path` atomically. With `backup`, the file it replaces
/// is kept as `<name>.bak`, to fall back to if the new one ever can't be read.
pub fn write(path: &Path, data: &[u8], backup: bool) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let _saving = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(
        path,
        &format!(".tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)),
    );
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        if backup && path.exists() {
            std::fs::copy(path, backup_path(path))?;
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// What reading a saved file found.
pub enum Loaded<T> {
    /// Nothing saved yet.
    Missing,
    Read(T),
    /// The file (and its backup, if any) couldn't be read; it was moved aside.
    Unreadable,
}

/// Read `path`, falling back to its backup (see [`write`]) if it's damaged.
/// A file that can't be read is renamed `<name>.unreadable-<time>`, so the
/// next save doesn't overwrite what might still be recovered from it.
pub fn load<T: DeserializeOwned>(path: &Path) -> Loaded<T> {
    // No file is a fresh start (someone may have deleted it on purpose):
    // the backup is only for a file that's there but damaged.
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(err) => {
            eprintln!("can't read {}: {err}", path.display());
            return Loaded::Unreadable;
        }
    };
    match serde_json::from_slice(&data) {
        Ok(value) => return Loaded::Read(value),
        Err(err) => {
            eprintln!("{} is damaged ({err}); setting it aside", path.display());
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let _ = std::fs::rename(path, with_suffix(path, &format!(".unreadable-{stamp}")));
        }
    }
    match std::fs::read(backup_path(path)).ok().map(|d| serde_json::from_slice(&d)) {
        Some(Ok(value)) => {
            eprintln!("restored {} from its backup", path.display());
            Loaded::Read(value)
        }
        _ => Loaded::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ternt-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn save(path: &Path, value: &Vec<u32>, backup: bool) -> std::io::Result<()> {
        write(path, &serde_json::to_vec(value).unwrap(), backup)
    }

    #[test]
    fn saves_and_loads() {
        let path = dir("round").join("a.json");
        assert!(matches!(load::<Vec<u32>>(&path), Loaded::Missing));
        save(&path, &vec![1, 2], false).unwrap();
        assert!(matches!(load::<Vec<u32>>(&path), Loaded::Read(v) if v == [1, 2]));
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn a_damaged_file_falls_back_to_its_backup_and_is_kept() {
        let path = dir("damaged").join("session.json");
        save(&path, &vec![1], true).unwrap();
        save(&path, &vec![1, 2], true).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(matches!(load::<Vec<u32>>(&path), Loaded::Read(v) if v == [1]));
        let kept = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("session.json.unreadable-"));
        assert!(kept, "the damaged file is set aside, not deleted");
    }

    #[test]
    fn a_deleted_file_starts_fresh_even_with_a_backup() {
        let path = dir("deleted").join("prefs.json");
        save(&path, &vec![1], true).unwrap();
        save(&path, &vec![2], true).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(load::<Vec<u32>>(&path), Loaded::Missing));
    }

    #[test]
    fn a_damaged_file_without_a_backup_is_unreadable_not_missing() {
        let path = dir("nobackup").join("history.json");
        std::fs::write(&path, b"garbage").unwrap();
        assert!(matches!(load::<Vec<u32>>(&path), Loaded::Unreadable));
    }
}
