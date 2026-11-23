//! Placing with Jev: what a message worth a look means for memory. Without Jev one model
//! call decides everything about it (keep, correct, forget, ignore) and writes the text.
//! With Jev the deciding is a choice question: the message changes one of the notes
//! related to it, or says something new, or holds nothing to keep. The model is then asked
//! only for the wording, of that one thing, and for "nothing" isn't asked at all, which is
//! most of what the patterns flag.
//!
//! Jev decides only when it is sure. A message that changes a note and adds something else,
//! or that it can't place, goes to the model whole, as without Jev.

use std::collections::HashMap;

use anyhow::Result;
use attention::Signal;
use inference::Jev;
use inference::providers::Api;
use memory::{Item, Source};
use serde_json::json;

use crate::{Decision, Experience, RELATED_LIMIT, describe, json_array, sift, to_decision};

/// Jev's choice is taken when it is at least this sure. Its clear cases answer near 1;
/// the ones it splits between two options, well below this.
const SURE_MIN: f32 = 0.8;
const NEW: &str = "new";
const NOTHING: &str = "nothing";

const INSTRUCTIONS: &str = "state.message is what a user just said to their assistant, which \
keeps notes about the user. Each option is one of those notes, or \"new\", or \"nothing\". \
Choose what the message means for the notes.";
const CHANGES: &str = "The message corrects, updates, adds a detail to or withdraws this note: ";
const NEW_MEANS: &str = "The message says something lasting about the user that is worth \
keeping and that none of the notes is about.";
const NOTHING_MEANS: &str = "Nothing in the message is worth keeping: a one-off request, a \
question, small talk, or something a note already says.";

impl<A: Api> Experience<A> {
    /// Decisions for `signals`, message by message: Jev's where it is sure, the model's
    /// (`decide_by_model`) for the rest.
    pub(crate) async fn place(
        &self,
        jev: &Jev,
        signals: &[&Signal],
        thread: &str,
    ) -> Result<Vec<Decision>> {
        let jev = jev.clone().within(sift::PATIENCE);
        // A message can raise several signals (a preference that is also a correction): one
        // question about it.
        let mut messages: Vec<(&str, Vec<&Signal>)> = Vec::new();
        for signal in signals {
            match messages.iter_mut().find(|(text, _)| *text == signal.text) {
                Some((_, raised)) => raised.push(signal),
                None => messages.push((&signal.text, vec![signal])),
            }
        }

        let mut decisions = Vec::new();
        let mut unplaced: Vec<&Signal> = Vec::new();
        for (text, raised) in messages {
            let related: Vec<Item> = self
                .memory
                .search(text, &self.scopes.searched(), RELATED_LIMIT)?
                .into_iter()
                .map(|hit| hit.item)
                .collect();
            let mut options: Vec<(String, String)> = related
                .iter()
                .map(|item| {
                    (
                        format!("m{}", item.id),
                        format!("{CHANGES}{}", item.body.text()),
                    )
                })
                .collect();
            options.push((NEW.into(), NEW_MEANS.into()));
            options.push((NOTHING.into(), NOTHING_MEANS.into()));
            let chosen = match jev
                .choose(&json!({ "message": text }), INSTRUCTIONS, &options)
                .await
            {
                Ok(chosen) if chosen.confidence >= SURE_MIN => chosen,
                Ok(chosen) => {
                    tracing::debug!(choice = %chosen.choice, confidence = chosen.confidence, "Jev isn't sure; the model places it");
                    unplaced.extend(raised);
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%error, "Jev couldn't place a message; the model does");
                    unplaced.extend(raised);
                    continue;
                }
            };
            tracing::debug!(choice = %chosen.choice, confidence = chosen.confidence, "placed");
            let source = Some(Source {
                thread: thread.to_string(),
                checkpoint: raised[0].checkpoint,
            });
            // The wording is the model's. If it fails for this message (no answer, or one
            // that can't be read), this message goes to the model whole with the unplaced
            // ones; what was decided for the others stands.
            let worded = match chosen.choice.as_str() {
                NOTHING => Ok(vec![Decision::Ignore {
                    reason: format!("nothing to keep (Jev, {:.2})", chosen.confidence),
                }]),
                NEW => self.word_new(text, source).await,
                note => match related
                    .into_iter()
                    .find(|item| format!("m{}", item.id) == note)
                {
                    Some(item) => self.word_change(text, item, source).await,
                    None => Err(anyhow::anyhow!("Jev chose {note}, which it wasn't offered")),
                },
            };
            match worded {
                Ok(worded) => decisions.extend(worded),
                Err(error) => {
                    tracing::warn!(%error, "a placed message couldn't be worded; the model decides it whole");
                    unplaced.extend(raised);
                }
            }
        }
        if !unplaced.is_empty() {
            match self.decide_by_model(&unplaced, thread).await {
                Ok(decided) => decisions.extend(decided),
                // With nothing else decided this is the failure it would be without Jev;
                // otherwise what Jev placed is kept, and the rest is lost as it would be.
                Err(error) if decisions.is_empty() => return Err(error),
                Err(error) => {
                    tracing::warn!(%error, "the model couldn't decide the unplaced messages");
                }
            }
        }
        Ok(decisions)
    }

    /// The message says something new: the model writes it down.
    async fn word_new(&self, text: &str, source: Option<Source>) -> Result<Vec<Decision>> {
        let reply = self.ask(WRITE_PROMPT, text.to_string()).await?;
        let none = HashMap::new();
        Ok(json_array(&reply)?
            .iter()
            .filter_map(|raw| to_decision(raw, &self.scopes.write, source.clone(), &none))
            .filter(|decision| matches!(decision, Decision::Retain(_)))
            .collect())
    }

    /// The message changes `item`: the model rewrites that note, or drops it.
    async fn word_change(
        &self,
        text: &str,
        item: Item,
        source: Option<Source>,
    ) -> Result<Vec<Decision>> {
        let input = json!({ "message": text, "note": describe(&item) });
        let reply = self.ask(REWRITE_PROMPT, input.to_string()).await?;
        let shown = HashMap::from([(item.id, item)]);
        // Whatever it answers is about that note: it was shown no other.
        Ok(json_array(&reply)?
            .iter()
            .filter_map(|raw| to_decision(raw, &self.scopes.write, source.clone(), &shown))
            .filter(|decision| !matches!(decision, Decision::Retain(_)))
            .take(1)
            .collect())
    }
}

const WRITE_PROMPT: &str = r#"You write an AI agent's long-term memory. The user's message says something lasting
about them that is worth keeping. Write it down. Answer with ONLY a JSON array, one entry per separate thing:
  {"action":"retain","kind":"semantic","text":"<standalone fact>","key":"<optional stable key>"}
  {"action":"retain","kind":"procedural","name":"...","trigger":"<when it applies>","steps":["..."]}
Write facts as standalone third-person statements ("The user prefers pnpm"), in the language the user used.
Leave out one-off requests and anything not about the user or how they want things done. Use a short key
(e.g. "name", "package-manager") for facts that are settings."#;

const REWRITE_PROMPT: &str = r#"You maintain an AI agent's long-term memory. You get a "note" it keeps about the user
and a "message" the user just sent that changes it. Answer with ONLY a JSON array holding one entry:
  {"action":"refine","id":<the note's id>,"text":"<the note, corrected or completed: a standalone third-person statement>"}
  {"action":"forget","id":<the note's id>}   when the user withdraws it and nothing replaces it
  {"action":"ignore","reason":"..."}          when the note already says it
Keep what the note says that the message doesn't change, in the language the note is in."#;

#[cfg(test)]
mod tests {
    use context::Message;
    use inference::jev::testing::server;
    use memory::Write;
    use state::Thread;

    use super::*;
    use crate::tests::{experience, key, me, unreachable};

    fn opened(text: &str) -> Thread {
        let mut thread = Thread::new("t1");
        thread.start_turn(Message::user(text)).unwrap();
        thread.start_step().unwrap();
        thread
    }

    /// A reply from Jev to the choice question; a `lasting` likelihood rides along, for
    /// the request `sift` makes of the same stand-in.
    fn chose(choice: &str, confidence: f32) -> String {
        json!({ "model": "jev-1.14.0", "usage": {}, "answers": {
            "choice": { "type": "choice", "choice": choice, "confidence": confidence },
            "lasting": { "type": "noul", "noul": 0.02 },
        }})
        .to_string()
    }

    /// A pattern flags it ("remember"), Jev finds nothing to keep: ignored, and the model
    /// (which nobody answers here) is never asked.
    #[tokio::test]
    async fn what_jev_finds_nothing_in_costs_no_model_call() {
        let (base, seen) = server("200 OK", chose("nothing", 0.99)).await;
        let exp = unreachable().with_jev(Jev::at(base, "k"));
        let outcome = exp
            .consider(&mut opened("Remember to be nice today!"))
            .await
            .unwrap();
        assert!(
            outcome.failed.is_empty() && outcome.written.is_empty(),
            "{outcome:#?}"
        );
        assert!(
            outcome
                .decisions
                .iter()
                .any(|d| matches!(d, Decision::Ignore { reason } if reason.contains("Jev"))),
            "{outcome:#?}"
        );
        let seen = seen.lock().unwrap();
        let criteria = seen.last().unwrap().1["questions"]["choice"]["criteria"].clone();
        assert_eq!(
            criteria.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["new", "nothing"]
        );
    }

    /// Sure of a choice, Jev has the model asked for the wording only; unsure, for the
    /// whole decision. Either way the model is asked, and here that is the failure.
    #[tokio::test]
    async fn the_model_words_what_jev_places_and_decides_what_it_cant() {
        for (choice, confidence) in [("new", 0.97), ("new", 0.55)] {
            let (base, _) = server("200 OK", chose(choice, confidence)).await;
            let exp = unreachable().with_jev(Jev::at(base, "k"));
            let outcome = (exp.consider(&mut opened("Remember that I'm allergic to peanuts.")))
                .await
                .unwrap();
            assert!(outcome.failed[0].starts_with("deciding:"), "{outcome:#?}");
        }
    }

    /// Jev and a cheap model: a new fact is written, a changed one rewritten in place,
    /// and a reminder that isn't a memory costs nothing.
    #[tokio::test]
    #[ignore = "calls TypeSafe and Anthropic"]
    async fn jev_places_and_the_model_words() {
        let jev = Jev::new(std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY"));
        let exp = experience(&key()).with_jev(jev);
        let school = (exp.memory())
            .write(Write::fact(
                me(),
                "The user's daughter Mia starts school at 8.",
            ))
            .unwrap();

        let outcome = (exp.consider(&mut opened("Remember that I'm allergic to peanuts.")))
            .await
            .unwrap();
        println!("{:?}", outcome.written);
        assert_eq!(outcome.written.len(), 1, "{outcome:#?}");
        assert!(
            outcome.written[0]
                .body
                .text()
                .to_lowercase()
                .contains("peanut")
        );

        let outcome = (exp.consider(&mut opened("Correction: Mia's school now starts at 8:30.")))
            .await
            .unwrap();
        println!("{:?}", outcome.written);
        assert_eq!(outcome.written.len(), 1, "{outcome:#?}");
        assert_eq!(outcome.written[0].replaces, Some(school.id), "{outcome:#?}");
        assert!(outcome.written[0].body.text().contains("8:30"));

        let outcome = exp
            .consider(&mut opened("Remember to be nice today!"))
            .await
            .unwrap();
        assert!(outcome.written.is_empty(), "{outcome:#?}");
        // Jev's own decision, not the model's: the model wasn't asked.
        let by_jev =
            |d: &Decision| matches!(d, Decision::Ignore { reason } if reason.contains("Jev"));
        assert!(outcome.decisions.iter().any(by_jev), "{outcome:#?}");
        assert_eq!(exp.memory().list(&[me()], 10).unwrap().len(), 2);
    }
}
