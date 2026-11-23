//! Air Traffic Control (as in Arc): links from other apps go to the space a
//! rule names, as a new tab there, instead of where Settings → Links sends
//! the rest. Rules are kept in prefs and tried in order; the first that
//! matches wins.
//!
//! - A *host* rule (`github.com`, `github.com/acme`) matches that host and
//!   its subdomains, and with a path, URLs under it.
//! - A *regex* rule is tried against the whole URL.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleKind {
    Host,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRule {
    pub kind: RuleKind,
    pub pattern: String,
    /// The space its links open in.
    pub space: u32,
}

/// Longest pattern kept, in characters.
const MAX_PATTERN: usize = 500;

impl LinkRule {
    /// Trimmed, or `None` when there's nothing to match.
    pub fn tidy(mut self) -> Option<Self> {
        self.pattern = self.pattern.trim().chars().take(MAX_PATTERN).collect();
        (!self.pattern.is_empty()).then_some(self)
    }

    pub fn matches(&self, url: &Url) -> bool {
        match self.kind {
            RuleKind::Host => host_matches(&self.pattern, url),
            RuleKind::Regex => compile(&self.pattern).is_ok_and(|re| re.is_match(url.as_str())),
        }
    }
}

fn compile(pattern: &str) -> Result<regex::Regex, regex::Error> {
    regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .build()
}

/// `pattern` is a host, maybe with a path: `github.com`, `*.google.com`,
/// `https://github.com/acme/`. Case doesn't matter for the host.
fn host_matches(pattern: &str, url: &Url) -> bool {
    let pattern = pattern.split_once("://").map_or(pattern, |(_, rest)| rest);
    let pattern = pattern.trim_start_matches("*.");
    let (host, path) = match pattern.find('/') {
        Some(i) => (&pattern[..i], pattern[i..].trim_end_matches('/')),
        None => (pattern, ""),
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let Some(url_host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let url_host = url_host.strip_prefix("www.").unwrap_or(&url_host);
    let host_ok = !host.is_empty() && (url_host == host || url_host.ends_with(&format!(".{host}")));
    // `/acme` matches /acme and /acme/x, not /acmeco.
    let path_ok = path.is_empty() || {
        let rest = url.path().strip_prefix(path);
        rest.is_some_and(|r| r.is_empty() || r.starts_with('/'))
    };
    host_ok && path_ok
}

/// The space the first matching rule names (rules for deleted spaces are
/// skipped), and the rule's index.
pub fn route(app: &AppHandle, url: &Url) -> Option<(usize, u32)> {
    let rules = crate::prefs::get(app).link_rules;
    let spaces = crate::browser::space_ids(app);
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| spaces.contains(&rule.space) && rule.matches(url))
        .map(|(i, rule)| (i, rule.space))
}

/// Where a link would go, for Settings' tester.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// The matching rule's index.
    rule: usize,
    space: u32,
}

#[tauri::command]
pub fn route_link(app: AppHandle, url: String) -> Option<Route> {
    let url = Url::parse(url.trim())
        .or_else(|_| Url::parse(&format!("https://{}", url.trim())))
        .ok()?;
    route(&app, &url).map(|(rule, space)| Route { rule, space })
}

/// Why a regex won't compile, for Settings.
#[tauri::command]
pub fn check_link_rule(kind: RuleKind, pattern: String) -> Result<(), String> {
    match kind {
        RuleKind::Host => Ok(()),
        RuleKind::Regex => compile(pattern.trim()).map(|_| ()).map_err(|e| {
            // The last line says what's wrong; the rest repeats the pattern.
            let line = e
                .to_string()
                .lines()
                .last()
                .unwrap_or_default()
                .trim()
                .to_string();
            line.strip_prefix("error: ")
                .map(str::to_string)
                .unwrap_or(line)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: RuleKind, pattern: &str) -> LinkRule {
        LinkRule {
            kind,
            pattern: pattern.into(),
            space: 1,
        }
    }

    fn hit(rule: &LinkRule, url: &str) -> bool {
        rule.matches(&Url::parse(url).unwrap())
    }

    #[test]
    fn hosts_match_themselves_and_their_subdomains() {
        let r = rule(RuleKind::Host, "github.com");
        assert!(hit(&r, "https://github.com/x"));
        assert!(hit(&r, "https://www.github.com/"));
        assert!(hit(&r, "https://gist.github.com/x"));
        assert!(hit(&r, "https://GitHub.com/x"));
        assert!(!hit(&r, "https://notgithub.com/"));
        assert!(!hit(&r, "https://github.com.evil.io/"));
        for pattern in [
            "*.github.com",
            "https://github.com/",
            "www.github.com",
            "GitHub.com",
        ] {
            assert!(
                hit(&rule(RuleKind::Host, pattern), "https://api.github.com/"),
                "{pattern}"
            );
        }
    }

    #[test]
    fn a_path_narrows_a_host_rule() {
        let r = rule(RuleKind::Host, "github.com/acme/");
        assert!(hit(&r, "https://github.com/acme"));
        assert!(hit(&r, "https://github.com/acme/repo/pull/1"));
        assert!(!hit(&r, "https://github.com/acmeco"));
        assert!(!hit(&r, "https://github.com/other/acme"));
    }

    #[test]
    fn regexes_match_the_whole_url() {
        let r = rule(RuleKind::Regex, r"^https://docs\.google\.com/.*/d/1abc");
        assert!(hit(&r, "https://docs.google.com/document/d/1abcdef/edit"));
        assert!(!hit(&r, "https://docs.google.com/document/d/9zzz/edit"));
        assert!(hit(
            &rule(RuleKind::Regex, "(?i)LINEAR"),
            "https://linear.app/acme"
        ));
        assert!(!hit(
            &rule(RuleKind::Regex, "(unclosed"),
            "https://a.com/(unclosed"
        ));
    }

    #[test]
    fn bad_regexes_say_why() {
        assert!(check_link_rule(RuleKind::Regex, "(unclosed".into()).is_err());
        assert!(check_link_rule(RuleKind::Regex, r"^https://a\.com".into()).is_ok());
        assert!(check_link_rule(RuleKind::Host, "(anything".into()).is_ok());
    }

    #[test]
    fn blank_rules_are_dropped() {
        assert_eq!(rule(RuleKind::Host, "  ").tidy(), None);
        assert_eq!(
            rule(RuleKind::Host, " a.com ").tidy().unwrap().pattern,
            "a.com"
        );
    }
}
