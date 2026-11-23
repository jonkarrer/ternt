//! Screening with Jev, for a dream: Jev looks first, and the strong model is asked only for
//! what Jev finds worth its while.
//!
//! The episode. Turns of greetings, thanks and small talk are summarized like any others,
//! into episodes nobody needs. Jev says whether anything happened in them; if not, the
//! model isn't asked.
//!
//! The cleanup. Asking a strong model to read every memory at
//! every dream, to find duplicates and contradictions that are usually not there, is the
//! most the memory work costs for the least it finds. Pairs of memories
//! that could be about the same thing (the same key, or words in common) are each one
//! yes-or-no question, and only the memories in a pair it suspects go to the model.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use anyhow::Result;
use inference::Jev;
use inference::providers::Api;
use memory::Item;
use serde_json::json;

use crate::Experience;

/// Dreaming is idle work: a batch of questions may take its time.
const PATIENCE: Duration = Duration::from_secs(10);
/// Other memories each one is paired with by the search, besides those with its key.
const NEIGHBOURS: usize = 4;
/// Questions per request, well inside what one request may hold.
const BATCH: usize = 60;
/// Pairs screened per dream; what is left over waits for the next.
const MAX_PAIRS: usize = 300;
/// A pair this likely to need cleaning up is shown to the model.
const SUSPECT_MIN: f32 = 0.5;

/// Turns less likely than this to hold anything are left without an episode. Low: a
/// summary too many costs a little, an episode missing can't be made up later by search.
const EVENTFUL_MIN: f32 = 0.3;
/// Transcript characters Jev reads to tell, newest kept: far inside what a question may hold.
const EVENTFUL_INPUT_CHARS: usize = 12_000;

const EVENTFUL_ASKS: &str = "state.turns is part of a conversation between a user and their assistant. Did anything happen in it worth keeping a summary of for later: something asked for and done, a decision, a plan, a result, or something learned? Greetings, thanks, small talk and exchanges that settled nothing are not.";

const STATE: &str = "An assistant's long-term memory about its user.";

fn asks(a: &Item, b: &Item) -> String {
    format!(
        "Two notes an assistant keeps about the same user. Should they be cleaned up together: \
         do they say the same thing, does one contradict or supersede the other, or should \
         they be merged into one note? Notes about different things, or different details of \
         one subject, are fine as they are.\nNote A: {}\nNote B: {}",
        a.body.text(),
        b.body.text()
    )
}

impl<A: Api> Experience<A> {
    /// Whether the `turns` of `thread` ending at checkpoint `end` are worth an episode. Yes
    /// when Jev can't say: a failure here must not cost a memory.
    pub(crate) async fn eventful(&self, jev: &Jev, thread: &str, end: usize, turns: &str) -> bool {
        if self.uneventful.lock().unwrap().get(thread) == Some(&end) {
            return false;
        }
        let state = json!({ "turns": crate::last_chars(turns, EVENTFUL_INPUT_CHARS) });
        let question = [("eventful".to_string(), EVENTFUL_ASKS.to_string())];
        let jev = jev.clone().within(PATIENCE);
        let likely = match jev.yes_or_no(&state, &question).await {
            Ok(answers) => answers.get("eventful").copied().unwrap_or(1.0),
            Err(error) => {
                tracing::warn!(%error, "Jev couldn't judge the turns; summarizing them");
                return true;
            }
        };
        tracing::debug!(thread, end, likely, "judged turns for an episode");
        if likely < EVENTFUL_MIN {
            (self.uneventful.lock().unwrap()).insert(thread.to_string(), end);
        }
        likely >= EVENTFUL_MIN
    }

    /// The memories among `items` that Jev suspects of repeating or contradicting another,
    /// for the model to settle. Empty when nothing needs a look, which is the usual case.
    pub(crate) async fn suspects(&self, jev: &Jev, items: &[Item]) -> Result<Vec<Item>> {
        let since = *self.screened.lock().unwrap();
        let newest = items.iter().map(|item| item.id).max().unwrap_or(since);
        let known: HashSet<i64> = items.iter().map(|item| item.id).collect();
        let scope = std::slice::from_ref(&self.scopes.write);

        // Each pair once, and only pairs with a memory no earlier dream has screened.
        let mut pairs: BTreeSet<(i64, i64)> = BTreeSet::new();
        for item in items.iter().filter(|item| item.id > since) {
            let keyed = items
                .iter()
                .filter(|other| item.key.is_some() && other.key == item.key)
                .map(|other| other.id);
            let alike = self
                .memory
                .search(&item.body.text(), scope, NEIGHBOURS + 1)?
                .into_iter()
                .map(|hit| hit.item.id);
            for other in keyed.chain(alike) {
                if other != item.id && known.contains(&other) {
                    pairs.insert((item.id.min(other), item.id.max(other)));
                }
            }
        }
        let by_id = |id: i64| items.iter().find(|item| item.id == id);
        let pairs: Vec<(&Item, &Item)> = pairs
            .into_iter()
            .filter_map(|(a, b)| Some((by_id(a)?, by_id(b)?)))
            .take(MAX_PAIRS)
            .collect();

        let jev = jev.clone().within(PATIENCE);
        let mut suspected: BTreeSet<i64> = BTreeSet::new();
        for batch in pairs.chunks(BATCH) {
            let questions: Vec<(String, String)> = batch
                .iter()
                .map(|(a, b)| (format!("p{}-{}", a.id, b.id), asks(a, b)))
                .collect();
            let answers = jev.yes_or_no(&json!(STATE), &questions).await?;
            for ((a, b), (name, _)) in batch.iter().zip(&questions) {
                if answers.get(name).is_some_and(|yes| *yes >= SUSPECT_MIN) {
                    suspected.extend([a.id, b.id]);
                }
            }
        }
        tracing::debug!(pairs = pairs.len(), suspected = ?suspected, "screened memories");
        // Only now: a request that failed leaves its pairs for the next dream.
        if pairs.len() < MAX_PAIRS {
            *self.screened.lock().unwrap() = newest;
        }
        Ok(items
            .iter()
            .filter(|item| suspected.contains(&item.id))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use inference::jev::testing::server;
    use memory::Write;

    use super::*;
    use crate::tests::{experience, key, me, unreachable};

    fn noul(yes: f32) -> serde_json::Value {
        json!({ "type": "noul", "noul": yes })
    }

    /// Every memory says "the user", so the search pairs each with the others and Jev is
    /// asked about all three pairs; it sees nothing to clean up, and the model (which nobody
    /// answers here) is never asked.
    #[tokio::test]
    async fn a_dream_asks_the_model_nothing_when_jev_suspects_nothing() {
        let exp = unreachable();
        let ids: Vec<i64> = [
            "The user likes dark roast coffee.",
            "The user drinks coffee at 7.",
            "The user's name is Jon.",
        ]
        .into_iter()
        .map(|fact| exp.memory().write(Write::fact(me(), fact)).unwrap().id)
        .collect();
        let pairs = [(0, 1), (0, 2), (1, 2)].map(|(a, b)| format!("p{}-{}", ids[a], ids[b]));
        let answers: serde_json::Map<String, serde_json::Value> = pairs
            .iter()
            .map(|pair| (pair.clone(), noul(0.12)))
            .collect();
        let reply = json!({ "model": "jev-1.14.0", "answers": answers, "usage": {} });
        let (base, seen) = server("200 OK", reply.to_string()).await;
        let exp = exp.with_jev(Jev::at(base, "k"));

        assert!(exp.refine().await.unwrap().is_empty());
        let asked = seen.lock().unwrap()[0].1["questions"].clone();
        let asked = asked.as_object().unwrap();
        assert_eq!(asked.len(), 3, "each pair once: {asked:?}");
        let coffee = asked[&pairs[0]]["instructions"].as_str().unwrap();
        assert!(
            coffee.contains("dark roast") && coffee.contains("at 7"),
            "{coffee}"
        );

        // Nothing new since: the next dream doesn't ask Jev again either.
        assert!(exp.refine().await.unwrap().is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    /// Jev suspects the pair, so the model is asked; unreachable, that is the failure.
    #[tokio::test]
    async fn a_suspected_pair_goes_to_the_model() {
        let exp = unreachable();
        let a = (exp.memory().write(Write::fact(me(), "The user uses npm."))).unwrap();
        let b = (exp
            .memory()
            .write(Write::fact(me(), "The user uses pnpm, never npm.")))
        .unwrap();
        let pair = format!("p{}-{}", a.id, b.id);
        let reply =
            json!({ "model": "jev-1.14.0", "answers": { pair.as_str(): noul(0.84) }, "usage": {} });
        let (base, _) = server("200 OK", reply.to_string()).await;
        let exp = exp.with_jev(Jev::at(base, "k"));
        assert!(exp.refine().await.is_err());
    }

    fn talked(turns: &[(&str, &str)]) -> state::Thread {
        let mut thread = state::Thread::new("t1");
        for (user, reply) in turns {
            crate::tests::turn(&mut thread, user, reply);
        }
        thread
    }

    const SMALL_TALK: [(&str, &str); 3] = [
        ("hey", "Hi! What can I do for you?"),
        ("nothing really, just checking you're there", "I'm here."),
        ("cool thanks", "Any time."),
    ];

    /// Jev finds nothing happened: no episode, and the model (which nobody answers here) is
    /// not asked. Asked once: the same turns aren't judged again at the next dream.
    #[tokio::test]
    async fn small_talk_gets_no_episode_and_no_model_call() {
        let reply =
            json!({ "model": "jev-1.14.0", "answers": { "eventful": noul(0.04) }, "usage": {} });
        let (base, seen) = server("200 OK", reply.to_string()).await;
        let exp = unreachable().with_jev(Jev::at(base, "k"));
        let thread = talked(&SMALL_TALK);

        assert!(exp.summarize(&thread).await.unwrap().is_none());
        assert!(exp.summarize(&thread).await.unwrap().is_none());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let turns = seen[0].1["state"]["turns"].as_str().unwrap();
        assert!(turns.starts_with("User: hey\nAssistant: Hi!"), "{turns}");
    }

    /// Something happened, or Jev can't say: the model is asked, as without Jev.
    #[tokio::test]
    async fn turns_worth_an_episode_or_unjudged_go_to_the_model() {
        let reply =
            json!({ "model": "jev-1.14.0", "answers": { "eventful": noul(0.9) }, "usage": {} });
        let (base, _) = server("200 OK", reply.to_string()).await;
        let exp = unreachable().with_jev(Jev::at(base, "k"));
        assert!(exp.summarize(&talked(&SMALL_TALK)).await.is_err());

        let (base, _) = server("529 Overloaded", r#"{"detail":"overloaded"}"#).await;
        let exp = unreachable().with_jev(Jev::at(base, "k"));
        assert!(exp.summarize(&talked(&SMALL_TALK)).await.is_err());
    }

    /// Jev itself, no chat model: small talk isn't worth an episode, getting something done is.
    #[tokio::test]
    #[ignore = "calls TypeSafe"]
    async fn jev_tells_small_talk_from_something_done() {
        let jev = Jev::new(std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY"));
        let exp = unreachable().with_jev(jev.clone());
        let turns =
            |thread: &state::Thread| crate::transcript(crate::messages(thread.checkpoints()));
        let idle = talked(&SMALL_TALK);
        assert!(!exp.eventful(&jev, "idle", 0, &turns(&idle)).await);
        let trip = talked(&[
            (
                "Plan a trip to Kyoto in April.",
                "Cherry blossom season; book early. I'd do 4 nights.",
            ),
            (
                "Find a ryokan near Gion.",
                "Try Gion Hatanaka, about $320 a night.",
            ),
            (
                "Book it for the 3rd to the 7th.",
                "Noted: Gion Hatanaka, April 3 to 7.",
            ),
        ]);
        assert!(exp.eventful(&jev, "trip", 0, &turns(&trip)).await);
    }

    /// Jev and a cheap model: of five memories, the contradiction is found and settled and
    /// the rest are left alone.
    #[tokio::test]
    #[ignore = "calls TypeSafe and Anthropic"]
    async fn a_dream_settles_the_one_contradiction() {
        let jev = Jev::new(std::env::var(Jev::KEY_VAR).expect("set TYPESAFE_API_KEY"));
        let exp = experience(&key()).with_jev(jev);
        for fact in [
            "The user uses npm.",
            "The user's name is Jon.",
            "The user likes dark roast coffee.",
            "The user drinks coffee every morning at 7.",
            "The user uses pnpm, never npm.",
        ] {
            exp.memory().write(Write::fact(me(), fact)).unwrap();
        }
        let refined = exp.refine().await.unwrap();
        let left: Vec<String> = (exp.memory().list(&[me()], 10).unwrap().iter())
            .map(|item| item.body.text())
            .collect();
        println!("refined {refined:?}\nleft {left:?}");
        assert_eq!(left.len(), 4, "{left:?}");
        assert!(!left.contains(&"The user uses npm.".to_string()));
        assert!(
            left.iter()
                .any(|text| text.contains("coffee every morning"))
        );
    }
}
