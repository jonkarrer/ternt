//! Starter detectors, ported from pi-durable-demo. Each looks for one kind of trigger. Tune them
//! against the labelled cases in the tests: add a case for every miss or false alarm you see.

// One span covering the whole text is a list of one range, not a range of numbers.
#![allow(clippy::single_range_in_vec_init)]

use context::{Block, Role};
use state::{Checkpoint, Event, Thread};

use crate::{Detector, Intent, Pattern, PatternDetector, Signal, Source};

pub fn defaults() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(explicit_memory()),
        Box::new(notice()),
        Box::new(standing_preference()),
        Box::new(identity()),
        Box::new(correction()),
        Box::new(reference_to_past()),
        Box::new(TurnStart),
        Box::new(Failure),
        Box::new(Repetition),
    ]
}

/// "remember that…", "don't forget", "keep in mind": the user asks to keep something.
pub fn explicit_memory() -> PatternDetector {
    PatternDetector::new(
        "explicit-memory",
        Intent::Retain,
        0.9,
        "asked to remember something",
    )
    .sources(&[Source::User])
    .pattern(
        Pattern::new(r"(?i)\bremember\s+(?:that|this|to|my|our|i|we|me)\b").not_after(
            r"(?i)(?:\b(?:do|did)\s+you|\b(?:don'?t|do not|can'?t|cannot|couldn'?t))\s+$",
        ),
    )
    .pattern(r"(?i)\b(?:don'?t|do not|never)\s+forget\b")
    .pattern(r"(?i)\bkeep (?:this |that |it )?in mind\b")
    .pattern(r"(?i)\b(?:make a |take a )?note (?:that|of this|of that|this down)\b")
    .pattern(r"(?i)\bfor future reference\b")
    .pattern(r"(?i)\b(?:save|store|add|put) (?:this|that|it) (?:to|in|into) (?:your )?memory\b")
    .pattern(r"(?i)\bmemori[sz]e\b")
}

/// "FYI", "heads up", "note:": the user flags information, less explicitly than "remember".
pub fn notice() -> PatternDetector {
    PatternDetector::new("notice", Intent::Retain, 0.6, "flags information as worth knowing")
        .sources(&[Source::User])
        .pattern(r"(?i)\b(?:fyi|heads[- ]up|just so you know|for (?:your )?information|good to know|worth knowing|be aware that)\b")
        .pattern(r"(?i)(?:^|[.!?]\s+)note\s*[:\-—]")
}

/// Standing preferences and habits: "I prefer…", "from now on…", "always use…".
pub fn standing_preference() -> PatternDetector {
    PatternDetector::new("standing-preference", Intent::Retain, 0.7, "states a lasting preference")
        .sources(&[Source::User])
        .pattern(r"(?i)\b(?:I|we)\s+(?:always|never|usually|generally|typically)\s+\w+")
        .pattern(r"(?i)\b(?:I|we)\s+(?:really\s+)?(?:prefer|hate|dislike|can'?t stand)\b")
        .pattern(r"(?i)\b(?:I'?d|I would|we'?d|we would)\s+(?:much\s+)?(?:rather|prefer)\b")
        .pattern(r"(?i)\b(?:from now on|going forward|moving forward|in (?:the )?future|by default)\b")
        // "Always use…", but not "Never mind".
        .pattern(
            Pattern::new(r"(?i)(?:^|[.!?]\s+)(?:please\s+)?(?:always|never)\s+")
                .not_before(r"(?i)^(?:mind\b|\W|$)"),
        )
        .pattern(r"(?i)\buse\s+[\w.-]+\s+instead of\b")
        .pattern(r"(?i)\b(?:don'?t|do not|never) (?:ever )?use\b")
        .pattern(r"(?i)\b(?:in|for|across) (?:all|every|each|any)\s+(?:[\w-]+\s+)?(?:files?|commits?|messages?|responses?|answers?|PRs?|pull requests?|projects?|repos?|scripts?|tests?)\b")
        .pattern(r"(?i)(?:^|[.!?]\s+)(?:please\s+)?stop\s+\w+ing\b")
        .pattern(r"(?i)\b(?:keep|make)\s+(?:your\s+)?(?:responses|answers|replies|explanations|code|messages|things)\s+(?:short|brief|concise|simple|terse|minimal)\b")
        .pattern(r"(?i)\b(?:be|stay)\s+(?:more\s+)?(?:concise|brief|terse)\b")
}

/// Facts about the user or their world: name, role, team, stack.
pub fn identity() -> PatternDetector {
    PatternDetector::new("identity", Intent::Retain, 0.6, "states a fact about the user or their work")
        .sources(&[Source::User])
        .pattern(r"(?i)\bmy name(?:'s| is)\b")
        .pattern(r"\b[Cc]all me\s+\p{Lu}\p{Ll}+")
        .pattern(r"(?i)\bI(?:'m| am)\s+(?:a|an)\s+(?:[\w-]+\s+)?(?:engineer|developer|designer|manager|student|founder|researcher|scientist|lead|architect|consultant|teacher|writer|analyst|administrator)\b")
        .pattern(r"(?i)\bI (?:work|am working|'m working) (?:at|for)\b")
        .pattern(r"(?i)\b(?:my|our) (?:team|company|org|organi[sz]ation|stack|project|repo|codebase|startup|product|ci|infra|infrastructure|backend|frontend|database|db|api|app|service|cluster|servers?|monorepo|pipeline) (?:is|uses|runs|has|builds|works|lives|deploys)\b")
        .pattern(
            Pattern::new(r"(?i)\bwe (?:use|run|deploy (?:to|on)|host (?:on|with)|build with)\s+\w+")
                .not_after(r"(?i)\b(?:can|could|should|would|shall|if|do|how)\s+$"),
        )
        .pattern(r"(?i)\bI(?:'m| am) (?:based|located) in\b")
        .pattern(r"(?i)\bI live in\b")
        .pattern(r"(?i)\bmy (?:pronouns|timezone|time zone) (?:are|is)\b")
        .pattern(r"(?i)\bI(?:'m| am) (?:on|using|running) (?:a\s+)?(?:Mac|macOS|Windows|Linux|Ubuntu|Debian|Fedora|Arch|NixOS|WSL)\b")
        .pattern(r"\bI(?:'m| am) \p{Lu}\p{Ll}+\s*[,.!]")
        .pattern(r"(?i)\bI (?:lead|run|manage|own|maintain)\s+(?:the|a|our)\s+[\w-]+(?:\s+[\w-]+)?\s+(?:team|project|service|repo|group)\b")
}

/// The user says something is wrong or has changed.
pub fn correction() -> PatternDetector {
    PatternDetector::new("correction", Intent::Refine, 0.7, "corrects or updates something")
        .sources(&[Source::User])
        .pattern(r"(?i)^\s*(?:no|nope|wrong|incorrect)\b\s*[,.!:\-—–]")
        .pattern(r"(?i)\bthat(?:'s| is| was) (?:not (?:right|correct|true|what I (?:said|asked|meant|wanted))|wrong|incorrect)\b")
        .pattern(r"(?i)\byou(?:'re| are) wrong\b")
        .pattern(r"(?i)\bI meant\b")
        .pattern(r"(?i)\bnot\s+[\w.-]+(?:\s+[\w.-]+)?\s*,\s*(?:but|it'?s)\b")
        .pattern(r"(?i)\bcorrection\s*:")
        .pattern(r"(?i)\b(?:that|this|it)(?:'s| is| was) (?:outdated|out of date|no longer (?:true|correct|the case|accurate))\b")
        .pattern(r"(?i)\b(?:we|I) (?:no longer|don'?t|do not) use\b")
        .pattern(r"(?i)\b(?:we|I) (?:switched|moved|migrated|changed) (?:over )?(?:to|from|off)\b")
        .pattern(r"(?i)\bnot quite\b")
        .pattern(r"(?i)\b(?:that'?s|that is|it'?s) (?:the )?(?:old|outdated|stale|previous|wrong)\s+\w+")
        .pattern(r"(?i)\bit(?:'s| is) [^.?!]{1,40}\bnow\b")
        .pattern(r"(?i)^\s*(?:actually|wait|hmm|oops|on second thought)\b")
        .pattern(r"(?i)\bscratch that\b")
}

/// Mentions of shared history: "like last time", "what did we decide".
pub fn reference_to_past() -> PatternDetector {
    PatternDetector::new("reference-to-past", Intent::Recall, 0.8, "refers to something from before")
        .sources(&[Source::User])
        .pattern(r"(?i)\b(?:do|did) you (?:remember|recall)\b")
        .pattern(r"(?i)\bremember when\b")
        .pattern(r"(?i)\b(?:like|as|same as) (?:last time|before|we did|you did|I said|I mentioned|we discussed|usual)\b")
        .pattern(r"(?i)\b(?:last|previous|earlier|other) (?:time|session|conversation|chat)\b")
        .pattern(r"(?i)\bwe (?:talked about|discussed|decided|agreed|went over|covered)\b")
        .pattern(r"(?i)\b(?:my|our|the) usual\b")
        .pattern(r"(?i)\b(?:what|which|where|how|when) (?:was|were|did) (?:we|I)\b")
        .pattern(r"(?i)\bwhat(?:'s| is) my\b")
        .pattern(r"(?i)\b(?:I|we) (?:already )?(?:told|showed|gave) you\b")
        .pattern(r"(?i)\bwhat did you (?:say|suggest|recommend|tell me)\b")
        .pattern(r"(?i)\b(?:from|since)\s+(?:yesterday|last (?:week|time|session)|earlier|before|this morning)\b")
        .pattern(r"(?i)\bsame\s+(?:setup|config|configuration|approach|way|format|style|structure|pattern|thing)\s+as\b")
        .pattern(r"(?i)\b(?:pull|bring|dig) up\s+(?:the|my|our)\b")
}

/// The user's message opening each turn: a low-salience nudge so every turn gets a quick
/// memory check.
pub struct TurnStart;

impl Detector for TurnStart {
    fn name(&self) -> &str {
        "turn-start"
    }

    fn detect(&self, thread: &Thread, checkpoint: &Checkpoint) -> Vec<Signal> {
        let Event::Message(message) = &checkpoint.event else {
            return Vec::new();
        };
        let opens_turn = checkpoint.id > 0
            && thread.checkpoints()[checkpoint.id - 1].event == Event::TurnStarted;
        let text = message.text();
        if !opens_turn || message.role != Role::User || text.trim().is_empty() {
            return Vec::new();
        }
        vec![Signal {
            detector: self.name().into(),
            intent: Intent::Recall,
            salience: 0.2,
            checkpoint: checkpoint.id,
            source: Source::User,
            spans: vec![0..text.len()],
            text,
            reason: "new turn".into(),
        }]
    }
}

/// A tool call failed: possibly a lesson worth keeping.
pub struct Failure;

impl Detector for Failure {
    fn name(&self) -> &str {
        "failure"
    }

    fn detect(&self, _thread: &Thread, checkpoint: &Checkpoint) -> Vec<Signal> {
        let Event::Message(message) = &checkpoint.event else {
            return Vec::new();
        };
        message
            .content
            .iter()
            .filter_map(|block| match block {
                Block::ToolResult {
                    content,
                    is_error: true,
                    ..
                } => Some(Signal {
                    detector: self.name().into(),
                    intent: Intent::Retain,
                    salience: 0.5,
                    checkpoint: checkpoint.id,
                    source: Source::Tool,
                    spans: vec![0..content.len()],
                    text: content.clone(),
                    reason: "a tool call failed".into(),
                }),
                _ => None,
            })
            .collect()
    }
}

/// The same sequence of tool calls as an earlier turn: a candidate procedure.
pub struct Repetition;

impl Detector for Repetition {
    fn name(&self) -> &str {
        "repetition"
    }

    fn detect(&self, thread: &Thread, checkpoint: &Checkpoint) -> Vec<Signal> {
        if checkpoint.event != Event::TurnEnded {
            return Vec::new();
        }
        let sequence = tool_sequence(thread, checkpoint.turn);
        if sequence.len() < 2 {
            return Vec::new();
        }
        let earlier = (1..checkpoint.turn)
            .filter(|&turn| tool_sequence(thread, turn) == sequence)
            .count();
        if earlier == 0 {
            return Vec::new();
        }
        let text = sequence.join(" → ");
        vec![Signal {
            detector: self.name().into(),
            intent: Intent::Retain,
            salience: (0.5 + 0.1 * earlier as f32).min(0.9),
            checkpoint: checkpoint.id,
            source: Source::State,
            spans: vec![0..text.len()],
            text,
            reason: format!(
                "same tool sequence as {earlier} earlier turn{}",
                if earlier == 1 { "" } else { "s" }
            ),
        }]
    }
}

/// The tools a turn called, in order.
fn tool_sequence(thread: &Thread, turn: u32) -> Vec<String> {
    thread
        .checkpoints()
        .iter()
        .filter(|c| c.turn == turn)
        .filter_map(|c| match &c.event {
            Event::Message(m) => Some(m.tool_calls().map(|(_, name, _)| name.to_string())),
            _ => None,
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attention;
    use context::Message;
    use serde_json::json;

    fn user_turn(text: &str) -> Thread {
        let mut thread = Thread::new("t");
        thread.start_turn(Message::user(text)).unwrap();
        thread
    }

    /// Labelled cases from pi-durable-demo: text, and the pattern detectors that should fire.
    const CASES: &[(&str, &[&str])] = &[
        // explicit-memory
        (
            "Remember that I use tabs, not spaces.",
            &["explicit-memory"],
        ),
        (
            "Please remember to run the linter before committing.",
            &["explicit-memory"],
        ),
        (
            "Can you remember that our staging URL is staging.example.com?",
            &["explicit-memory"],
        ),
        (
            "Don't forget the API key lives in Doppler.",
            &["explicit-memory"],
        ),
        (
            "Keep in mind the client is on Windows.",
            &["explicit-memory"],
        ),
        (
            "Make a note that the deploy window is Tuesdays.",
            &["explicit-memory"],
        ),
        (
            "For future reference, the bucket is called assets-prod.",
            &["explicit-memory"],
        ),
        (
            "Save this to memory: prod DB is read-only on weekends.",
            &["explicit-memory"],
        ),
        ("Memorize this port: 8443.", &["explicit-memory"]),
        // standing-preference
        ("I prefer pnpm over npm.", &["standing-preference"]),
        (
            "From now on, write commit messages in the imperative mood.",
            &["standing-preference"],
        ),
        (
            "Always use double quotes in TypeScript.",
            &["standing-preference"],
        ),
        ("Never push directly to main.", &["standing-preference"]),
        ("Use vitest instead of jest.", &["standing-preference"]),
        ("We usually deploy on Fridays.", &["standing-preference"]),
        (
            "I'd rather you ask before deleting files.",
            &["standing-preference"],
        ),
        ("Don't use any for types.", &["standing-preference"]),
        (
            "By default, answer in British English.",
            &["standing-preference"],
        ),
        // identity
        ("My name is Jon.", &["identity"]),
        ("Call me Sam.", &["identity"]),
        ("I'm a backend engineer.", &["identity"]),
        ("I work at a small startup.", &["identity"]),
        ("Our stack is Bun, Postgres and Nix.", &["identity"]),
        ("We deploy to Cloudflare Workers.", &["identity"]),
        ("I'm based in Ohio.", &["identity"]),
        // correction
        ("No, the port is 8080.", &["correction"]),
        ("That's wrong — it was released in 2021.", &["correction"]),
        ("That's not what I asked.", &["correction"]),
        ("I meant the staging server.", &["correction"]),
        ("Not npm, but pnpm.", &["correction"]),
        ("Correction: the meeting is Thursday.", &["correction"]),
        ("That's outdated, we moved off Heroku.", &["correction"]),
        ("We switched to Postgres last month.", &["correction"]),
        ("We no longer use Redis.", &["correction"]),
        // reference-to-past
        (
            "Do you remember the bug with the login page?",
            &["reference-to-past"],
        ),
        ("Set it up like last time.", &["reference-to-past"]),
        (
            "What did we decide about the schema?",
            &["reference-to-past"],
        ),
        (
            "We discussed this in the previous session.",
            &["reference-to-past"],
        ),
        ("Give me my usual summary.", &["reference-to-past"]),
        ("What's my preferred editor?", &["reference-to-past"]),
        ("I told you the deadline already.", &["reference-to-past"]),
        // notice
        (
            "Heads up, the staging db gets wiped every night.",
            &["notice"],
        ),
        ("FYI the build takes about ten minutes.", &["notice"]),
        (
            "Note: this file is generated, don't edit by hand.",
            &["notice"],
        ),
        // first held-out round
        ("btw I'm on a Mac, so use brew commands", &["identity"]),
        (
            "Use 2-space indentation in all YAML files.",
            &["standing-preference"],
        ),
        (
            "Stop adding emojis to commit messages.",
            &["standing-preference"],
        ),
        ("Please keep responses short.", &["standing-preference"]),
        (
            "Hmm, not quite. The bucket is in us-east-2.",
            &["correction"],
        ),
        (
            "wait, that's the old endpoint — it's /v3 now",
            &["correction"],
        ),
        ("Actually, make it a POST request.", &["correction"]),
        (
            "Can you pull up the notes from yesterday?",
            &["reference-to-past"],
        ),
        (
            "Same setup as the auth service please",
            &["reference-to-past"],
        ),
        ("I'm Jordan, I lead the platform team", &["identity"]),
        ("Our CI runs on GitHub Actions.", &["identity"]),
        ("How did you get that number?", &[]),
        (
            "Remember, this is a breaking change, bump the major version.",
            &[],
        ),
        ("The user said they always get logged out.", &[]),
        ("Can you do it like this instead: map then filter?", &[]),
        ("I love how clean this is", &[]),
        // typographic apostrophes, non-ASCII names, dashes
        (
            "Don’t forget the API key lives in Doppler.",
            &["explicit-memory"],
        ),
        ("I’m a backend engineer.", &["identity"]),
        (
            "I’d rather you ask before deleting files.",
            &["standing-preference"],
        ),
        ("That’s wrong — it was released in 2021.", &["correction"]),
        ("What’s my preferred editor?", &["reference-to-past"]),
        ("I don’t remember that error.", &[]),
        ("Call me Émile.", &["identity"]),
        ("I’m Zoë.", &["identity"]),
        ("No — the port is 8080.", &["correction"]),
        // several at once
        (
            "Remember that I prefer tabs.",
            &["explicit-memory", "standing-preference"],
        ),
        (
            "No, I meant pnpm — from now on use pnpm.",
            &["correction", "standing-preference"],
        ),
        // should not fire
        ("What is the capital of France?", &[]),
        ("It always fails on CI, can you look?", &[]),
        ("Write a function that sums an array.", &[]),
        ("Can we use Redis for caching here?", &[]),
        ("How do we deploy this?", &[]),
        ("Never mind, I figured it out.", &[]),
        ("I like this answer, thanks.", &[]),
        ("I don't remember the exact error.", &[]),
        ("The test failed with a timeout.", &[]),
        ("Explain how React hooks work.", &[]),
        ("Fix the typo in README.md.", &[]),
        ("What was the capital of Prussia?", &[]),
        ("Is this the right approach?", &[]),
        ("Let's refactor the parser.", &[]),
    ];

    #[test]
    fn labelled_cases() {
        let detectors = [
            explicit_memory(),
            notice(),
            standing_preference(),
            identity(),
            correction(),
            reference_to_past(),
        ];
        let mut failures = Vec::new();
        for (text, expected) in CASES {
            let thread = user_turn(text);
            let message = &thread.checkpoints()[1];
            // Each detector on its own: Attention merges same-intent signals, hiding one.
            let mut fired: Vec<&str> = detectors
                .iter()
                .filter(|d| !d.detect(&thread, message).is_empty())
                .map(|d| d.name.as_str())
                .collect();
            fired.sort();
            let mut expected = expected.to_vec();
            expected.sort();
            if fired != expected {
                failures.push(format!("{text:?}: expected {expected:?}, got {fired:?}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn attention_merges_and_ranks() {
        let thread = user_turn("Remember that my name is Jon. What's my usual order?");
        let signals = Attention::default().attend(&thread, 0);
        let summary: Vec<_> = signals
            .iter()
            .map(|s| (s.detector.as_str(), s.intent, s.salience))
            .collect();
        // explicit-memory and identity are both retain on the same text: merged, 0.9 kept.
        // reference-to-past (0.8) and turn-start (0.2) are both recall: merged, 0.8 kept.
        assert_eq!(
            summary,
            [
                ("explicit-memory", Intent::Retain, 0.9),
                ("reference-to-past", Intent::Recall, 0.8),
            ]
        );
        assert_eq!(signals[0].spans.len(), 2);
        assert_eq!(
            signals[0].reason,
            "asked to remember something; states a fact about the user or their work"
        );
    }

    #[test]
    fn attend_starts_where_asked() {
        let mut thread = user_turn("My name is Jon.");
        let seen = thread.checkpoints().len();
        thread.push(Message::assistant("Hi Jon.")).unwrap();
        assert!(Attention::default().attend(&thread, seen).is_empty());
    }

    fn tool_turn(thread: &mut Thread, tools: &[&str], fail: bool) {
        thread.start_turn(Message::user("do the thing")).unwrap();
        for (i, name) in tools.iter().enumerate() {
            thread.start_step().unwrap();
            thread
                .push(Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolCall {
                        id: format!("c{i}"),
                        name: (*name).into(),
                        input: json!({}),
                    }],
                })
                .unwrap();
            thread
                .push(Message {
                    role: Role::User,
                    content: vec![Block::ToolResult {
                        call_id: format!("c{i}"),
                        content: "done".into(),
                        is_error: fail,
                    }],
                })
                .unwrap();
        }
        thread.end_turn().unwrap();
    }

    #[test]
    fn repetition_and_failure() {
        let mut thread = Thread::new("t");
        tool_turn(&mut thread, &["build", "deploy"], false);
        let after_first = thread.checkpoints().len();
        assert!(
            Attention::new(vec![Box::new(Repetition)])
                .attend(&thread, 0)
                .is_empty()
        );
        tool_turn(&mut thread, &["build", "deploy"], false);
        let signals = Attention::new(vec![Box::new(Repetition)]).attend(&thread, after_first);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].text, "build → deploy");
        assert_eq!(signals[0].reason, "same tool sequence as 1 earlier turn");

        let mut thread = Thread::new("t");
        tool_turn(&mut thread, &["build"], true);
        let signals = Attention::new(vec![Box::new(Failure)]).attend(&thread, 0);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].source, Source::Tool);
    }
}
