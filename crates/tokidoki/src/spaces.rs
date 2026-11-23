//! Several tokidokis in one process, each on its own: a program with separate places to work
//! (a browser's spaces, say) gives each its own, which learns only what happens there.
//!
//! Each lives in its own folder under one root, holding its state, memory and journal files
//! and nothing shared: removing the folder removes all of it. Names are the embedding
//! program's (an ID it already has); they become folder names, so only lowercase letters,
//! digits, `-` and `_` are taken: no two names are one folder, whatever the file system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result};
use inference::Vendor;
use interface::Rejected;

use crate::{Config, Tokidoki, Tools};

/// How a space's agent is to run, the host's choice for that space (`configure`): whose
/// models, and which of the process's tools. Keys are the process's, one per provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub provider: Vendor,
    /// Tool names, as the process knows them.
    pub tools: Vec<String>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            provider: Vendor::Anthropic,
            tools: Vec::new(),
        }
    }
}

/// The open tokidokis, by name; cheap to share behind an `Arc`.
pub struct Spaces {
    root: PathBuf,
    open: Mutex<HashMap<String, Tokidoki>>,
    /// What each configured space runs as; `default` for the others.
    setups: Mutex<HashMap<String, Setup>>,
    default: Setup,
    /// Held while one is opened, closed or deleted: a space is never opened while it (or its
    /// folder) is still going away, nor twice at once.
    changing: tokio::sync::Mutex<()>,
    /// `close_all` was called: nothing opens again. A request still in flight at shutdown
    /// would otherwise reopen its space after the drain and leave work running.
    closing: AtomicBool,
}

impl Spaces {
    /// Tokidokis kept under `root`, one folder each. Opens none.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            open: Mutex::default(),
            setups: Mutex::default(),
            default: Setup::default(),
            changing: tokio::sync::Mutex::default(),
            closing: AtomicBool::new(false),
        }
    }

    /// What a space runs as until `configure` says otherwise.
    pub fn with_default(mut self, setup: Setup) -> Self {
        self.default = setup;
        self
    }

    /// How `name` runs, or would: what `configure` last said, else the default.
    pub fn setup(&self, name: &str) -> Setup {
        let setups = self.setups.lock().unwrap_or_else(PoisonError::into_inner);
        setups
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Tokidoki>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where `name`'s files are (or would be).
    fn data(&self, name: &str) -> Result<PathBuf> {
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'));
        match valid {
            true => Ok(self.root.join(name)),
            false => Err(Rejected(format!("{name:?} can't name a space")).into()),
        }
    }

    /// The tokidoki `name` if it is open.
    pub fn get(&self, name: &str) -> Option<Tokidoki> {
        let open = self.lock().get(name).cloned();
        open.filter(|tokidoki| !tokidoki.is_closed())
    }

    /// The tokidoki `name`: the open one, or opened now with `config` (made for its data
    /// folder) and `tools`, resuming what it left unfinished. A new name starts empty.
    pub async fn open(
        &self,
        name: &str,
        config: impl FnOnce(&Path) -> Config,
        tools: Tools,
    ) -> Result<Tokidoki> {
        let open = |data: &Path, _: &Setup| {
            let mut config = config(data);
            config.data = data.to_path_buf();
            Tokidoki::open(config, tools)
        };
        self.open_with(name, open).await
    }

    /// The tokidoki `name`: the open one, or what `open` makes of its data folder and its
    /// setup now. `open` runs only when there is none open, and once.
    pub async fn open_with(
        &self,
        name: &str,
        open: impl FnOnce(&Path, &Setup) -> Result<Tokidoki>,
    ) -> Result<Tokidoki> {
        self.data(name)?;
        // The usual case first, without waiting behind another space's close or delete.
        if let Some(tokidoki) = self.get(name) {
            return Ok(tokidoki);
        }
        let _changing = self.changing.lock().await;
        self.open_locked(name, open)
    }

    /// `open_with` with `changing` held: opens `name` as its setup says, unless it is.
    fn open_locked(
        &self,
        name: &str,
        open: impl FnOnce(&Path, &Setup) -> Result<Tokidoki>,
    ) -> Result<Tokidoki> {
        let data = self.data(name)?;
        if let Some(tokidoki) = self.get(name) {
            return Ok(tokidoki);
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(Rejected("shutting down".into()).into());
        }
        let tokidoki = open(&data, &self.setup(name))?;
        self.lock().insert(name.to_string(), tokidoki.clone());
        Ok(tokidoki)
    }

    /// Has `name` run as `setup` from now on, and returns it running so: as it is if it
    /// already does, else stopped and opened again (what it was doing resumes under the new
    /// provider). Remembered for the process's life: a later open uses it too.
    pub async fn configure(
        &self,
        name: &str,
        setup: Setup,
        open: impl FnOnce(&Path, &Setup) -> Result<Tokidoki>,
    ) -> Result<Tokidoki> {
        self.data(name)?;
        let _changing = self.changing.lock().await;
        if self.setup(name) != setup {
            self.setups
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(name.to_string(), setup);
            self.stop(name).await?;
        }
        self.open_locked(name, open)
    }

    /// Stops `name` if it is open: its work resumes when it is opened again.
    pub async fn close(&self, name: &str) -> Result<()> {
        let _changing = self.changing.lock().await;
        self.stop(name).await
    }

    async fn stop(&self, name: &str) -> Result<()> {
        let open = self.lock().remove(name);
        match open {
            Some(tokidoki) => tokidoki.close().await,
            None => Ok(()),
        }
    }

    /// Stops every open tokidoki, and lets none open again: before the program exits.
    pub async fn close_all(&self) -> Result<()> {
        self.closing.store(true, Ordering::Release);
        let _changing = self.changing.lock().await;
        let open: Vec<Tokidoki> = self.lock().drain().map(|(_, tokidoki)| tokidoki).collect();
        // All at once: each stops committing right away, however long another takes to stop.
        let closing: Vec<_> = open
            .into_iter()
            .map(|tokidoki| tokio::spawn(async move { tokidoki.close().await }))
            .collect();
        // Every one is closed, whatever the others did; the first failure is reported.
        let mut closed = Ok(());
        for closing in closing {
            closed = closed.and(closing.await.context("closing a space")?);
        }
        closed
    }

    /// Stops `name` and deletes everything it has: its conversations, its memory, its
    /// journal. There is no undoing it.
    pub async fn delete(&self, name: &str) -> Result<()> {
        let data = self.data(name)?;
        let _changing = self.changing.lock().await;
        self.stop(name).await?;
        self.setups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        match std::fs::remove_dir_all(&data) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("deleting {}", data.display()))
            }
            _ => Ok(()),
        }
    }
}
