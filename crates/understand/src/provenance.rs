//! Who wrote what (docs/DATA_MODEL.md, layer 3).
//!
//! - Commits come from git and are attributed to an agent when the evidence allows: the agent edited
//!   the commit's files since the previous commit ("edited"), a co-author trailer names it
//!   ("trailer"), or one of its sessions was active here just before ("session", a likely guess).
//!   Otherwise a commit belongs to its git author.
//! - `git blame` says which commit wrote each current line; line spans turn that into "which commits
//!   wrote this function". Uncommitted lines are recorded too, and credited to the agent that last
//!   edited the file, if any.
//! Blame only re-runs for files that changed, so this is cheap to call from the background loop.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use engram_core::meta::{get_meta, set_meta};
use engram_core::rusqlite::{OptionalExtension, params};
use engram_core::{Connection, Result};
use serde::Serialize;

const UNCOMMITTED: &str = "uncommitted";
/// How far back before a commit an active agent session counts as "likely wrote it".
const SESSION_WINDOW_MIN: i64 = 30;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn utc(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.with_timezone(&Utc))
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Agent edits (from session events) in `[from, to]`, as (agent, session, file relative to root).
fn agent_edits(conn: &Connection, root: &Path, from: &str, to: &str) -> Result<Vec<(String, String, String)>> {
    let root = root.to_string_lossy().to_string();
    let mut stmt = conn.prepare(
        "SELECT json_extract(payload, '$.agent'), json_extract(payload, '$.session'), f.value
           FROM events, json_each(json_extract(payload, '$.tools')) t, json_each(json_extract(t.value, '$.files')) f
          WHERE kind = 'agent.turn' AND ts > ?1 AND ts <= ?2",
    )?;
    let rows = stmt.query_map(params![from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = vec![];
    for row in rows {
        let (agent, session, file) = row?;
        let rel = file.strip_prefix(&root).map(|f| f.trim_start_matches('/').to_string()).unwrap_or(file);
        out.push((agent, session, rel));
    }
    Ok(out)
}

/// Pull new commits and attribute each one.
fn sync_commits(conn: &Connection, root: &Path) -> Result<usize> {
    let last = get_meta(conn, "prov.commits_head")?;
    let range = last.as_deref().map(|l| format!("{l}..HEAD")).unwrap_or_else(|| "HEAD".into());
    let Some(bodies) = git(root, &["log", "--reverse", "--no-merges", "--format=%H%x1f%aI%x1f%an%x1f%s%x1f%b%x1e", &range]) else { return Ok(0) };
    let files_out = git(root, &["log", "--no-merges", "--name-only", "--format=@@%H", &range]).unwrap_or_default();
    let mut files: HashMap<String, Vec<String>> = HashMap::new();
    let mut current = String::new();
    for line in files_out.lines() {
        if let Some(sha) = line.strip_prefix("@@") {
            current = sha.to_string();
        } else if !line.is_empty() {
            files.entry(current.clone()).or_default().push(line.to_string());
        }
    }
    let mut previous: Option<DateTime<Utc>> = conn
        .query_row("SELECT MAX(ts) FROM commits", [], |r| r.get::<_, Option<String>>(0))?
        .as_deref()
        .and_then(utc);
    let mut added = 0;
    let mut head = None;
    for record in bodies.split('\u{1e}').map(str::trim).filter(|r| !r.is_empty()) {
        let parts: Vec<&str> = record.split('\u{1f}').collect();
        let [sha, date, author, subject, body] = parts[..] else { continue };
        let Some(when) = utc(date) else { continue };
        let changed = files.remove(sha).unwrap_or_default();
        let since = previous.unwrap_or(when - Duration::hours(6)).max(when - Duration::days(2));
        let (mut agent, mut session, mut evidence) = (None, None, None);

        // 1. The agent that edited these files since the previous commit.
        let mut votes: HashMap<(String, String), usize> = HashMap::new();
        for (a, s, f) in agent_edits(conn, root, &iso(since), &iso(when))? {
            if changed.iter().any(|c| c == &f || c.ends_with(&format!("/{f}")) || f.ends_with(&format!("/{c}"))) {
                *votes.entry((a, s)).or_default() += 1;
            }
        }
        if let Some(((a, s), _)) = votes.into_iter().max_by_key(|(_, n)| *n) {
            (agent, session, evidence) = (Some(a), Some(s), Some("edited"));
        }
        // 2. A co-author trailer naming an agent.
        if agent.is_none() {
            let lower = body.to_lowercase();
            for (needle, name) in [("co-authored-by: claude", "claude-code"), ("co-authored-by: codex", "codex"), ("co-authored-by: muse", "muse-code")] {
                if lower.contains(needle) {
                    (agent, evidence) = (Some(name.to_string()), Some("trailer"));
                }
            }
        }
        // 3. An agent session active here just before the commit (a likely guess, labelled as such).
        if agent.is_none() {
            let recent: Option<(String, String)> = conn
                .query_row(
                    "SELECT json_extract(payload, '$.agent'), json_extract(payload, '$.session') FROM events
                      WHERE kind = 'agent.turn' AND ts > ?1 AND ts <= ?2
                      GROUP BY 1, 2 ORDER BY COUNT(*) DESC LIMIT 1",
                    params![iso(when - Duration::minutes(SESSION_WINDOW_MIN)), iso(when)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((a, s)) = recent {
                (agent, session, evidence) = (Some(a), Some(s), Some("session"));
            }
        }
        conn.execute(
            "INSERT OR REPLACE INTO commits (sha, ts, author, subject, files, agent, session, evidence) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![sha, iso(when), author, subject, serde_json::to_string(&changed)?, agent, session, evidence],
        )?;
        previous = Some(when);
        head = Some(sha.to_string());
        added += 1;
    }
    if let Some(h) = head {
        set_meta(conn, "prov.commits_head", &h)?;
    }
    Ok(added)
}

/// Which commit wrote each line of the working copy of `file` (index = line - 1).
fn blame(root: &Path, file: &str) -> Option<Vec<String>> {
    let out = git(root, &["blame", "--line-porcelain", "-w", "--", file])?;
    let mut lines = vec![];
    for l in out.lines() {
        let mut parts = l.split(' ');
        if let (Some(sha), Some(_orig), Some(fin)) = (parts.next(), parts.next(), parts.next()) {
            if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) && fin.parse::<usize>().is_ok() {
                lines.push(if sha.chars().all(|c| c == '0') { UNCOMMITTED.to_string() } else { sha.to_string() });
            }
        }
    }
    Some(lines)
}

fn store_blame(conn: &Connection, file: &str, lines: &[String]) -> Result<()> {
    let tally = |from: usize, to: usize| -> HashMap<&str, i64> {
        let mut m = HashMap::new();
        for sha in lines.iter().take(to).skip(from.saturating_sub(1)) {
            *m.entry(sha.as_str()).or_default() += 1;
        }
        m
    };
    let mut targets: Vec<(String, usize, usize)> = vec![];
    if let Some(id) = conn.query_row("SELECT id FROM entities WHERE kind = 'file' AND qualified_name = ?", [file], |r| r.get::<_, String>(0)).optional()? {
        targets.push((id, 1, lines.len()));
    }
    let mut stmt = conn.prepare("SELECT entity_id, start_line, end_line FROM code_symbols WHERE file = ?")?;
    for row in stmt.query_map([file], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize, r.get::<_, i64>(2)? as usize)))? {
        targets.push(row?);
    }
    for (id, from, to) in targets {
        conn.execute("DELETE FROM code_history WHERE entity_id = ?", [&id])?;
        for (sha, n) in tally(from, to) {
            conn.execute("INSERT INTO code_history (entity_id, commit_sha, lines) VALUES (?, ?, ?)", params![id, sha, n])?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct ProvStats {
    pub commits_added: usize,
    pub files_blamed: usize,
}

/// Bring commits and line authorship up to date for `files` (re-blaming only changed ones).
pub fn refresh(conn: &Connection, root: &Path, files: &[String]) -> Result<ProvStats> {
    let mut stats = ProvStats { commits_added: sync_commits(conn, root)?, ..Default::default() };
    let head = git(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    for file in files {
        let Ok(meta) = std::fs::metadata(root.join(file)) else { continue };
        let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
        let key = format!("{mtime}-{}-{}", meta.len(), head.trim());
        let meta_key = format!("prov.blame:{file}");
        if get_meta(conn, &meta_key)?.as_deref() == Some(key.as_str()) {
            continue;
        }
        if let Some(lines) = blame(root, file) {
            store_blame(conn, file, &lines)?;
            stats.files_blamed += 1;
        }
        set_meta(conn, &meta_key, &key)?;
    }
    Ok(stats)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub sha: String,
    pub ts: Option<String>,
    pub who: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributor {
    /// "Claude Code", "Codex", or the git author's name.
    pub who: String,
    pub lines: i64,
    pub is_agent: bool,
    /// edited | trailer | session (likely) | git
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub agent: String,
    pub session: String,
    /// The transcript document for the session, when indexed.
    pub document_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub lines: i64,
    pub contributors: Vec<Contributor>,
    pub created: Option<Change>,
    pub last_changed: Option<Change>,
    pub commits: usize,
    pub sessions: Vec<SessionRef>,
}

pub fn agent_label(agent: &str) -> String {
    match agent {
        "claude-code" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "muse-code" => "Muse Code".into(),
        other => other.into(),
    }
}

fn session_doc_key(agent: &str, session: &str) -> String {
    let id = match agent {
        "claude-code" => "claude",
        "muse-code" => "muse",
        other => other,
    };
    format!("session:{id}:{session}")
}

/// Who wrote an entity's current lines, when it was created and last changed, and which agent
/// sessions were involved.
pub fn provenance(conn: &Connection, entity_id: &str) -> Result<Provenance> {
    let mut stmt = conn.prepare(
        "SELECT h.commit_sha, h.lines, c.ts, c.author, c.subject, c.agent, c.session, c.evidence
           FROM code_history h LEFT JOIN commits c ON c.sha = h.commit_sha WHERE h.entity_id = ?",
    )?;
    type Row = (String, i64, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);
    let rows: Vec<Row> = stmt
        .query_map([entity_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    let mut p = Provenance::default();
    let mut by: HashMap<(String, bool, String), i64> = HashMap::new();
    let mut sessions: Vec<SessionRef> = vec![];
    let mut dated: Vec<Change> = vec![];
    for (sha, lines, ts, author, subject, agent, session, evidence) in rows {
        p.lines += lines;
        let (who, is_agent, ev) = match (&agent, sha.as_str()) {
            (Some(a), _) => (agent_label(a), true, evidence.clone().unwrap_or_else(|| "edited".into())),
            (None, UNCOMMITTED) => ("uncommitted".to_string(), false, "working copy".into()),
            (None, _) => (author.clone().unwrap_or_else(|| "unknown".into()), false, "git".into()),
        };
        *by.entry((who.clone(), is_agent, ev)).or_default() += lines;
        if let (Some(a), Some(s)) = (&agent, &session) {
            if !sessions.iter().any(|x| &x.session == s) {
                let document_id = conn
                    .query_row("SELECT id FROM documents WHERE external_id = ?", [session_doc_key(a, s)], |r| r.get::<_, String>(0))
                    .optional()?;
                sessions.push(SessionRef { agent: agent_label(a), session: s.clone(), document_id });
            }
        }
        if sha != UNCOMMITTED {
            p.commits += 1;
            dated.push(Change { sha: sha.chars().take(7).collect(), ts, who, subject: subject.unwrap_or_default() });
        }
    }
    dated.sort_by(|a, b| a.ts.cmp(&b.ts));
    p.created = dated.first().cloned();
    p.last_changed = dated.last().cloned();
    let mut contributors: Vec<Contributor> = by.into_iter().map(|((who, is_agent, evidence), lines)| Contributor { who, lines, is_agent, evidence }).collect();
    contributors.sort_by(|a, b| b.lines.cmp(&a.lines));
    p.contributors = contributors;
    p.sessions = sessions;
    Ok(p)
}

/// One line for agents: "written by Claude Code (62 lines, edited), Ashish (8); created 2026-10-28 in
/// 3f2a1c9, last changed 2026-10-31 in 9b8e7d6 by Codex".
pub fn provenance_line(p: &Provenance) -> Option<String> {
    if p.lines == 0 {
        return None;
    }
    let who: Vec<String> = p
        .contributors
        .iter()
        .take(3)
        .map(|c| {
            let likely = if c.evidence == "session" { ", likely" } else { "" };
            format!("{} ({} lines{likely})", c.who, c.lines)
        })
        .collect();
    let day = |c: &Change| c.ts.as_deref().map_or(String::new(), |t| t.chars().take(10).collect());
    let mut s = format!("written by {}", who.join(", "));
    if let Some(c) = &p.created {
        s.push_str(&format!("; created {} in {}", day(c), c.sha));
    }
    if let Some(c) = &p.last_changed {
        s.push_str(&format!("; last changed {} in {} by {}", day(c), c.sha, c.who));
    }
    Some(s)
}
