//! `tokidoki-serve --data <dir>`: tokidoki as a sidecar process (`tokidoki::serve`).
//!
//! Each space's agent keeps its files in `<dir>/<space>`. Whose models they run on comes from
//! the environment: `TOKIDOKI_PROVIDER` (`anthropic`, the default, `fireworks` or `baseten`)
//! and that provider's key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`);
//! `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` to run other models than the provider's
//! usual. stdout is the protocol and nothing else; what would be logged goes to stderr.

use std::ops::Not;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use tokidoki::serve::{self, Open};
use tokidoki::{Config, Spaces, Tokidoki, Tools};
use tokio::io::BufReader;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() {
    let served = run().await;
    if let Err(error) = &served {
        eprintln!("tokidoki: {error:#}");
    }
    // Not by returning: the runtime would wait for the thread still reading stdin, which the
    // host keeps open until this process is gone.
    std::process::exit(served.is_err().into());
}

async fn run() -> Result<()> {
    let data = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [flag, dir] if flag == "--data" => PathBuf::from(dir),
        _ => bail!("usage: tokidoki-serve --data <dir>"),
    };
    std::fs::create_dir_all(&data).with_context(|| format!("creating {}", data.display()))?;
    // Two processes on the same files would each run the other's unfinished work.
    let lock = std::fs::File::create(data.join(".lock"))?;
    if lock.try_lock().is_err() {
        bail!("another tokidoki is serving {}", data.display());
    }

    // Read once: the environment is the host's settings as they were when it started this.
    let provider = Config::provider_from_env()?;
    let key = std::env::var(provider.key_var())
        .ok()
        .filter(|key| !key.is_empty());
    let config =
        move |data: &Path| Config::new(data, provider, key.clone().unwrap_or_default()).with_env();
    let has_key = config(&data).key.is_empty().not();
    let about = serde_json::json!({
        "provider": provider.id(),
        "model": config(&data).model,
        "has_key": has_key,
    });
    // Without the key an agent runs nothing, and what it has can still be read.
    let open = move |data: &Path| match has_key {
        true => Tokidoki::open(config(data), Tools::new()),
        false => Tokidoki::open_asleep(config(data), format!("{} is not set", provider.key_var())),
    };

    let spaces = Arc::new(Spaces::new(&data));
    let serving = serve::run(
        spaces.clone(),
        Arc::new(open) as Open,
        about,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    );
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        served = serving => served,
        // The program is going, and kills this process shortly: stop writing now.
        _ = terminate.recv() => {
            (tokio::time::timeout(serve::STOP_WITHIN, spaces.close_all()).await).unwrap_or(Ok(()))
        }
    }
}
