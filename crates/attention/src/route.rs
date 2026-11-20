//! Route cues: what Attention notices in a message before it's placed in a thread. They only
//! find cues; Experience decides the route. English only, ported from pi-durable-demo.
//! Tune them against the labelled cases in the tests.

use std::ops::Range;

use regex::Regex;

use crate::merge_spans;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteCue {
    /// "new topic", "by the way": the user is starting something else.
    NewTopic,
    /// "back to the Metabase thing": names an earlier thread; the spans say which.
    ThreadReference,
    /// "yes, do that", "the second one": only makes sense in the last thread.
    Continuation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteSignal {
    pub detector: String,
    pub cue: RouteCue,
    pub salience: f32,
    pub text: String,
    /// What triggered it; for a thread reference, the words naming the thread.
    pub spans: Vec<Range<usize>>,
    pub reason: String,
}

impl RouteSignal {
    /// The text of the spans, joined: for a thread reference, the thread's name.
    pub fn spanned(&self) -> String {
        self.spans
            .iter()
            .map(|s| &self.text[s.clone()])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub trait RouteDetector: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, text: &str) -> Vec<RouteSignal>;
}

/// Patterns with a salience each; the signal takes the highest that matched. A pattern's
/// `ref` group, when present, is the span; otherwise the whole match.
pub struct CueDetector {
    pub name: String,
    pub cue: RouteCue,
    pub reason: String,
    pub patterns: Vec<(Regex, f32)>,
    /// Only messages this short, in words, are examined.
    pub max_length: Option<usize>,
}

impl RouteDetector for CueDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn detect(&self, text: &str) -> Vec<RouteSignal> {
        let text = text.trim();
        if text.is_empty() || self.max_length.is_some_and(|max| length_of(text) > max) {
            return Vec::new();
        }
        let mut spans = Vec::new();
        let mut salience: f32 = 0.0;
        for (regex, s) in &self.patterns {
            for caps in regex.captures_iter(text) {
                let span = caps
                    .name("ref")
                    .unwrap_or_else(|| caps.get(0).unwrap())
                    .range();
                if !span.is_empty() {
                    spans.push(span);
                    salience = salience.max(*s);
                }
            }
        }
        if spans.is_empty() {
            return Vec::new();
        }
        vec![RouteSignal {
            detector: self.name.clone(),
            cue: self.cue,
            salience,
            text: text.to_string(),
            spans: merge_spans(spans),
            reason: self.reason.clone(),
        }]
    }
}

fn length_of(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn defaults() -> Vec<Box<dyn RouteDetector>> {
    vec![
        Box::new(new_topic()),
        Box::new(thread_reference()),
        Box::new(continuation()),
    ]
}

fn patterns(list: &[(&str, f32)]) -> Vec<(Regex, f32)> {
    list.iter()
        .map(|(p, s)| (Regex::new(p).expect("valid pattern"), *s))
        .collect()
}

/// Strong: says so. Soft: "by the way", which may still be related.
pub fn new_topic() -> CueDetector {
    CueDetector {
        name: "new-topic".into(),
        cue: RouteCue::NewTopic,
        reason: "starts a new topic".into(),
        max_length: None,
        patterns: patterns(&[
            (
                r"(?i)\b(?:new|different|unrelated|separate|another|other) (?:topic|subject|question|matter)\b",
                0.9,
            ),
            (
                r"(?i)\b(?:chang(?:e|ing)|switch(?:ing)?) (?:of |the )?(?:topic|subject|gears)\b",
                0.9,
            ),
            (
                r"(?i)\b(?:on an?|in an?) (?:unrelated|different|separate|other) note\b",
                0.9,
            ),
            (
                r"(?i)\b(?:something|anything) (?:else|different|unrelated)\b",
                0.8,
            ),
            (
                r"(?i)^\s*(?:unrelated(?:ly)?|totally unrelated|random question)\b",
                0.9,
            ),
            (
                r"(?i)^\s*(?:by the way|btw|anyway|oh,? also|quick question)\b",
                0.5,
            ),
        ]),
    }
}

/// The words naming a thread: a few words, stopping at punctuation.
const REF: &str = r"(?P<ref>[^.?!,;:\n]{2,60}?)";
const THREAD_NOUN: &str = r"(?:thing|stuff|topic|discussion|conversation|chat|thread|plan|project|idea|question|research)";
/// Where a thread's name ends: a thread noun, punctuation, or the end. Matched, not looked
/// ahead at (Rust's regex has no lookahead); the name is the `ref` group either way.
const END: &str = r"(?:\s*[.?!,;:]|\s*$)";

pub fn thread_reference() -> CueDetector {
    let p = |s: String, salience| (s, salience);
    let list = [
        p(
            format!(
                r"(?i)\b(?:back to|return(?:ing)? to|going back to|continu(?:e|ing)(?: with| on)?|pick(?:ing)? (?:back )?up(?: on| with)?|resum(?:e|ing))\s+(?:the |our |that |my |this )?{REF}(?:\s+{THREAD_NOUN}\b|{END})"
            ),
            0.9,
        ),
        p(
            format!(
                r"(?i)\b(?:about|regarding|on|with) (?:the |that |our |my ){REF} (?:we|you|i) (?:talked|discussed|were|spoke|did|looked|chatted|worked|started)"
            ),
            0.8,
        ),
        p(
            format!(
                r"(?i)\bwhere (?:were|did) we (?:leave off |land |end up )?(?:on|with) (?:the |our |that )?{REF}{END}"
            ),
            0.9,
        ),
        p(
            format!(
                r"(?i)\b(?:remember|recall) (?:the |our |that ){REF} (?:{THREAD_NOUN}|we|you|i)\b"
            ),
            0.8,
        ),
        p(
            format!(
                r"(?i)\b(?:what (?:happened|came of|about)|how(?:'s| is| did| are)|any (?:update|news|progress)) (?:with |on |about |to )?(?:the |our |that |my ){REF}(?:\s+(?:going|go|turn out|coming along))?{END}"
            ),
            0.6,
        ),
    ];
    CueDetector {
        name: "thread-reference".into(),
        cue: RouteCue::ThreadReference,
        reason: "refers to an earlier conversation".into(),
        max_length: None,
        patterns: list
            .iter()
            .map(|(p, s)| (Regex::new(p).expect("valid pattern"), *s))
            .collect(),
    }
}

/// Short replies that only make sense where the talk left off.
pub fn continuation() -> CueDetector {
    CueDetector {
        name: "continuation".into(),
        cue: RouteCue::Continuation,
        reason: "only makes sense in the last conversation".into(),
        max_length: Some(8),
        patterns: patterns(&[
            (
                r"(?i)^\s*(?:yes|yeah|yep|yup|sure|ok(?:ay)?|no|nope|nah|please do|do it|do that|go ahead|go for it|sounds good|perfect|great|thanks?(?: you)?|that one|this one|the (?:first|second|third|last|other|cheaper|better) one|both|neither|why|how so|and then|more|continue|keep going|try again|the same)\b",
                0.8,
            ),
            (
                r"(?i)^\s*(?:and |but |so )?(?:it|that|this|those|they|them|he|she)\b",
                0.6,
            ),
            (
                r"(?i)^\s*what about (?:it|that|them|the other|the rest)\b",
                0.8,
            ),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attention;

    #[test]
    fn labelled_cues() {
        let attention = Attention::default();
        let cases: &[(&str, Option<(RouteCue, f32)>)] = &[
            (
                "New topic: help me plan a trip to Kyoto",
                Some((RouteCue::NewTopic, 0.9)),
            ),
            (
                "Totally unrelated, what's a good pasta recipe?",
                Some((RouteCue::NewTopic, 0.9)),
            ),
            (
                "On a different note, how do I renew my passport?",
                Some((RouteCue::NewTopic, 0.9)),
            ),
            (
                "Can I ask you something else?",
                Some((RouteCue::NewTopic, 0.8)),
            ),
            ("Let's change the subject", Some((RouteCue::NewTopic, 0.9))),
            (
                "By the way, what time is it in Tokyo?",
                Some((RouteCue::NewTopic, 0.5)),
            ),
            ("yes, do it", Some((RouteCue::Continuation, 0.8))),
            ("The second one", Some((RouteCue::Continuation, 0.8))),
            ("ok", Some((RouteCue::Continuation, 0.8))),
            (
                "what about the other one?",
                Some((RouteCue::Continuation, 0.8)),
            ),
            ("it's too expensive", Some((RouteCue::Continuation, 0.6))),
            ("How do I set up Metabase with Postgres?", None),
            ("Write me a haiku about autumn leaves falling softly", None),
            (
                "Yesterday I went to the store and bought apples, then I came home and cooked dinner for everyone",
                None,
            ),
            (
                "It was a long day at work and I need ideas for relaxing tonight without screens",
                None,
            ),
        ];
        let mut failures = Vec::new();
        for (text, expected) in cases {
            let got = attention.notice(text).first().map(|s| (s.cue, s.salience));
            if got != *expected {
                failures.push(format!("{text:?}: expected {expected:?}, got {got:?}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn thread_references_name_the_thread() {
        let attention = Attention::default();
        let cases = [
            (
                "Back to the Metabase thing — what did we decide?",
                "Metabase",
            ),
            ("Let's get back to the Kyoto trip.", "Kyoto trip"),
            ("Can we pick up on the cable research", "cable"),
            (
                "Where did we leave off on the dashboard migration?",
                "dashboard migration",
            ),
            (
                "About the cart we were looking at, is it still there?",
                "cart",
            ),
            ("Any update on the Metabase report?", "Metabase report"),
        ];
        let mut failures = Vec::new();
        for (text, name) in cases {
            let got = attention
                .notice(text)
                .into_iter()
                .find(|s| s.cue == RouteCue::ThreadReference)
                .map(|s| s.spanned());
            if got.as_deref() != Some(name) {
                failures.push(format!("{text:?}: expected {name:?}, got {got:?}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }
}
