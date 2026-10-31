use std::collections::HashMap;

use crate::extract::Candidate;

/// Where a candidate's quote was found, and how exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Grounding {
    /// The events the quote really comes from (models often cite the wrong number).
    pub evidence: Vec<i64>,
    /// false when the quote matched near-verbatim rather than exactly; confidence is lowered.
    pub exact: bool,
}

const MARKUP: [char; 15] = ['*', '_', '`', '#', '>', '~', '[', ']', '(', ')', '"', '\'', '“', '”', '’'];

/// Lowercase, drop Markdown emphasis and quote marks, collapse whitespace, so `**voxel grid**`
/// in the log matches `voxel grid` in the model's quote.
fn plain(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if MARKUP.contains(&c) { ' ' } else { c.to_ascii_lowercase() }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Longest common subsequence of two word lists, O(n·m). Measures "the same words, in order".
fn lcs(a: &[String], b: &[String]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    for x in a {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { prev[j + 1].max(cur[j]) };
        }
        prev = cur;
    }
    prev[b.len()]
}

const NEAR_VERBATIM: f64 = 0.8;
const MIN_QUOTE_WORDS: usize = 5;

/// Grounding check: a memory is kept only if its quote comes from the log it was given.
/// 1. exact quote (ignoring case and Markdown) in a cited event;
/// 2. exact quote in any other event of the task (the citation is corrected);
/// 3. near-verbatim: ≥ 80% of the quote's words, in order, within one event of the task.
/// Anything else (invented or loosely paraphrased) is rejected.
pub fn verify_candidate(c: &Candidate, by_seq: &HashMap<i64, String>) -> Result<Grounding, &'static str> {
    let quote = plain(&c.quote);
    if quote.is_empty() {
        return Err("empty quote");
    }
    let mut events: Vec<(&i64, &String)> = by_seq.iter().collect();
    events.sort_by_key(|(seq, _)| (!c.evidence.contains(seq), **seq));

    if let Some((seq, _)) = events.iter().find(|(_, text)| plain(text).contains(&quote)) {
        return Ok(Grounding { evidence: vec![**seq], exact: true });
    }
    let q = words(&c.quote);
    if q.len() >= MIN_QUOTE_WORDS {
        let best = events
            .iter()
            .map(|(seq, text)| (**seq, lcs(&q, &words(text)) as f64 / q.len() as f64))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((seq, _)) = best.filter(|(_, r)| *r >= NEAR_VERBATIM) {
            return Ok(Grounding { evidence: vec![seq], exact: false });
        }
    }
    Err("quote not found in the task's events")
}
