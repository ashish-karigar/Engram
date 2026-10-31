//! Find memories that cost tokens without helping: changelog lines posing as decisions,
//! near-duplicates, raw identifiers, and low-confidence guesses. Suggestions only; a person
//! decides, and archiving is reversible.

use engram_core::memory::Memory;
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use serde::Serialize;

use crate::reconcile::similarity;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weak {
    pub memory: Memory,
    /// `changelog`, `duplicate`, `identifier` or `low_confidence`.
    pub reason: String,
    pub detail: String,
}

/// Memories at least this similar (Jaccard over title and claim words) say the same thing.
const DUPLICATE: f64 = 0.5;
const LOW_CONFIDENCE: f64 = 0.6;
const CHANGE_VERBS: &[&str] = &[
    "add", "added", "adds", "build", "built", "create", "created", "implement", "implemented", "port", "ported", "update",
    "updated", "remove", "removed", "fix", "fixed", "rename", "renamed", "move", "moved", "migrate", "migrated",
];
const CREDENTIAL_WORDS: &[&str] = &["client id", "client_id", "api key", "apikey", "secret", "token", "password"];

fn has_reason(m: &Memory) -> bool {
    m.rationale.as_deref().is_some_and(|r| {
        let r = r.trim().to_lowercase();
        !r.is_empty() && r != m.claim.trim().to_lowercase() && !r.contains("durable truth") && !r.starts_with("this is a")
    })
}

/// A decision with no reason whose claim reads like a commit subject ("Added X", "X was added").
fn is_changelog(m: &Memory) -> bool {
    if m.kind != "decision" || has_reason(m) {
        return false;
    }
    let claim = m.claim.to_lowercase();
    let first = claim.split_whitespace().next().unwrap_or("");
    CHANGE_VERBS.contains(&first)
        || ["was", "were"].iter().any(|aux| CHANGE_VERBS.iter().any(|v| v.ends_with('d') && claim.contains(&format!("{aux} {v}"))))
}

/// A long token mixing letters and digits (keys, client ids, hashes) next to a credential word.
fn has_identifier(m: &Memory) -> bool {
    let text = format!("{} {}", m.title, m.claim).to_lowercase();
    let opaque = text.split(|c: char| !c.is_alphanumeric()).any(|t| {
        t.len() >= 20 && t.chars().any(|c| c.is_ascii_digit()) && t.chars().any(|c| c.is_ascii_alphabetic())
    });
    opaque && CREDENTIAL_WORDS.iter().any(|w| text.contains(w))
}

/// Which of two near-duplicates to keep: the one with a reason, then higher confidence, then older.
fn keeps(a: &Memory, b: &Memory) -> bool {
    (has_reason(a), a.confidence, std::cmp::Reverse(&a.observed_at)) >= (has_reason(b), b.confidence, std::cmp::Reverse(&b.observed_at))
}

pub fn weak_memories(conn: &Connection) -> Result<Vec<Weak>> {
    let mut stmt = conn.prepare("SELECT id FROM memories WHERE status = 'current' ORDER BY observed_at")?;
    let ids: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let memories = engram_core::memory::get_memories(conn, &ids)?;
    let mut out: Vec<Weak> = vec![];
    let flagged = |out: &Vec<Weak>, id: &str| out.iter().any(|w| w.memory.id == id);

    for m in &memories {
        let reason = if has_identifier(m) {
            Some(("identifier", "Holds a raw identifier or credential-like value; keep those in config, not memory.".to_string()))
        } else if is_changelog(m) {
            Some(("changelog", "Restates a change without a reason; git history already has it.".to_string()))
        } else if m.confidence < LOW_CONFIDENCE {
            Some(("low_confidence", format!("Confidence {:.2}.", m.confidence)))
        } else {
            None
        };
        if let Some((reason, detail)) = reason {
            out.push(Weak { memory: m.clone(), reason: reason.into(), detail });
        }
    }
    for (i, a) in memories.iter().enumerate() {
        for b in &memories[i + 1..] {
            let (text_a, text_b) = (format!("{} {}", a.title, a.claim), format!("{} {}", b.title, b.claim));
            if similarity(&text_a, &text_b) < DUPLICATE {
                continue;
            }
            let (keep, drop) = if keeps(a, b) { (a, b) } else { (b, a) };
            if !flagged(&out, &drop.id) && !flagged(&out, &keep.id) {
                out.push(Weak { memory: drop.clone(), reason: "duplicate".into(), detail: format!("Says the same as “{}”.", keep.title) });
            }
        }
    }
    Ok(out)
}

/// Archive (or restore) memories. Archived memories leave search and the brief but keep their history.
pub fn set_archived(conn: &Connection, ids: &[String], archived: bool) -> Result<usize> {
    let mut n = 0;
    for id in ids {
        let status: Option<String> = conn.query_row("SELECT status FROM memories WHERE id = ?", params![id], |r| r.get(0)).ok();
        match (status.as_deref(), archived) {
            (Some("current"), true) => engram_core::memory::set_status(conn, id, "archived")?,
            (Some("archived"), false) => engram_core::memory::set_status(conn, id, "current")?,
            _ => continue,
        }
        n += 1;
    }
    Ok(n)
}
