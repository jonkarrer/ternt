//! `tokidoki-serve --data <dir>`: tokidoki as a sidecar process (`tokidoki::serve`).
//!
//! Each space's agent keeps its files in `<dir>/<space>`. The host says per space, with
//! `configure`, whose models it runs on and which tools it has; the keys are the process's,
//! from its environment: `ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`, whichever
//! the host has. A space never configured runs on `TOKIDOKI_PROVIDER` (`anthropic` unless
//! set). `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` run other models than a provider's
//! usual. stdout is the protocol and nothing else; what would be logged goes to stderr.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use tokidoki::serve::{self, Open};
use tokidoki::{Config, Setup, Spaces, Tokidoki, Tools, Vendor};
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

/// The keys the host gave, one per provider it has one for.
struct Keys(Vec<(Vendor, String)>);

impl Keys {
    /// Read once: the environment is the host's settings as they were when it started this.
    fn from_env() -> Self {
        let keys = Vendor::ALL.into_iter().filter_map(|vendor| {
            let key = std::env::var(vendor.key_var()).ok()?;
            (!key.is_empty()).then_some((vendor, key))
        });
        Self(keys.collect())
    }

    fn get(&self, vendor: Vendor) -> Option<&str> {
        self.0
            .iter()
            .find(|(v, _)| *v == vendor)
            .map(|(_, key)| key.as_str())
    }
}

/// The process's tools by name, those `setup.tools` asks for. None yet.
fn tools(setup: &Setup) -> Result<Tools> {
    if let Some(name) = setup.tools.first() {
        bail!("unknown tool {name}");
    }
    Ok(Tools::new())
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

    let keys = Keys::from_env();
    let default = Setup {
        provider: Config::provider_from_env()?,
        tools: Vec::new(),
    };
    let providers: serde_json::Map<String, serde_json::Value> = Vendor::ALL
        .iter()
        .map(|v| (v.id().to_string(), keys.get(*v).is_some().into()))
        .collect();
    let about = serde_json::json!({
        "providers": providers,
        "default_provider": default.provider.id(),
        "tools": {},
    });
    // Only the provider, tools and model settings are the host's to pass; the user, speakers
    // and system prompt are tokidoki's own (`chat` reads them, a host's environment shouldn't).
    // Without the provider's key an agent runs nothing, and what it has can still be read.
    let open = move |data: &Path, setup: &Setup| {
        let key = keys.get(setup.provider);
        let config = Config::new(data, setup.provider, key.unwrap_or_default().to_string())
            .with_models_from_env();
        match key {
            Some(_) => Tokidoki::open(config, tools(setup)?),
            None => {
                Tokidoki::open_asleep(config, format!("{} is not set", setup.provider.key_var()))
            }
        }
    };

    let spaces = Arc::new(Spaces::new(&data).with_default(default));
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
            (tokio::time::timeout(serve::STOP_BUDGET, spaces.close_all()).await).unwrap_or(Ok(()))
        }
    }
}
