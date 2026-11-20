//! Attention finds triggers. It looks at a thread's new checkpoints and raises a signal when
//! something may need a reaction from memory ("this may need a recall", "this may be worth
//! saving"). It never reads or writes memory and never decides; Experience does.
//!
//! It runs before every model call, so it must be cheap: detectors are plain synchronous code
//! (patterns, structure, counts). No model calls, no I/O.

pub mod detectors;
use std::ops::Range;

use context::{Block, Role};
use regex::Regex;
use state::{Checkpoint, Event, Thread};

/// What a signal suggests. Experience may act on it differently, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Something in memory may be relevant here.
    Recall,
    /// Something here may be worth keeping.
    Retain,
    /// Something here may correct or update what memory holds.
    Refine,
}

/// Whose text a signal was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    User,
    Assistant,
    Tool,
    /// The thread's structure (a repeated tool sequence), not anyone's words.
    State,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// The detector that raised it.
    pub detector: String,
    pub intent: Intent,
    /// 0–1: how strongly this calls for a reaction.
    pub salience: f32,
    /// The checkpoint it was found in.
    pub checkpoint: usize,
    pub source: Source,
    /// The text the detector examined.
    pub text: String,
    /// What triggered it inside `text`, as byte ranges.
    pub spans: Vec<Range<usize>>,
    /// Short and human-readable: why this fired.
    pub reason: String,
}

/// One trigger finder. Synchronous and side-effect free.
pub trait Detector: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, thread: &Thread, checkpoint: &Checkpoint) -> Vec<Signal>;
}

pub struct Attention {
    detectors: Vec<Box<dyn Detector>>,
    /// Signals below this salience are dropped.
    pub threshold: f32,
}

impl Default for Attention {
    fn default() -> Self {
        Self::new(detectors::defaults())
    }
}

impl Attention {
    pub fn new(detectors: Vec<Box<dyn Detector>>) -> Self {
        Self {
            detectors,
            threshold: 0.0,
        }
    }

    /// Runs every detector over checkpoints `from..`. Signals on the same text with the same
    /// intent are merged (spans combined, highest salience kept). Highest salience first.
    /// Empty when nothing needs attention, which should be the common case.
    pub fn attend(&self, thread: &Thread, from: usize) -> Vec<Signal> {
        let mut merged: Vec<Signal> = Vec::new();
        for checkpoint in thread.checkpoints().iter().skip(from) {
            for detector in &self.detectors {
                for signal in detector.detect(thread, checkpoint) {
                    if signal.salience < self.threshold {
                        continue;
                    }
                    let same = merged.iter_mut().find(|s| {
                        s.checkpoint == signal.checkpoint
                            && s.source == signal.source
                            && s.text == signal.text
                            && s.intent == signal.intent
                    });
                    let Some(same) = same else {
                        merged.push(signal);
                        continue;
                    };
                    same.spans.extend(signal.spans);
                    same.spans = merge_spans(std::mem::take(&mut same.spans));
                    same.reason = format!("{}; {}", same.reason, signal.reason);
                    if signal.salience > same.salience {
                        same.salience = signal.salience;
                        same.detector = signal.detector;
                    }
                }
            }
        }
        // Stable: equal salience keeps checkpoint order.
        merged.sort_by(|a, b| b.salience.total_cmp(&a.salience));
        merged
    }
}

/// The texts a checkpoint carries, by source. Only messages have any.
pub(crate) fn texts(checkpoint: &Checkpoint) -> Vec<(Source, String)> {
    let Event::Message(message) = &checkpoint.event else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let text = message.text();
    if !text.is_empty() {
        let source = match message.role {
            Role::User => Source::User,
            Role::Assistant => Source::Assistant,
        };
        out.push((source, text));
    }
    for block in &message.content {
        if let Block::ToolResult { content, .. } = block {
            out.push((Source::Tool, content.clone()));
        }
    }
    out
}

/// A regex, plus what must not come right before or after a match: Rust's `regex` has no
/// lookaround, so `not_after` is tested against the text before the match (anchor it with `$`)
/// and `not_before` against the text after it (anchor it with `^`). A `'` in any of them also
/// matches the typographic `’` that phones and editors type.
pub struct Pattern {
    regex: Regex,
    not_after: Option<Regex>,
    not_before: Option<Regex>,
}

impl Pattern {
    /// Panics on an invalid regex: patterns are written in code.
    pub fn new(regex: &str) -> Self {
        Self {
            regex: compile(regex),
            not_after: None,
            not_before: None,
        }
    }

    pub fn not_after(mut self, regex: &str) -> Self {
        self.not_after = Some(compile(regex));
        self
    }

    pub fn not_before(mut self, regex: &str) -> Self {
        self.not_before = Some(compile(regex));
        self
    }

    fn spans(&self, text: &str) -> impl Iterator<Item = Range<usize>> {
        self.regex.find_iter(text).filter_map(move |m| {
            let blocked = m.is_empty()
                || self
                    .not_after
                    .as_ref()
                    .is_some_and(|r| r.is_match(&text[..m.start()]))
                || self
                    .not_before
                    .as_ref()
                    .is_some_and(|r| r.is_match(&text[m.end()..]));
            (!blocked).then_some(m.range())
        })
    }
}

impl From<&str> for Pattern {
    fn from(regex: &str) -> Self {
        Pattern::new(regex)
    }
}

fn compile(regex: &str) -> Regex {
    Regex::new(&regex.replace('\'', "['’]")).expect("valid pattern")
}

/// The most common detector: any pattern matching a message's text fires.
pub struct PatternDetector {
    pub name: String,
    pub intent: Intent,
    pub salience: f32,
    pub reason: String,
    /// Which texts to test; empty means all.
    pub sources: Vec<Source>,
    pub patterns: Vec<Pattern>,
}

impl PatternDetector {
    pub fn new(name: &str, intent: Intent, salience: f32, reason: &str) -> Self {
        Self {
            name: name.into(),
            intent,
            salience,
            reason: reason.into(),
            sources: Vec::new(),
            patterns: Vec::new(),
        }
    }

    pub fn sources(mut self, sources: &[Source]) -> Self {
        self.sources = sources.to_vec();
        self
    }

    pub fn pattern(mut self, pattern: impl Into<Pattern>) -> Self {
        self.patterns.push(pattern.into());
        self
    }
}

impl Detector for PatternDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn detect(&self, _thread: &Thread, checkpoint: &Checkpoint) -> Vec<Signal> {
        texts(checkpoint)
            .into_iter()
            .filter(|(source, _)| self.sources.is_empty() || self.sources.contains(source))
            .filter_map(|(source, text)| {
                let spans: Vec<_> = self.patterns.iter().flat_map(|p| p.spans(&text)).collect();
                (!spans.is_empty()).then(|| Signal {
                    detector: self.name.clone(),
                    intent: self.intent,
                    salience: self.salience,
                    checkpoint: checkpoint.id,
                    source,
                    spans: merge_spans(spans),
                    text,
                    reason: self.reason.clone(),
                })
            })
            .collect()
    }
}

/// Sorted, with overlapping ranges joined.
pub(crate) fn merge_spans(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|s| s.start);
    let mut out: Vec<Range<usize>> = Vec::new();
    for span in spans {
        match out.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => out.push(span),
        }
    }
    out
}
