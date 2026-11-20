//! Settling: after a long idle (the agent's `settle_after_ms`), a thread's context is reset to a
//! resume card that holds just enough to pick it up again. History keeps everything.

use anyhow::{Result, ensure};
use context::Message;
use inference::providers::Api;
use memory::Resume;
use state::{Event, Thread};

use crate::{Experience, card_input};

const RESUME_INPUT_CHARS: usize = 24_000;
/// Heads the context of a settled thread.
pub const RESUME_HANDOFF: &str =
    "Where this conversation left off (it went quiet for a while; pick it up from here):";

impl<A: Api> Experience<A> {
    /// Writes the thread's resume card and resets its context to it. `None` when nothing
    /// finished since the last card.
    #[tracing::instrument(skip_all)]
    pub async fn settle(&self, thread: &mut Thread) -> Result<Option<Resume>> {
        ensure!(!thread.in_turn(), "can't settle during a turn");
        let previous = self.memory.resume(&thread.id)?;
        let previous = previous.as_ref().map(|r| (r.text.as_str(), r.to));
        let Some((end, input)) = card_input(thread, previous, "Previous card", RESUME_INPUT_CHARS)
        else {
            return Ok(None);
        };
        let checkpoints = &thread.checkpoints()[..=end];
        let active_at = checkpoints
            .iter()
            .rev()
            .find(|c| matches!(c.event, Event::Message(_)))
            .map_or(checkpoints[end].at, |c| c.at);
        let text = self.ask(RESUME_PROMPT, input).await?;
        let resume = self.memory.add_resume(
            &thread.id,
            self.scopes.write.clone(),
            text.trim(),
            end,
            thread.turn(),
            active_at,
        )?;
        let len = thread.context().messages.len();
        let handoff = Message::user(format!("{RESUME_HANDOFF}\n{}", resume.text));
        thread.replace(0..len, vec![handoff])?;
        tracing::info!(resume = %resume.text, replaced = len, "settled");
        Ok(Some(resume))
    }
}

const RESUME_PROMPT: &str =
    "Write a resume card for this conversation, so it can be picked up later from the card alone.
Use these headings:
About: <one line>
Where we left off: <the last thing being worked on, and its state>
Open loops: <questions, next steps or promises still pending; \"none\" if none>
Key details: <names, numbers, decisions, links and preferences needed to continue>
Be concrete and brief (under 200 words). Plain text only.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{experience, key, turn};
    use context::Role;

    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn settles_into_a_card() {
        let exp = experience(&key());
        let mut thread = Thread::new("kyoto");
        turn(
            &mut thread,
            "Plan a trip to Kyoto in April.",
            "Cherry blossom season; book early.",
        );
        turn(
            &mut thread,
            "Find a ryokan near Gion.",
            "Try Gion Hatanaka.",
        );
        let resume = exp
            .settle(&mut thread)
            .await
            .unwrap()
            .expect("a resume card");
        println!("{}", resume.text);

        assert_eq!(
            (resume.to, resume.turns),
            (thread.checkpoints().len() - 2, 2)
        );
        assert!(resume.text.contains("Gion"));
        assert_eq!(exp.memory().resume("kyoto").unwrap(), Some(resume));
        let messages = &thread.context().messages;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].text().starts_with(RESUME_HANDOFF));
        assert_eq!(messages[0].role, Role::User);
        // Nothing new since: no second card.
        assert!(exp.settle(&mut thread).await.unwrap().is_none());
    }
}
