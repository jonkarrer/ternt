//! Jev (<https://docs.typesafe.ai>), TypeSafe's model for typed answers: it is sent some
//! `state` and named questions about it, and gives each a typed answer with how sure it is.
//! Not a chat model: there is nothing to prompt and no text to parse, and a request costs a
//! small fraction of the cheapest chat model's, so it suits the many small decisions made
//! around a conversation (which thread an input belongs to, first).
//!
//! One request, no memory on their side: everything a question needs goes in `state`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const TYPESAFE: &str = "https://api.typesafe.ai";
/// Pinned, as TypeSafe advises once thresholds are set against a version's confidences
/// (the router's `classifier_min`). Moving it means checking those again.
pub const MODEL: &str = "jev-1.14.0";
/// These decisions stand in front of an answer: one that is slow is worth less than the
/// fallback behind it.
const TIMEOUT: Duration = Duration::from_secs(5);
/// What a decision made on the way to a reply (where a message goes, what to recall for
/// it) waits for Jev: several times what Jev usually takes, short enough that a stalled
/// request isn't felt as a hang. Idle work can wait longer.
pub const BEFORE_A_REPLY: Duration = Duration::from_secs(2);
/// After Jev didn't answer, or answered that it can't just now, it isn't asked again for
/// this long: every question fails at once and its fallback decides. A turn asks Jev
/// several things one after the other, and without this an outage would cost it each
/// one's full wait before its first model call.
const REST: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Jev {
    http: reqwest::Client,
    base: String,
    key: String,
    model: String,
    timeout: Duration,
    /// Until when it is left alone (`REST`). Shared by its clones, which are the same Jev
    /// asked with another patience.
    resting: Arc<Mutex<Option<Instant>>>,
}

/// The option Jev picked from a choice question's, and how sure it is (0 to 1).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    pub choice: String,
    pub confidence: f32,
}

impl Jev {
    /// The environment variable its key comes in.
    pub const KEY_VAR: &str = "TYPESAFE_API_KEY";

    pub fn new(key: impl Into<String>) -> Self {
        Self::at(TYPESAFE, key)
    }

    /// Against another server than TypeSafe's: a test's.
    pub fn at(base: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into(),
            key: key.into(),
            model: MODEL.into(),
            timeout: TIMEOUT,
            resting: Arc::default(),
        }
    }

    /// Waiting at most this long for an answer, for a decision with less time than most.
    pub fn within(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The answers to `questions` about `state`, by the names the questions were given.
    pub async fn ask(&self, state: &Value, questions: Value) -> Result<Map<String, Value>> {
        if let Some(until) = *self.resting.lock().unwrap()
            && Instant::now() < until
        {
            bail!("Jev didn't answer a moment ago and isn't asked again yet");
        }
        let body = json!({ "model": self.model, "state": state, "questions": questions });
        let response = self
            .http
            .post(format!("{}/v1/systemone", self.base))
            .bearer_auth(&self.key)
            .timeout(self.timeout)
            .json(&body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.rest();
                // Without the address: the error is logged.
                bail!("Jev didn't answer: {}", error.without_url());
            }
        };
        let status = response.status();
        // Its trouble, not the question's: the next question would fare no better.
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            self.rest();
        }
        if !status.is_success() {
            let said = response.text().await.unwrap_or_default();
            let said: String = said.trim().chars().take(300).collect();
            bail!("Jev failed ({status}): {said}");
        }
        #[derive(Deserialize)]
        struct Answered {
            answers: Map<String, Value>,
        }
        let answered: Answered = response.json().await.context("reading Jev's answer")?;
        Ok(answered.answers)
    }

    fn rest(&self) {
        *self.resting.lock().unwrap() = Some(Instant::now() + REST);
    }

    /// Yes-or-no questions (name, what is asked), all about the same `state`, in one request:
    /// each answered with how likely "yes" is, 0 to 1, by its name. Jev calls these nouls.
    pub async fn yes_or_no(
        &self,
        state: &Value,
        questions: &[(String, String)],
    ) -> Result<HashMap<String, f32>> {
        let asked: Map<String, Value> = questions
            .iter()
            .map(|(name, asks)| {
                (
                    name.clone(),
                    json!({ "type": "noul", "instructions": asks }),
                )
            })
            .collect();
        let answers = self.ask(state, asked.into()).await?;
        questions
            .iter()
            .map(|(name, _)| {
                let yes = answers.get(name).and_then(|answer| answer["noul"].as_f64());
                let yes = yes.with_context(|| format!("Jev left {name:?} unanswered"))?;
                Ok((name.clone(), yes as f32))
            })
            .collect()
    }

    /// One choice question: which of `options` (name, what it means) fits `state`, going by
    /// `instructions`. The names are the question's criteria, so they must differ.
    pub async fn choose(
        &self,
        state: &Value,
        instructions: &str,
        options: &[(String, String)],
    ) -> Result<Choice> {
        let criteria: Map<String, Value> = options
            .iter()
            .map(|(name, means)| (name.clone(), means.as_str().into()))
            .collect();
        let question =
            json!({ "type": "choice", "instructions": instructions, "criteria": criteria });
        let mut answers = self.ask(state, json!({ "choice": question })).await?;
        let answer = answers
            .remove("choice")
            .context("Jev left the question unanswered")?;
        let choice: Choice = serde_json::from_value(answer).context("reading Jev's choice")?;
        if !options.iter().any(|(name, _)| *name == choice.choice) {
            bail!("Jev chose {:?}, which wasn't offered", choice.choice);
        }
        Ok(choice)
    }
}

/// A stand-in for TypeSafe's server, for tests here and in the crates that ask Jev things:
/// nothing in a test suite should need the real one, or a key.
pub mod testing {
    use std::sync::{Arc, Mutex};

    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// What the server was sent, request by request: the head as text, the body as JSON.
    pub type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    /// A server that answers every request with `status` (`200 OK`) and `body`. Returns its
    /// address, for `Jev::at`, and what it has been sent so far.
    pub async fn server(status: &'static str, body: impl Into<String>) -> (String, Seen) {
        let body = body.into();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Seen::default();
        let requests = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut read = Vec::new();
                let mut buffer = [0u8; 4096];
                // The head, then as much body as it says it has.
                let (head, sent) = loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    read.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&read);
                    if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if sent.len() >= length {
                            break (head.to_string(), sent.to_string());
                        }
                    }
                    assert!(n > 0, "the request ended early");
                };
                let sent = serde_json::from_str(&sent).unwrap_or(Value::Null);
                requests.lock().unwrap().push((head, sent));
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (base, seen)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::server;
    use super::*;

    /// After an answer that says Jev can't just now, the next question isn't sent: it
    /// fails at once, for its fallback to decide, and so does a clone's.
    #[tokio::test]
    async fn an_outage_is_not_waited_for_twice() {
        let (base, seen) = testing::server("529 Overloaded", r#"{"detail":"overloaded"}"#).await;
        let jev = Jev::at(base, "k");
        let question = [("q".to_string(), "Is it?".to_string())];
        assert!(jev.yes_or_no(&json!({}), &question).await.is_err());
        let patient = jev.clone().within(Duration::from_secs(10));
        let error = patient.yes_or_no(&json!({}), &question).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("isn't asked again yet"),
            "{error:#}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    /// A question Jev refuses (a bad key, a bad request) says nothing about the next one.
    #[tokio::test]
    async fn a_refused_question_does_not_stop_the_next() {
        let (base, seen) = testing::server("400 Bad Request", r#"{"detail":"no"}"#).await;
        let jev = Jev::at(base, "k");
        let question = [("q".to_string(), "Is it?".to_string())];
        assert!(jev.yes_or_no(&json!({}), &question).await.is_err());
        assert!(jev.yes_or_no(&json!({}), &question).await.is_err());
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    fn options() -> Vec<(String, String)> {
        vec![
            (
                "billing".into(),
                "Invoices, refunds, payment methods".into(),
            ),
            ("technical".into(), "Bugs, errors, outages".into()),
        ]
    }

    #[tokio::test]
    async fn a_choice_is_asked_as_one_typed_question_and_read_with_its_confidence() {
        let (base, seen) = server(
            "200 OK",
            r#"{"model":"jev-1.14.0","answers":{"choice":{"type":"choice","choice":"billing","confidence":0.93,
                "probabilities":{"billing":0.93,"technical":0.07}}},"usage":{"input_tokens":41,"output_tokens":1}}"#,
        )
        .await;
        let chosen = Jev::at(base, "secret")
            .choose(
                &json!({ "ticket": "I was charged twice" }),
                "Which team?",
                &options(),
            )
            .await
            .unwrap();
        assert_eq!(
            chosen,
            Choice {
                choice: "billing".into(),
                confidence: 0.93
            }
        );

        let (head, body) = seen.lock().unwrap()[0].clone();
        assert!(head.starts_with("POST /v1/systemone "), "{head}");
        assert!(
            head.to_lowercase().contains("authorization: bearer secret"),
            "{head}"
        );
        assert_eq!(
            body,
            json!({
                "model": MODEL,
                "state": { "ticket": "I was charged twice" },
                "questions": { "choice": {
                    "type": "choice",
                    "instructions": "Which team?",
                    "criteria": {
                        "billing": "Invoices, refunds, payment methods",
                        "technical": "Bugs, errors, outages",
                    },
                }},
            })
        );
    }

    #[tokio::test]
    async fn a_refusal_is_an_error_that_says_what_jev_said_and_never_the_key() {
        let (base, _) = server("401 Unauthorized", r#"{"detail":"Invalid API key"}"#).await;
        let error = Jev::at(base, "secret")
            .choose(&json!("x"), "Which?", &options())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("401") && error.contains("Invalid API key"),
            "{error}"
        );
        assert!(!error.contains("secret"), "{error}");
    }

    #[tokio::test]
    async fn an_option_that_was_not_offered_is_an_error() {
        let (base, _) = server(
            "200 OK",
            r#"{"model":"jev-1.14.0","answers":{"choice":{"type":"choice","choice":"sales","confidence":0.9}},"usage":{}}"#,
        )
        .await;
        let error = Jev::at(base, "k")
            .choose(&json!("x"), "Which?", &options())
            .await;
        assert!(error.unwrap_err().to_string().contains("wasn't offered"));
    }

    #[tokio::test]
    async fn yes_or_no_questions_go_in_one_request_and_come_back_as_likelihoods() {
        let (base, seen) = server(
            "200 OK",
            r#"{"model":"jev-1.14.0","answers":{"lasting":{"type":"noul","noul":0.9},"urgent":{"type":"noul","noul":0.05}},"usage":{}}"#,
        )
        .await;
        let questions = [
            (
                "lasting".to_string(),
                "Does it state a lasting fact?".to_string(),
            ),
            ("urgent".to_string(), "Is it urgent?".to_string()),
        ];
        let answers = Jev::at(base, "k")
            .yes_or_no(&json!({ "message": "I use pnpm." }), &questions)
            .await
            .unwrap();
        assert_eq!(
            answers,
            HashMap::from([("lasting".into(), 0.9), ("urgent".into(), 0.05)])
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].1["questions"]["urgent"],
            json!({ "type": "noul", "instructions": "Is it urgent?" })
        );
    }

    /// The real thing, on the cookbook's kind of question. No chat model is called.
    #[tokio::test]
    #[ignore = "calls TypeSafe"]
    async fn jev_routes_a_ticket() {
        let key = std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY");
        let started = std::time::Instant::now();
        let chosen = Jev::new(key)
            .choose(
                &json!({ "ticket": "I was charged twice for my subscription this month." }),
                "Which team should handle this support ticket?",
                &options(),
            )
            .await
            .unwrap();
        println!("{chosen:?} in {:?}", started.elapsed());
        assert_eq!(chosen.choice, "billing");
    }
}
