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

pub const SYSTEM_PROMPT: &str = r#"You maintain the long-term engineering memory of a software project. You read a log of
activity (commits, file changes, and user/agent conversation turns) and record only what a developer
or coding agent joining next month would need to know.

Return ONLY JSON: {"memories":[{"type","title","claim","rationale","quote","evidence","files","confidence"}]}

Record:
- decision: a technical choice that was made, with WHY in "rationale" (e.g. "Use a voxel grid for lidar downsampling" because "radius filtering cost 80 ms per frame").
- rejected_option: an alternative that was considered and turned down, and why.
- dead_end: an approach that was tried and failed, and why.
- fact: a durable truth about how the system works (architecture, data flow, an interface contract, a constraint).
- preference: a standing rule from the user about how to work (e.g. "never add co-author lines to commits").

Do NOT record: requests ("the user asked for X"), approvals, status updates, progress reports, greetings,
one-off tasks, plans that were not carried out, or anything that will not still be true next week.

Rules:
- "quote" must be copied character-for-character from ONE event you cite, 5 to 25 words. If you cannot quote it, do not record it.
- "evidence": the event numbers (#N) the quote and claim come from.
- "rationale": the project's own reason, as stated in the log (e.g. "the mobile app cannot hold cookies").
  Leave it empty when the log gives no reason. Never explain why you chose to record the memory.
- "title": a short noun phrase naming the topic (e.g. "Auth strategy"), so a later change to the same topic can replace it.
- "confidence": 0.5 if inferred, 0.8 if stated, 1.0 only if stated and confirmed by a commit.
Returning {"memories": []} is correct when nothing qualifies. Fewer, better memories beat many weak ones."#;

pub fn extract_candidates(llm: &dyn LlmClient, digest: &str) -> Result<Vec<Candidate>> {
    let raw = llm.complete(&LlmRequest {
        system: SYSTEM_PROMPT.into(),
        prompt: format!("Activity log:\n\n{digest}"),
        max_tokens: 2500,
    })?;
    Ok(parse_candidates(&raw))
}

/// Tolerant parser: finds the JSON object in the reply, drops items that are malformed.
/// Models differ in how they wrap the answer: `{"memories": [...]}`, a bare list, or either inside
/// a fenced code block with prose around it. Accept all of them.
pub fn parse_candidates(raw: &str) -> Vec<Candidate> {
    let mut shapes = [('{', '}'), ('[', ']')];
    shapes.sort_by_key(|(open, _)| raw.find(*open).unwrap_or(usize::MAX));
    let parsed = shapes.iter().find_map(|(open, close)| {
        let (start, end) = (raw.find(*open)?, raw.rfind(*close)?);
        (end > start).then(|| serde_json::from_str::<Value>(&raw[start..=end]).ok()).flatten()
    });
    let list = match parsed {
        Some(Value::Array(items)) => items,
        Some(obj) => ["memories", "items", "results"].iter().find_map(|k| obj[*k].as_array().cloned()).unwrap_or_default(),
        None => vec![],
    };
    list.iter().filter_map(to_candidate).collect()
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
