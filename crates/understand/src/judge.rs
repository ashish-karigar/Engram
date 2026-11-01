//! Model-judged links between memories. Word overlap (`reconcile`) links a memory the moment it
//! is saved; this pass asks the model, later and in the background, how each memory relates to
//! its nearest neighbours, so "backend moved to AWS" meets "backend is hosted on Oracle" even
//! though they share few words.
//!
//! The model only labels pairs we show it, and code decides what a label may change:
//! - nothing is deleted: a replaced memory is superseded and stays as history;
//! - a link it is unsure of waits for review instead of being applied;
//! - a contradiction is never resolved by the model, only flagged for a person;
//! - facts derived from the code (`system:` scope) change only when the code does;
//! - a memory with no related neighbour stays on its own;
//! - every judgment keeps the model's name and reason, and a failed call changes nothing.

use std::collections::HashSet;

use engram_core::db::{new_id, now, tx};
use engram_core::graph::{EdgeOpts, add_edge};
use engram_core::memory::{Memory, SearchOpts, get_memories, get_memory, reinforce, search_memories, set_status};
use engram_core::rusqlite::{OptionalExtension, params};
use engram_core::{Connection, Error, Result};
use serde::Serialize;
use serde_json::Value;

use crate::llm::{LlmClient, LlmRequest};

pub const RELATIONS: &[&str] = &["same", "replaces", "extends", "depends_on", "contradicts"];

/// At or above this a link is applied; below it, down to `REVIEW`, it waits for a person.
const APPLY: f64 = 0.75;
/// `same` and `replaces` retire a memory, so they need more certainty.
const APPLY_RETIRING: f64 = 0.85;
/// Below this a proposed link is dropped.
const REVIEW: f64 = 0.55;
/// Neighbours shown to the model per memory: other memories, and facts derived from the code.
const NEIGHBOURS: usize = 8;
const CODE_FACTS: usize = 3;

pub const SYSTEM_PROMPT: &str = r#"You connect the engineering memories of one software project. You get one memory, M, and a
numbered list of candidate memories (C1, C2, ...). For each candidate that is related to M, say how.

Relations:
- same: states the same fact or decision as M; only the wording differs.
- replaces: the same topic, but one changes or reverses the other (e.g. "hosted on Oracle" and
  "moved hosting to AWS"; "use REST" and "switched to gRPC"). Which is newer is decided from the dates.
- extends: adds a detail (a limit, schedule, setting, identity, mechanism) about the specific thing the
  candidate states, without changing it (e.g. "uploads are capped at 25 MB" extends "users upload
  avatars to S3").
- depends_on: M relies on the candidate; if the candidate changed, M would have to change too.
- contradicts: the two claim incompatible things and neither clearly replaces the other.

Candidates marked "from code" were derived from the project's code and configuration.

Rules:
- Shared words or a shared component are not a relation: "the API returns errors as RFC 7807 JSON" and
  "the API runs on port 8080" are unrelated, as are two different options on the same screen.
- When unsure, leave the candidate out. Most candidates are unrelated; returning none is normal.
- "reason": one sentence naming the specific point the two share or disagree on. Refer to the
  memories by what they say, never as "M" or "C1".
- "confidence": 0.6 if plausible, 0.8 if clear, 0.95 only if the text leaves no doubt.

Return ONLY JSON: {"links":[{"id":"C2","relation":"extends","reason":"...","confidence":0.8}]}"#;

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// The candidate's memory id.
    pub dst: String,
    pub relation: String,
    pub reason: String,
    pub confidence: f64,
}

fn is_code_fact(m: &Memory) -> bool {
    m.scope.as_deref().is_some_and(|s| s.starts_with("system:"))
}

fn describe(label: &str, m: &Memory) -> String {
    let date = m.valid_from.get(..10).unwrap_or(&m.valid_from);
    let source = if is_code_fact(m) { " · from code" } else { "" };
    let why = m.rationale.as_deref().map(|r| format!(" Why: {r}")).unwrap_or_default();
    format!("{label}: [{} · {date}{source}] {}: {}{why}", m.kind, m.title, m.claim)
}

/// Ask the model how `memory` relates to each candidate. Labels for ids it was not shown are dropped.
pub fn judge(llm: &dyn LlmClient, memory: &Memory, candidates: &[Memory]) -> Result<Vec<Verdict>> {
    if candidates.is_empty() {
        return Ok(vec![]);
    }
    let raw = judge_raw(llm, memory, candidates)?;
    let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    Ok(parse_verdicts(&raw, &ids))
}

/// The model's reply as text, before parsing.
pub fn judge_raw(llm: &dyn LlmClient, memory: &Memory, candidates: &[Memory]) -> Result<String> {
    let listed: Vec<String> = candidates.iter().enumerate().map(|(i, c)| describe(&format!("C{}", i + 1), c)).collect();
    let prompt = format!("{}\n\nCandidates:\n{}", describe("M", memory), listed.join("\n"));
    // Muse Spark thinks before it answers and the thinking counts against this budget; a tight
    // budget returns an empty or cut-off reply on exactly the pairs that need thought.
    llm.complete(&LlmRequest { system: SYSTEM_PROMPT.into(), prompt, max_tokens: 8000 })
}

/// Tolerant parser, like `extract::parse_candidates`: finds the JSON in the reply and keeps only
/// well-formed labels for listed candidates, at most one per candidate.
pub fn parse_verdicts(raw: &str, ids: &[String]) -> Vec<Verdict> {
    let parsed = match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => serde_json::from_str::<Value>(&raw[start..=end]).ok(),
        _ => None,
    };
    let items = parsed.and_then(|v| v["links"].as_array().cloned()).unwrap_or_default();
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|item| {
            let label = item["id"].as_str()?.trim().trim_start_matches(['C', 'c']);
            let dst = ids.get(label.parse::<usize>().ok()?.checked_sub(1)?)?.clone();
            let relation = item["relation"].as_str()?.trim().to_lowercase();
            let reason = plain_reason(item["reason"].as_str()?.trim());
            if !RELATIONS.contains(&relation.as_str()) || reason.is_empty() || !seen.insert(dst.clone()) {
                return None;
            }
            Some(Verdict { dst, relation, reason, confidence: item["confidence"].as_f64().map_or(0.6, |c| c.clamp(0.0, 1.0)) })
        })
        .collect()
}

/// Reasons are shown to people, who never saw the prompt's "M" and "C3" labels.
fn plain_reason(reason: &str) -> String {
    static LABELS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let labels = LABELS.get_or_init(|| regex::Regex::new(r"\b(M|C\d+)\b").unwrap());
    labels.replace_all(reason, |c: &regex::Captures| if &c[1] == "M" { "one" } else { "the other" }).into_owned()
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RelinkReport {
    pub judged: usize,
    pub calls: usize,
    pub applied: usize,
    pub pending: usize,
    /// Applied replacements the current rules no longer allow, undone and sent to review.
    pub reopened: usize,
    pub error: Option<String>,
}

/// Neighbours by keyword search, for callers without vectors.
pub fn keyword_neighbours(conn: &Connection, m: &Memory) -> Vec<String> {
    let opts = SearchOpts { limit: Some(20), ..Default::default() };
    search_memories(conn, &format!("{} {}", m.title, m.claim), &opts).map(|hits| hits.into_iter().map(|h| h.memory.id).collect()).unwrap_or_default()
}

fn already_judged(conn: &Connection, a: &str, b: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM link_judgments WHERE (src = ?1 AND dst = ?2) OR (src = ?2 AND dst = ?1)", [a, b], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Judge up to `limit` memories that have not been judged yet, newest first. `neighbours` returns
/// candidate ids for a memory, best first. Stops at the first failed model call; those memories
/// are tried again next time.
///
/// Facts derived from the code are judged too: they are new only when the code changed (a
/// different host, auth provider, database), which is exactly when older notes may be outdated.
pub fn relink(conn: &Connection, llm: &dyn LlmClient, neighbours: &dyn Fn(&Memory) -> Vec<String>, limit: usize) -> Result<RelinkReport> {
    let mut stmt = conn.prepare(
        "SELECT id FROM memories m WHERE status = 'current'
           AND NOT EXISTS (SELECT 1 FROM memories_judged j WHERE j.memory_id = m.id)
         ORDER BY rowid DESC LIMIT ?",
    )?;
    let todo: Vec<String> = stmt.query_map([limit as i64], |r| r.get(0))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut report = RelinkReport::default();
    report.reopened = recheck(conn)?;
    let judged_by = llm.name();
    for id in todo {
        // An earlier memory in this pass may have retired this one.
        let Some(memory) = get_memory(conn, &id)?.filter(|m| m.status == "current") else {
            mark_judged(conn, &id, &judged_by, 0)?;
            continue;
        };
        let ids: Vec<String> = neighbours(&memory).into_iter().filter(|n| *n != memory.id).collect();
        let (mut others, mut facts) = (vec![], vec![]);
        for c in get_memories(conn, &ids)? {
            // Two code facts never disagree in a way a person should settle; the map rebuilds them.
            let both_code = is_code_fact(&memory) && is_code_fact(&c);
            if c.status != "current" || both_code || already_judged(conn, &memory.id, &c.id)? {
                continue;
            }
            let list = if is_code_fact(&c) { &mut facts } else { &mut others };
            list.push(c);
        }
        // `get_memories` may not keep the search order; restore it so the best neighbours are shown.
        let rank = |m: &Memory| ids.iter().position(|i| *i == m.id).unwrap_or(usize::MAX);
        others.sort_by_key(rank);
        facts.sort_by_key(rank);
        others.truncate(NEIGHBOURS);
        facts.truncate(CODE_FACTS);
        let candidates: Vec<Memory> = others.into_iter().chain(facts).collect();
        if candidates.is_empty() {
            mark_judged(conn, &memory.id, &judged_by, 0)?;
            report.judged += 1;
            continue;
        }
        report.calls += 1;
        let mut verdicts = match judge(llm, &memory, &candidates) {
            Ok(v) => v,
            Err(err) => {
                report.error = Some(err.to_string());
                break;
            }
        };
        // Conflicts first: a memory that disagrees with the code may not retire anything (`may_retire`).
        let disputes = |v: &Verdict| v.relation == "contradicts" || candidates.iter().any(|c| c.id == v.dst && is_code_fact(c));
        verdicts.sort_by_key(|v| !disputes(v));
        let mut links = 0;
        tx(conn, |conn| {
            for v in &verdicts {
                let Some(other) = candidates.iter().find(|c| c.id == v.dst) else { continue };
                match record(conn, &memory, other, v, &judged_by)? {
                    Some("applied") => report.applied += 1,
                    Some(_) => report.pending += 1,
                    None => continue,
                }
                links += 1;
            }
            mark_judged(conn, &memory.id, &judged_by, links)
        })?;
        report.judged += 1;
    }
    Ok(report)
}

fn mark_judged(conn: &Connection, id: &str, judged_by: &str, links: usize) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO memories_judged (memory_id, judged_by, judged_at, links) VALUES (?, ?, ?, ?)",
        params![id, judged_by, now(), links as i64],
    )?;
    Ok(())
}

/// What a verdict becomes: (relation, "applied" | "pending"), or None when it is too weak to keep.
/// `code` is true when either memory was derived from the code.
pub fn outcome(v: &Verdict, code: bool) -> Option<(&'static str, &'static str)> {
    // The code's facts are not replaced by what someone wrote, and what someone wrote is not
    // retired by the code without a person agreeing: a disagreement is a conflict to review.
    let relation = match v.relation.as_str() {
        "replaces" if code => "contradicts",
        "same" if code => "extends",
        r => RELATIONS.iter().find(|x| **x == r).copied().unwrap_or("extends"),
    };
    let needed = if matches!(relation, "same" | "replaces") { APPLY_RETIRING } else { APPLY };
    let status = match relation {
        _ if v.confidence < REVIEW => return None,
        "contradicts" => "pending",
        _ if v.confidence >= needed => "applied",
        _ => "pending",
    };
    Some((relation, status))
}

/// Store one verdict and apply it if it is certain enough. Returns the judgment's status, or
/// None when the verdict was too weak to keep.
fn record(conn: &Connection, m: &Memory, other: &Memory, v: &Verdict, judged_by: &str) -> Result<Option<&'static str>> {
    let Some((relation, mut status)) = outcome(v, is_code_fact(m) || is_code_fact(other)) else { return Ok(None) };
    if relation == "replaces" && status == "applied" {
        let (older, newer) = by_age(m, other);
        if !may_retire(conn, older, newer)? {
            status = "pending";
        }
    }
    conn.execute(
        "INSERT INTO link_judgments (id, src, dst, relation, reason, confidence, judged_by, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![new_id("lnk"), m.id, other.id, relation, v.reason, v.confidence, judged_by, status, now()],
    )?;
    if status == "applied" {
        apply(conn, m, other, relation, v.confidence)?;
    } else if relation == "contradicts" {
        // Shown in the graph while it waits; ended when someone resolves it.
        add_edge(conn, &m.id, &other.id, "contradicts", EdgeOpts { weight: Some(v.confidence), ..Default::default() })?;
    }
    Ok(Some(status))
}

/// Whether `newer` may retire `older` without a person agreeing. Being newer is not enough:
/// - a memory from conversation alone may not retire one a commit backs (chat plans change,
///   commits are what happened);
/// - a memory in an open conflict with the code may not retire anything.
fn may_retire(conn: &Connection, older: &Memory, newer: &Memory) -> Result<bool> {
    if newer.commit_sha.is_none() && older.commit_sha.is_some() {
        return Ok(false);
    }
    let disputed: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM link_judgments j JOIN memories x ON x.id = CASE WHEN j.src = ?1 THEN j.dst ELSE j.src END
           WHERE j.relation = 'contradicts' AND j.status = 'pending' AND ?1 IN (j.src, j.dst) AND x.scope LIKE 'system:%')",
        [&newer.id],
        |r| r.get(0),
    )?;
    Ok(!disputed)
}

/// Undo applied replacements that `may_retire` would not allow now (they were applied under
/// older rules) and send them to review. Returns how many were reopened.
fn recheck(conn: &Connection) -> Result<usize> {
    let mut stmt = conn.prepare("SELECT id, src, dst FROM link_judgments WHERE status = 'applied' AND relation = 'replaces'")?;
    let rows: Vec<(String, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut reopened = 0;
    for (id, src, dst) in rows {
        let (Some(a), Some(b)) = (get_memory(conn, &src)?, get_memory(conn, &dst)?) else { continue };
        let (older, newer) = by_age(&a, &b);
        if newer.status != "current" {
            continue;
        }
        if !may_retire(conn, older, newer)? {
            tx(conn, |conn| {
                revert(conn, &a, &b, "replaces")?;
                conn.execute("UPDATE link_judgments SET status = 'pending', decided_at = NULL WHERE id = ?", [&id])?;
                Ok(())
            })?;
            reopened += 1;
        } else if older.status == "current" {
            // Brought back by undoing a different link; this one still holds.
            retire(conn, older, newer)?;
        }
    }
    Ok(reopened)
}

/// Reverse `apply` for the same (src, dst) pair: a retired memory becomes current again and the
/// link is removed. Memories are never deleted, so nothing is lost either way.
fn revert(conn: &Connection, m: &Memory, other: &Memory, relation: &str) -> Result<()> {
    let (older, newer) = by_age(m, other);
    let drop_edge = |src: &str, dst: &str, kind: &str| conn.execute("DELETE FROM edges WHERE src = ? AND dst = ? AND type = ?", [src, dst, kind]);
    match relation {
        "same" => {
            conn.execute("UPDATE memories SET status = 'current', valid_to = NULL, updated_at = ? WHERE id = ? AND status = 'archived'", params![now(), newer.id])?;
            conn.execute("UPDATE memories SET reinforcement = MAX(0, reinforcement - 1) WHERE id = ?", [&older.id])?;
            drop_edge(&newer.id, &older.id, "duplicates")?;
        }
        "replaces" => {
            drop_edge(&newer.id, &older.id, "supersedes")?;
            // Another applied link may still replace it; then it stays history.
            let still_replaced: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM edges WHERE dst = ? AND type = 'supersedes' AND valid_to IS NULL)",
                [&older.id],
                |r| r.get(0),
            )?;
            if !still_replaced {
                conn.execute("UPDATE memories SET status = 'current', valid_to = NULL, updated_at = ? WHERE id = ? AND status = 'superseded'", params![now(), older.id])?;
            }
        }
        "extends" | "depends_on" => {
            drop_edge(&m.id, &other.id, relation)?;
        }
        other => return Err(Error::msg(format!("cannot undo a {other} link"))),
    }
    Ok(())
}

/// (older, newer) by when each became true; on a tie the memory being judged counts as newer.
fn by_age<'a>(m: &'a Memory, other: &'a Memory) -> (&'a Memory, &'a Memory) {
    if other.valid_from > m.valid_from { (m, other) } else { (other, m) }
}

fn apply(conn: &Connection, m: &Memory, other: &Memory, relation: &str, confidence: f64) -> Result<()> {
    let weight = EdgeOpts { weight: Some(confidence), ..Default::default() };
    match relation {
        "same" => {
            // Keep the first statement, count the repeat, retire the duplicate.
            let (older, newer) = by_age(m, other);
            reinforce(conn, &older.id)?;
            set_status(conn, &newer.id, "archived")?;
            add_edge(conn, &newer.id, &older.id, "duplicates", weight)?;
        }
        "replaces" => {
            let (older, newer) = by_age(m, other);
            retire(conn, older, newer)?;
        }
        "extends" | "depends_on" => {
            add_edge(conn, &m.id, &other.id, relation, weight)?;
        }
        other => return Err(Error::msg(format!("cannot apply {other} without a decision"))),
    }
    Ok(())
}

/// `newer` replaces `older`: the old one becomes history, as `memory::supersede` does.
fn retire(conn: &Connection, older: &Memory, newer: &Memory) -> Result<()> {
    if is_code_fact(older) {
        return Err(Error::msg("facts from the code change only when the code does"));
    }
    conn.execute(
        "UPDATE memories SET status = 'superseded', valid_to = ?, updated_at = ? WHERE id = ? AND status = 'current'",
        params![newer.valid_from, now(), older.id],
    )?;
    add_edge(conn, &newer.id, &older.id, "supersedes", EdgeOpts { valid_from: Some(newer.valid_from.clone()), ..Default::default() })?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingLink {
    pub id: String,
    /// pending, or applied (in `retired`).
    pub status: String,
    pub relation: String,
    pub reason: String,
    pub confidence: f64,
    pub judged_by: String,
    pub created_at: String,
    pub src: Memory,
    pub dst: Memory,
    /// True when that side was derived from the code.
    pub src_from_code: bool,
    pub dst_from_code: bool,
}

/// Links waiting for a person, contradictions first.
pub fn pending(conn: &Connection, limit: usize) -> Result<Vec<PendingLink>> {
    load_links(conn, "status = 'pending' ORDER BY relation = 'contradicts' DESC, created_at DESC", limit, true)
}

/// Memories the model retired on its own (replaced or duplicate), newest first, so a person can undo them.
pub fn retired(conn: &Connection, limit: usize) -> Result<Vec<PendingLink>> {
    load_links(conn, "status = 'applied' AND relation IN ('same', 'replaces') ORDER BY created_at DESC", limit, false)
}

fn load_links(conn: &Connection, filter: &str, limit: usize, both_current: bool) -> Result<Vec<PendingLink>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, src, dst, relation, reason, confidence, judged_by, created_at, status FROM link_judgments WHERE {filter} LIMIT ?"
    ))?;
    let rows: Vec<(String, String, String, String, String, f64, String, String, String)> = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)))?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    let mut out = vec![];
    for (id, src, dst, relation, reason, confidence, judged_by, created_at, status) in rows {
        let (Some(src), Some(dst)) = (get_memory(conn, &src)?, get_memory(conn, &dst)?) else { continue };
        // A pending link settled some other way already (one side was replaced or archived).
        if both_current && (src.status != "current" || dst.status != "current") {
            continue;
        }
        let (src_from_code, dst_from_code) = (is_code_fact(&src), is_code_fact(&dst));
        let reason = plain_reason(&reason);
        out.push(PendingLink { id, status, relation, reason, confidence, judged_by, created_at, src, dst, src_from_code, dst_from_code });
    }
    Ok(out)
}

/// Unresolved contradictions as (memory id, title, reason), for the brief and the map. The
/// subject is the memory someone wrote; the other side may be the code.
pub fn conflicts(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    Ok(pending(conn, 50)?
        .into_iter()
        .filter(|p| p.relation == "contradicts")
        .map(|p| {
            let (subject, other, from_code) = if p.src_from_code { (p.dst, p.src, true) } else { (p.src, p.dst, p.dst_from_code) };
            let source = if from_code { "the code" } else { "another memory" };
            (subject.id, subject.title, format!("conflicts with {source}, \"{}\" ({}): {}", other.claim, other.id, p.reason))
        })
        .collect())
}

/// A person's decision on a pending link:
/// - `approve`: apply the proposed link (not for contradictions, which need `keep`);
/// - `decline`: not related; the pair is never proposed again;
/// - `keep` with a memory id: for a contradiction, that memory stands and the other is replaced by it;
/// - `undo`: reverse an applied link (a retired memory comes back) and never propose it again.
pub fn decide(conn: &Connection, judgment: &str, action: &str, keep: Option<&str>) -> Result<()> {
    let (src, dst, relation, confidence, status): (String, String, String, f64, String) = conn
        .query_row("SELECT src, dst, relation, confidence, status FROM link_judgments WHERE id = ?", [judgment], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?
        .ok_or_else(|| Error::msg(format!("unknown link: {judgment}")))?;
    let (Some(a), Some(b)) = (get_memory(conn, &src)?, get_memory(conn, &dst)?) else {
        return Err(Error::msg("one of the memories no longer exists"));
    };
    if action == "undo" {
        if status != "applied" || relation == "contradicts" {
            return Err(Error::msg("only an applied link can be undone"));
        }
        return tx(conn, |conn| {
            revert(conn, &a, &b, &relation)?;
            conn.execute("UPDATE link_judgments SET status = 'declined', decided_at = ? WHERE id = ?", params![now(), judgment])?;
            Ok(())
        });
    }
    if status != "pending" {
        return Err(Error::msg(format!("this link was already {status}")));
    }
    tx(conn, |conn| {
        let next = match (action, relation.as_str()) {
            ("decline", _) => "declined",
            ("approve", "contradicts") => return Err(Error::msg("choose which memory to keep")),
            ("approve", r) => {
                apply(conn, &a, &b, r, confidence)?;
                "applied"
            }
            ("keep", "contradicts") => {
                let (kept, dropped) = match keep {
                    Some(k) if k == a.id => (&a, &b),
                    Some(k) if k == b.id => (&b, &a),
                    _ => return Err(Error::msg("`keep` must be one of the two memories")),
                };
                retire(conn, dropped, kept)?;
                "applied"
            }
            _ => return Err(Error::msg(format!("cannot {action} a {relation} link"))),
        };
        if relation == "contradicts" {
            conn.execute(
                "UPDATE edges SET valid_to = ? WHERE src = ? AND dst = ? AND type = 'contradicts' AND valid_to IS NULL",
                params![now(), a.id, b.id],
            )?;
        }
        conn.execute("UPDATE link_judgments SET status = ?, decided_at = ? WHERE id = ?", params![next, now(), judgment])?;
        Ok(())
    })
}
