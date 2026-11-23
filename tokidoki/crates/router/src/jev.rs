//! Jev as the router's classifier: one typed question, "which of these threads does this
//! message continue, or none", answered with a confidence the router holds to its
//! `classifier_min`. It reads what the model tier would (the candidates' titles and cards,
//! where the user was last), for a small fraction of the price and without a reply to parse.

use std::future::Future;
use std::pin::Pin;

use inference::Jev;
use serde_json::json;

use crate::{Classified, Classifier, Input, ThreadView, ago, card_doc, elapsed};

/// The option that stands for no thread. Threads are offered by number, so no thread's ID
/// can be mistaken for it, whatever a store calls its threads.
const NEW: &str = "new";

const INSTRUCTIONS: &str = "state.message is what a user just said to their assistant. Each \
option is one of their earlier conversations, described by its title and a short summary. \
Choose the conversation the message continues: the same subject, a follow-up, or something \
it refers back to. Choose \"new\" when it starts something none of them covers.";

pub struct JevClassifier(pub Jev);

impl Classifier for JevClassifier {
    fn classify<'a>(
        &'a self,
        input: &'a Input,
        last: Option<&'a ThreadView>,
        candidates: &'a [&'a ThreadView],
    ) -> Pin<Box<dyn Future<Output = Classified> + Send + 'a>> {
        Box::pin(async move {
            let name = |index: usize| (index + 1).to_string();
            let mut options: Vec<(String, String)> = candidates
                .iter()
                .enumerate()
                .map(|(index, thread)| {
                    let active = ago(elapsed(input.now, thread.active_at));
                    (
                        name(index),
                        format!("{}\n(last active {active})", card_doc(thread)),
                    )
                })
                .collect();
            options.push((
                NEW.into(),
                "A new conversation: the message continues none of the others.".into(),
            ));
            let mut state = json!({ "message": input.text });
            if let Some(index) =
                last.and_then(|last| candidates.iter().position(|t| t.info.id == last.info.id))
            {
                state["the_user_was_last_in"] = name(index).into();
            }
            match self.0.choose(&state, INSTRUCTIONS, &options).await {
                Ok(chosen) if chosen.choice == NEW => Some((None, chosen.confidence)),
                Ok(chosen) => {
                    let index = chosen.choice.parse::<usize>().ok()?.checked_sub(1)?;
                    let thread = candidates.get(index)?;
                    Some((Some(thread.info.id.clone()), chosen.confidence))
                }
                Err(error) => {
                    tracing::warn!(%error, "Jev couldn't route; the model decides");
                    None
                }
            }
        })
    }
}
