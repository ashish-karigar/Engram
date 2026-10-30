use engram_core::Result;
use serde_json::Value;

use crate::llm::{LlmClient, LlmRequest};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: String,
    pub title: String,
    pub claim: String,
    pub rationale: Option<String>,
    /// Verbatim text from the evidence that supports the claim.
    pub quote: String,
    pub evidence: Vec<i64>,
    pub files: Vec<String>,
    pub confidence: f64,
}

const ALLOWED: &[&str] = &["decision", "rejected_option", "dead_end", "fact", "preference"];

pub const SYSTEM_PROMPT: &str = r#"You extract durable engineering memory from a log of coding-agent and developer activity.
Return ONLY JSON: {"memories":[{"type","title","claim","rationale","quote","evidence","files","confidence"}]}.
- type: decision | rejected_option | dead_end | fact | preference
- decision: something chosen, with WHY in "rationale". rejected_option: an alternative considered and not chosen, with why. dead_end: an attempt that failed.
- quote: a short VERBATIM snippet copied from one cited event that supports the claim. Never invent quotes.
- evidence: the event numbers (the #N in the log) you used.
- files: project file paths involved. confidence: 0..1.
Skip trivia, formatting, and anything you cannot ground in the log. An empty list is a good answer."#;

pub fn extract_candidates(llm: &dyn LlmClient, digest: &str) -> Result<Vec<Candidate>> {
    let raw = llm.complete(&LlmRequest {
        system: SYSTEM_PROMPT.into(),
        prompt: format!("Activity log:\n\n{digest}"),
        max_tokens: 2500,
    })?;
    Ok(parse_candidates(&raw))
}

/// Tolerant parser: finds the JSON object in the reply, drops items that are malformed.
pub fn parse_candidates(raw: &str) -> Vec<Candidate> {
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else { return vec![] };
    if end <= start {
        return vec![];
    }
    let Ok(data) = serde_json::from_str::<Value>(&raw[start..=end]) else { return vec![] };
    data["memories"].as_array().map(|list| list.iter().filter_map(to_candidate).collect()).unwrap_or_default()
}

fn non_empty(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn to_candidate(item: &Value) -> Option<Candidate> {
    let kind = non_empty(&item["type"]).filter(|k| ALLOWED.contains(&k.as_str()))?;
    let evidence: Vec<i64> = item["evidence"].as_array()?.iter().filter_map(Value::as_i64).collect();
    if evidence.is_empty() {
        return None;
    }
    Some(Candidate {
        kind,
        title: non_empty(&item["title"])?,
        claim: non_empty(&item["claim"])?,
        quote: non_empty(&item["quote"])?,
        rationale: non_empty(&item["rationale"]),
        files: item["files"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect()).unwrap_or_default(),
        confidence: item["confidence"].as_f64().map_or(0.5, |c| c.clamp(0.0, 1.0)),
        evidence,
    })
}
