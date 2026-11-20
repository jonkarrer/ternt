//! Keyword matching between an input and threads' cards. Plain code, no model.

use std::collections::{HashMap, HashSet};

use crate::ThreadView;

/// Too common to tell threads apart: stopwords, and words people use to come back to one.
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "please", "so", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "to", "us", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "will", "with", "would", "you", "your", "about", "back",
    "thing", "stuff",
];

/// Search terms: words, lightly stemmed, without stopwords.
pub fn terms_of(text: &str) -> HashSet<String> {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2 && !STOP.contains(word))
        .map(|word| {
            ["ing", "ed", "es", "s"]
                .iter()
                .find_map(|suffix| word.strip_suffix(suffix).filter(|s| !s.is_empty()))
                .unwrap_or(word)
                .to_string()
        })
        .collect()
}

/// Threads by how much of the query they cover, rare terms weighted higher (IDF over the
/// threads). Scores 0–1, best first; ties keep the given order (most recent first).
pub fn rank_threads<'a>(
    query: &str,
    threads: &[&'a ThreadView],
    docs: &HashMap<String, String>,
) -> Vec<(&'a str, f64)> {
    let query = terms_of(query);
    if query.is_empty() || threads.is_empty() {
        return Vec::new();
    }
    let terms: Vec<(&str, HashSet<String>)> = threads
        .iter()
        .map(|t| (t.info.id.as_str(), terms_of(&docs[&t.info.id])))
        .collect();
    let n = terms.len() as f64;
    let idf: HashMap<&String, f64> = query
        .iter()
        .map(|t| {
            let df = terms.iter().filter(|(_, set)| set.contains(t)).count() as f64;
            (t, (1.0 + n / if df == 0.0 { 0.5 } else { df }).ln())
        })
        .collect();
    let total: f64 = idf.values().sum();
    let mut ranked: Vec<(&str, f64)> = terms
        .iter()
        .map(|(id, set)| {
            let covered: f64 = query
                .iter()
                .filter(|t| set.contains(*t))
                .map(|t| idf[t])
                .sum();
            (*id, covered / total)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_match_across_forms() {
        let terms = terms_of("Booking the trips to Kyoto");
        for t in ["book", "trip", "kyoto"] {
            assert!(terms.contains(t), "{t} in {terms:?}");
        }
        assert!(!terms.contains("the"));
    }
}
