//! How a space's agent runs: its provider and its tools, the user's choice
//! per space (Settings → Agent). The setup is kept with the space in the
//! session and told to the sidecar with `configure` before anything else is
//! asked of that space; the keys are shared (`keys.rs`), one per provider.

use serde::{Deserialize, Serialize};

/// Who answers for a space. Bring your own key: the one for this provider is
/// what the agent uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Fireworks,
    Anthropic,
}

impl Provider {
    /// As the sidecar is told.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fireworks => "fireworks",
            Self::Anthropic => "anthropic",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    #[serde(default)]
    pub provider: Provider,
    /// Tool names the sidecar knows (none offered yet).
    #[serde(default)]
    pub tools: Vec<String>,
}

/// How a space's agent runs now, as the sidecar answers `configure`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceStatus {
    pub provider: String,
    /// The model that answers, when the process says (tokidoki does).
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// The process has the provider's key; without it the agent reads but can't answer.
    #[serde(alias = "has_key")]
    pub has_key: bool,
    /// Why it can't answer, when it can't.
    #[serde(default)]
    pub asleep: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_without_a_setup_runs_on_fireworks_with_no_tools() {
        let setup: Setup = serde_json::from_str("{}").unwrap();
        assert_eq!(setup, Setup::default());
        assert_eq!(setup.provider, Provider::Fireworks);
        let setup: Setup =
            serde_json::from_str(r#"{"provider":"anthropic","tools":["web_search"]}"#).unwrap();
        assert_eq!(setup.provider.as_str(), "anthropic");
        assert_eq!(setup.tools, ["web_search"]);
    }

    #[test]
    fn the_sidecars_answer_is_read_as_it_writes_it() {
        let status: SpaceStatus = serde_json::from_str(
            r#"{"provider":"fireworks","model":"m","tools":[],"has_key":false,"asleep":"FIREWORKS_API_KEY is not set"}"#,
        )
        .unwrap();
        assert!(!status.has_key);
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["hasKey"], false);
        assert_eq!(json["asleep"], "FIREWORKS_API_KEY is not set");
    }
}
