//! Sifting with Jev: at the start of a turn, one request asks what the patterns and the
//! keyword search can't tell. Is the message worth remembering, though no pattern says so?
//! And which memories would help answer it, though they may share no word with it ("a
//! dinner recipe" and "the user is vegetarian")?
//!
//! Each is a yes-or-no question answered with a likelihood, all in the one request. Jev
//! only judges: a memory it finds relevant is recalled as it stands, and a message it finds
//! worth keeping still goes to the model, which writes the fact. Without a TypeSafe key, or
//! when Jev doesn't answer, recall and retain work as they did: by pattern and by search.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Result;
use attention::{Intent, Signal, Source};
use inference::Jev;
use inference::providers::Api;
use memory::Item;
use serde_json::json;

use crate::Experience;

/// A turn waits for this before its first model call: longer than Jev usually takes by
/// several times, short enough that a stalled request isn't felt as a hang.
pub(crate) const PATIENCE: Duration = inference::jev::BEFORE_A_REPLY;
/// Memories Jev is asked about per message: the search's hits first, then the newest.
const CANDIDATES: usize = 24;
/// Of those, how many the search may bring.
const SEARCHED: usize = 8;
/// A memory this likely to help is recalled…
const RELEVANT_MIN: f32 = 0.6;
/// …up to this many, the likeliest first. More than a search alone brings: these were judged.
const RECALL_LIMIT: usize = 5;
/// A message this likely to say something lasting is put to the model to retain, as
/// TypeSafe's own memory example does.
const RETAIN_MIN: f32 = 0.5;
/// The question about the message itself; the others are named after the memory they ask of.
const LASTING: &str = "lasting";

const LASTING_ASKS: &str = "state.message is what a user just said to their assistant. Does \
it state something lasting about the user that is worth remembering for future \
conversations: a fact about them, their life or their work, a preference, or a standing \
instruction? One-off requests, questions and small talk are not.";

const RELEVANT_ASKS: &str = "state.message is what a user just said to their assistant. Would \
knowing this about the user help the assistant respond well to it?\nKnown about the user: ";

/// What Jev said of a turn's opening message.
#[derive(Debug)]
pub(crate) struct Sifted {
    /// The message, as the signal that opened the turn carried it.
    pub text: String,
    pub checkpoint: usize,
    /// How likely the message says something worth keeping.
    pub lasting: f32,
    /// The memories that would help answer it, likeliest first.
    pub relevant: Vec<Item>,
}

impl Sifted {
    /// The signal for a message worth keeping that no detector flagged; `None` when one
    /// did, or when it isn't.
    pub fn retain(&self, signals: &[Signal]) -> Option<Signal> {
        let flagged = signals.iter().any(|s| {
            s.checkpoint == self.checkpoint && matches!(s.intent, Intent::Retain | Intent::Refine)
        });
        (self.lasting >= RETAIN_MIN && !flagged).then(|| Signal {
            detector: "jev".into(),
            intent: Intent::Retain,
            salience: self.lasting,
            checkpoint: self.checkpoint,
            source: Source::User,
            text: self.text.clone(),
            spans: Vec::new(),
            reason: "says something lasting about the user".into(),
        })
    }
}

impl<A: Api> Experience<A> {
    /// Asks Jev about the user's message that opened this turn, if `signals` has one.
    /// `in_context` are the memories already recalled into the thread: not asked about again.
    pub(crate) async fn sift(
        &self,
        jev: &Jev,
        signals: &[Signal],
        in_context: &HashSet<i64>,
    ) -> Result<Option<Sifted>> {
        // Attention marks every turn's opening message as a possible recall.
        let Some(opening) = signals
            .iter()
            .filter(|s| s.intent == Intent::Recall && s.source == Source::User)
            .max_by_key(|s| s.checkpoint)
        else {
            return Ok(None);
        };
        let scopes = self.scopes.searched();
        let mut candidates: Vec<Item> = Vec::new();
        let searched = self.memory.search(&opening.text, &scopes, SEARCHED)?;
        let newest = self.memory.list(&scopes, CANDIDATES)?;
        for item in searched.into_iter().map(|hit| hit.item).chain(newest) {
            if candidates.len() < CANDIDATES
                && !in_context.contains(&item.id)
                && !candidates.iter().any(|c| c.id == item.id)
            {
                candidates.push(item);
            }
        }

        let name = |item: &Item| format!("m{}", item.id);
        let mut questions = vec![(LASTING.to_string(), LASTING_ASKS.to_string())];
        questions.extend(
            candidates
                .iter()
                .map(|item| (name(item), format!("{RELEVANT_ASKS}{}", item.body.text()))),
        );
        let answers = jev
            .clone()
            .within(PATIENCE)
            .yes_or_no(&json!({ "message": opening.text }), &questions)
            .await?;
        let likely = |name: &str| answers.get(name).copied().unwrap_or(0.0);

        let mut relevant: Vec<(f32, Item)> = candidates
            .into_iter()
            .map(|item| (likely(&name(&item)), item))
            .filter(|(yes, _)| *yes >= RELEVANT_MIN)
            .collect();
        relevant.sort_by(|a, b| b.0.total_cmp(&a.0));
        tracing::debug!(
            lasting = likely(LASTING),
            relevant = ?relevant.iter().map(|(yes, item)| (item.id, yes)).collect::<Vec<_>>(),
            asked = questions.len(),
            "sifted"
        );
        Ok(Some(Sifted {
            text: opening.text.clone(),
            checkpoint: opening.checkpoint,
            lasting: likely(LASTING),
            relevant: relevant
                .into_iter()
                .take(RECALL_LIMIT)
                .map(|(_, item)| item)
                .collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use inference::jev::testing::server;
    use memory::Write;
    use serde_json::Value;
    use state::Thread;

    use super::*;
    use crate::tests::{experience, key, me};
    use context::Message;

    const RECIPE: &str = "Suggest a dinner recipe for tonight.";

    fn opened(text: &str) -> Thread {
        let mut thread = Thread::new("t1");
        thread.start_turn(Message::user(text)).unwrap();
        thread.start_step().unwrap();
        thread
    }

    /// A reply from Jev with these likelihoods.
    fn nouls(answers: &[(String, f32)]) -> String {
        let answers: serde_json::Map<String, Value> = answers
            .iter()
            .map(|(name, yes)| (name.clone(), json!({ "type": "noul", "noul": yes })))
            .collect();
        json!({ "model": "jev-1.14.0", "answers": answers, "usage": {} }).to_string()
    }

    /// The search finds nothing for the recipe (no word in common with either memory), and
    /// would have recalled nothing. Jev is asked about both and one helps.
    #[tokio::test]
    async fn jev_recalls_what_helps_whatever_words_it_uses() {
        let exp = experience("unused");
        let diet = (exp
            .memory()
            .write(Write::fact(me(), "The user is vegetarian.")))
        .unwrap();
        let tool = (exp
            .memory()
            .write(Write::fact(me(), "The user uses pnpm, never npm.")))
        .unwrap();
        let (base, seen) = server(
            "200 OK",
            nouls(&[
                ("lasting".into(), 0.04),
                (format!("m{}", diet.id), 0.9),
                (format!("m{}", tool.id), 0.05),
            ]),
        )
        .await;
        let exp = exp.with_jev(Jev::at(base, "k"));
        let mut thread = opened(RECIPE);

        let outcome = exp.consider(&mut thread).await.unwrap();
        assert_eq!(outcome.recalled, std::slice::from_ref(&diet));
        assert!(
            outcome.written.is_empty() && outcome.failed.is_empty(),
            "{outcome:#?}"
        );
        assert!(
            thread.context().messages[0]
                .text()
                .contains("- The user is vegetarian.")
        );

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one request for the turn");
        let sent = &seen[0].1;
        assert_eq!(sent["state"]["message"], RECIPE);
        let questions = sent["questions"].as_object().unwrap();
        assert_eq!(
            questions.len(),
            3,
            "the message, and each memory: {questions:?}"
        );
        let about_diet = questions[&format!("m{}", diet.id)]["instructions"]
            .as_str()
            .unwrap();
        assert!(about_diet.ends_with("The user is vegetarian."));
    }

    /// No pattern flags this message; Jev says it is worth keeping, so it reaches the model
    /// (which nobody answers here: the attempt is what shows).
    #[tokio::test]
    async fn jev_flags_what_is_worth_keeping_when_no_pattern_does() {
        let said = "Mia starts school at 8, so mornings are tight.";
        let without = experience("unused");
        let outcome = without.consider(&mut opened(said)).await.unwrap();
        assert!(
            outcome.signals.iter().all(|s| s.intent == Intent::Recall),
            "{outcome:#?}"
        );

        let (base, _) = server("200 OK", nouls(&[("lasting".into(), 0.9)])).await;
        let exp = crate::tests::unreachable().with_jev(Jev::at(base, "k"));
        let outcome = exp.consider(&mut opened(said)).await.unwrap();
        let flagged: Vec<_> = (outcome.signals.iter())
            .filter(|s| s.detector == "jev")
            .collect();
        assert_eq!(flagged.len(), 1, "{outcome:#?}");
        assert_eq!(
            (flagged[0].intent, flagged[0].salience),
            (Intent::Retain, 0.9)
        );
        assert!(outcome.failed[0].starts_with("deciding:"), "{outcome:#?}");
    }

    /// Jev down: the turn goes on as it would without it, by search, and says what failed.
    #[tokio::test]
    async fn without_an_answer_from_jev_the_search_recalls() {
        let (base, _) = server("529 Overloaded", r#"{"detail":"overloaded"}"#).await;
        let exp = experience("unused").with_jev(Jev::at(base, "k"));
        let name = (exp
            .memory()
            .write(Write::fact(me(), "The user's name is Jon.")))
        .unwrap();

        let outcome = exp.consider(&mut opened("What's my name?")).await.unwrap();
        assert_eq!(outcome.recalled, [name]);
        assert!(outcome.failed[0].starts_with("sifting:"), "{outcome:#?}");
    }

    /// Jev itself: the memory that helps is recalled though it shares no word with the
    /// message, the others aren't, and a request isn't taken for something to keep.
    #[tokio::test]
    #[ignore = "calls TypeSafe"]
    async fn jev_sifts_a_message() {
        let jev = Jev::new(std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY"));
        let exp = experience("unused").with_jev(jev);
        let diet = (exp
            .memory()
            .write(Write::fact(me(), "The user is vegetarian.")))
        .unwrap();
        for other in [
            "The user uses pnpm, never npm.",
            "The user deploys from the Mac mini, never from laptops.",
            "The user's daughter Mia starts school at 8.",
        ] {
            exp.memory().write(Write::fact(me(), other)).unwrap();
        }
        let started = std::time::Instant::now();
        let outcome = exp.consider(&mut opened(RECIPE)).await.unwrap();
        println!("{:?} in {:?}", outcome.recalled, started.elapsed());
        assert!(outcome.recalled.contains(&diet), "{outcome:#?}");
        assert!(outcome.recalled.len() <= 2, "{outcome:#?}");
        assert!(
            outcome.signals.iter().all(|s| s.detector != "jev"),
            "{outcome:#?}"
        );
    }

    /// Jev and the model together, on a cheap model: a fact stated in passing, which no
    /// pattern catches, is kept.
    #[tokio::test]
    #[ignore = "calls TypeSafe and Anthropic"]
    async fn a_fact_said_in_passing_is_remembered() {
        let jev = Jev::new(std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY"));
        let exp = experience(&key()).with_jev(jev);
        let said = "Mia starts school at 8, so mornings are tight.";
        let outcome = exp.consider(&mut opened(said)).await.unwrap();
        println!("{:?}", outcome.written);
        assert_eq!(outcome.written.len(), 1, "{outcome:#?}");
        assert!(
            outcome.written[0].body.text().contains("Mia"),
            "{outcome:#?}"
        );
    }
}
