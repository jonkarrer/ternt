//! `web_search`: the web through Exa (<https://exa.ai>). One request per call; the model gets
//! each page's title, address, date and the passages Exa found most relevant to the query,
//! which is enough to answer from and to cite, and far smaller than the pages.

use std::fmt::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent::{Replay, Tool, ToolFuture};
use anyhow::{Context as _, bail};
use serde::Deserialize;
use serde_json::{Value, json};

const EXA: &str = "https://api.exa.ai";
/// A search that hasn't answered by now won't make the reply better by being waited for.
const TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_RESULTS: u64 = 5;
const MAX_RESULTS: u64 = 10;
/// Per passage, in characters: a page that is one long line shouldn't fill the context.
const MAX_PASSAGE: usize = 600;

pub struct WebSearch {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl WebSearch {
    /// What a space switches it on by.
    pub const NAME: &str = "web_search";
    /// The environment variable its key comes in.
    pub const KEY_VAR: &str = "EXA_API_KEY";

    pub fn new(key: impl Into<String>) -> Self {
        Self::at(EXA, key)
    }

    /// Against another server than Exa's: a test's.
    pub fn at(base: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into(),
            key: key.into(),
        }
    }
}

#[derive(Deserialize)]
struct Arguments {
    query: String,
    num_results: Option<u64>,
    days: Option<u64>,
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    results: Vec<Page>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    title: Option<String>,
    url: String,
    published_date: Option<String>,
    #[serde(default)]
    highlights: Vec<String>,
}

impl Tool for WebSearch {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: Self::NAME.into(),
            description: "Search the web. Use it for anything recent, anything you are unsure \
                          of, and whenever the user asks you to look something up. Returns the \
                          most relevant pages with their address, date and key passages; cite \
                          the addresses you rely on."
                .into(),
            // Ranges are said, not declared: some providers refuse numeric bounds in a schema.
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to look for, as a full natural-language question or description."
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "How many pages to return, 1 to 10. 5 unless said."
                    },
                    "days": {
                        "type": "integer",
                        "description": "Only pages published in the last this many days. Leave out unless recency matters."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    /// A search reads; cut off by a crash, it is simply made again.
    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            let asked: Arguments =
                serde_json::from_value(arguments).context("web_search takes a query")?;
            let query = asked.query.trim();
            if query.is_empty() {
                bail!("web_search takes a query");
            }
            let mut body = json!({
                "query": query,
                "type": "auto",
                "numResults": asked.num_results.unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS),
                "contents": { "highlights": true },
            });
            if let Some(days) = asked.days.filter(|days| *days > 0) {
                body["startPublishedDate"] = days_ago(SystemTime::now(), days).into();
            }
            let response = self
                .http
                .post(format!("{}/search", self.base))
                .header("x-api-key", &self.key)
                .timeout(TIMEOUT)
                .json(&body)
                .send()
                .await
                // Without the address: an error is shown to the model and kept in the journal.
                .map_err(|error| {
                    anyhow::anyhow!("the search didn't answer: {}", error.without_url())
                })?;
            let status = response.status();
            if !status.is_success() {
                let said = response.text().await.unwrap_or_default();
                bail!("the search failed ({status}): {}", cut(said.trim(), 300));
            }
            let found: Found = response
                .json()
                .await
                .context("the search answered with something unreadable")?;
            Ok(shown(query, &found.results))
        })
    }
}

/// What the model reads: numbered pages, each its title, address and passages.
fn shown(query: &str, pages: &[Page]) -> String {
    if pages.is_empty() {
        return format!("No pages found for \u{201c}{query}\u{201d}.");
    }
    let mut out = String::new();
    for (n, page) in pages.iter().enumerate() {
        let title = page.title.as_deref().map(str::trim).unwrap_or_default();
        let title = if title.is_empty() {
            "(untitled)"
        } else {
            title
        };
        let _ = writeln!(out, "{}. {title}", n + 1);
        // The day is enough; Exa gives a full timestamp.
        match page.published_date.as_deref().and_then(|at| at.get(..10)) {
            Some(day) => _ = writeln!(out, "   {} ({day})", page.url),
            None => _ = writeln!(out, "   {}", page.url),
        }
        for passage in &page.highlights {
            let passage = passage.split_whitespace().collect::<Vec<_>>().join(" ");
            if !passage.is_empty() {
                let _ = writeln!(out, "   {}", cut(&passage, MAX_PASSAGE));
            }
        }
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}

/// `text`, or its first `max` characters and an ellipsis.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
        None => text.to_string(),
    }
}

/// The moment `days` before `now`, as Exa takes it: `2026-11-20T00:00:00.000Z`, from the
/// start of that day.
fn days_ago(now: SystemTime, days: u64) -> String {
    let today = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) / 86_400;
    // Days since the epoch to a civil date (Howard Hinnant's algorithm).
    let z = today.saturating_sub(days) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02}T00:00:00.000Z")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// A server that answers every request with `status` and `body`, keeping what it was
    /// sent (head and body, as text).
    async fn server(status: &'static str, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                // The head, then as much body as it says it has.
                loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(head) = text.find("\r\n\r\n") {
                        let length = text[..head]
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        if request.len() >= head + 4 + length {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (base, seen)
    }

    /// The JSON a request carried.
    fn sent(request: &str) -> Value {
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    const TWO_PAGES: &str = r#"{"results":[
        {"title":" Rust 2.0 announced ","url":"https://blog.example/rust","publishedDate":"2026-11-20T09:30:00.000Z",
         "highlights":["The  team\nannounced it today.","A second passage."]},
        {"title":null,"url":"https://example.org/plain","highlights":[]}
    ]}"#;

    #[tokio::test]
    async fn a_search_asks_exa_and_shows_the_pages_it_found() {
        let (base, seen) = server("200 OK", TWO_PAGES).await;
        let tool = WebSearch::at(base, "the-key");

        let shown = tool
            .execute(json!({ "query": "  rust news  ", "num_results": 3 }))
            .await
            .unwrap();

        assert_eq!(
            shown,
            "1. Rust 2.0 announced\n   https://blog.example/rust (2026-11-20)\n   \
             The team announced it today.\n   A second passage.\n\n\
             2. (untitled)\n   https://example.org/plain"
        );
        let requests = seen.lock().unwrap();
        assert!(requests[0].starts_with("POST /search "));
        assert!(requests[0].to_lowercase().contains("x-api-key: the-key"));
        let body = sent(&requests[0]);
        assert_eq!(body["query"], "rust news");
        assert_eq!(body["numResults"], 3);
        assert_eq!(body["contents"]["highlights"], true);
        assert!(body.get("startPublishedDate").is_none());
    }

    #[tokio::test]
    async fn it_asks_for_no_more_than_ten_pages_and_only_recent_ones_when_told() {
        let (base, seen) = server("200 OK", r#"{"results":[]}"#).await;
        let tool = WebSearch::at(base, "k");

        let shown = tool
            .execute(json!({ "query": "x", "num_results": 500, "days": 7 }))
            .await
            .unwrap();
        tool.execute(json!({ "query": "x", "num_results": 0 }))
            .await
            .unwrap();

        assert_eq!(shown, "No pages found for \u{201c}x\u{201d}.");
        let requests = seen.lock().unwrap();
        assert_eq!(sent(&requests[0])["numResults"], 10);
        let since = sent(&requests[0])["startPublishedDate"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(since, days_ago(SystemTime::now(), 7));
        assert_eq!(sent(&requests[1])["numResults"], 1);
    }

    #[tokio::test]
    async fn a_refusal_is_an_error_the_model_can_read_without_the_key_in_it() {
        let (base, _) = server("401 Unauthorized", r#"{"error":"Invalid API key"}"#).await;
        let tool = WebSearch::at(base, "secret-key");

        let error = tool.execute(json!({ "query": "x" })).await.unwrap_err();

        let said = format!("{error:#}");
        assert!(
            said.contains("401") && said.contains("Invalid API key"),
            "{said}"
        );
        assert!(!said.contains("secret-key"));
    }

    #[tokio::test]
    async fn no_query_is_refused_before_anything_is_sent() {
        let tool = WebSearch::at("http://127.0.0.1:1", "k");
        assert!(tool.execute(json!({})).await.is_err());
        assert!(tool.execute(json!({ "query": "  " })).await.is_err());
    }

    #[test]
    fn days_ago_is_the_start_of_that_day() {
        let now = UNIX_EPOCH + Duration::from_secs(1_795_435_200 + 3_600);
        let today = days_ago(now, 0);
        assert!(today.ends_with("T00:00:00.000Z"));
        // 2024-03-01 minus one day is the leap day.
        let march_first = UNIX_EPOCH + Duration::from_secs(1_709_251_200 + 5);
        assert_eq!(days_ago(march_first, 0), "2024-03-01T00:00:00.000Z");
        assert_eq!(days_ago(march_first, 1), "2024-02-29T00:00:00.000Z");
        assert_eq!(days_ago(march_first, 366), "2023-03-01T00:00:00.000Z");
    }

    #[test]
    fn a_long_passage_is_cut() {
        assert_eq!(cut("héllo", 2), "hé\u{2026}");
        assert_eq!(cut("héllo", 5), "héllo");
    }

    /// The real thing: `just test tools --live` with `EXA_API_KEY` set. No model is called.
    #[tokio::test]
    #[ignore = "calls Exa"]
    async fn live_exa_finds_pages() {
        let key = std::env::var(WebSearch::KEY_VAR).expect("EXA_API_KEY");
        let shown = WebSearch::new(key)
            .execute(json!({ "query": "the Rust programming language homepage", "num_results": 2 }))
            .await
            .unwrap();
        println!("{shown}");
        assert!(shown.starts_with("1. "));
        assert!(shown.contains("https://"));
    }
}
