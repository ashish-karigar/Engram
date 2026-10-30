use std::collections::HashMap;

use crate::extract::Candidate;

fn normalise(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Grounding check: a memory is accepted only if every cited event was in the task
/// and its quote appears verbatim (modulo whitespace and case) in one of them.
pub fn verify_candidate(c: &Candidate, by_seq: &HashMap<i64, String>) -> Result<(), &'static str> {
    let cited: Option<Vec<&String>> = c.evidence.iter().map(|seq| by_seq.get(seq)).collect();
    let cited = cited.ok_or("cites an event outside the task")?;
    let quote = normalise(&c.quote);
    if cited.iter().any(|text| normalise(text).contains(&quote)) {
        Ok(())
    } else {
        Err("quote not found in cited evidence")
    }
}
