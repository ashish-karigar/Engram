//! The activity timeline: the raw event log regrouped into what a person reads as history.
//!
//! Conversation turns become sessions (split where the agent was idle for an hour), file changes
//! become edit bursts, commits and new memories stay as single milestones. Background work
//! (snapshots, document indexing) is hidden unless asked for, and only counted.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use engram_core::events::{StoredEvent, events_between};
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use serde_json::{Value, json};

/// A pause longer than this starts a new block of the same session.
const SESSION_GAP: Duration = Duration::minutes(60);
/// File changes and background events closer than this are one burst.
const BURST_GAP: Duration = Duration::minutes(10);
const MESSAGE_CHARS: usize = 600;
const MAX_MESSAGES: usize = 300;
const SYSTEM_KINDS: &[&str] = &["snapshot.created", "document.ingested"];

pub struct ActivityQuery {
    /// Only activity before this time (ISO 8601); default now. Pass the previous page's `from`.
    pub before: Option<String>,
    pub days: i64,
    /// Include background events (snapshots, document indexing).
    pub system: bool,
}

fn parse(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|d| d.with_timezone(&Utc))
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// Paths relative to the project, so the timeline shows `src/app.rs`, not the full path.
fn relative(path: &str, root: &str) -> String {
    path.strip_prefix(root).map(|p| p.trim_start_matches('/')).unwrap_or(path).to_string()
}

/// Editors' temporary files from atomic saves (`.!123!App.tsx`, `App.tsx~`, `.App.tsx.swp`).
fn is_temp(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with(".!") || name.ends_with('~') || name.ends_with(".swp") || name.ends_with(".tmp")
}

/// The line that says what the user asked. Skips injected context (`<environment_context>`); when
/// an agent wraps attachments around the request (Codex: `# Files mentioned by the user:` …
/// `## My request:`), takes the line after that heading; otherwise the first non-heading line.
fn request_line(text: &str) -> Option<&str> {
    if text.starts_with('<') {
        return None;
    }
    let mut lines = text.lines().map(str::trim);
    if text.contains("## My request") {
        lines.find(|l| l.starts_with("## My request"));
    }
    lines.find(|l| !l.is_empty() && !l.starts_with('#'))
}

#[derive(Default)]
struct Session {
    agent: String,
    session: String,
    start: String,
    end: String,
    title: Option<String>,
    user: usize,
    assistant: usize,
    tools: usize,
    compactions: usize,
    files: BTreeSet<String>,
    messages: Vec<Value>,
}

impl Session {
    fn add(&mut self, e: &StoredEvent, root: &str) {
        let p = &e.payload;
        self.end = e.ts.clone();
        if e.kind == "agent.compacted" {
            self.compactions += 1;
            self.messages.push(json!({"ts": e.ts, "role": "compacted"}));
            return;
        }
        let role = p["role"].as_str().unwrap_or("assistant");
        let text = p["text"].as_str().unwrap_or("").trim();
        for tool in p["tools"].as_array().into_iter().flatten() {
            self.tools += 1;
            for f in tool["files"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|f| !is_temp(f)) {
                self.files.insert(relative(f, root));
            }
        }
        if text.is_empty() {
            return;
        }
        if role == "user" {
            self.user += 1;
            if self.title.is_none() {
                self.title = request_line(text).map(|l| clip(l, 140));
            }
        } else {
            self.assistant += 1;
        }
        if self.messages.len() < MAX_MESSAGES {
            self.messages.push(json!({"ts": e.ts, "role": role, "text": clip(text, MESSAGE_CHARS)}));
        }
    }

    fn into_item(self) -> Value {
        json!({
            "kind": "session", "agent": self.agent, "session": self.session, "start": self.start, "end": self.end,
            "title": self.title.unwrap_or_else(|| "Session without messages".into()),
            "messages": self.messages, "userMessages": self.user, "agentMessages": self.assistant,
            "toolCalls": self.tools, "compactions": self.compactions, "files": self.files,
        })
    }
}

/// Consecutive events of one kind of work, closer together than `BURST_GAP`.
struct Burst {
    start: String,
    end: String,
    files: BTreeSet<String>,
    counts: BTreeMap<String, usize>,
    titles: BTreeSet<String>,
}

fn add_to_bursts(bursts: &mut Vec<Burst>, e: &StoredEvent) -> usize {
    let close = bursts.last().is_some_and(|b| match (parse(&b.end), parse(&e.ts)) {
        (Some(end), Some(ts)) => ts - end <= BURST_GAP,
        _ => false,
    });
    if !close {
        bursts.push(Burst { start: e.ts.clone(), end: e.ts.clone(), files: BTreeSet::new(), counts: BTreeMap::new(), titles: BTreeSet::new() });
    }
    let burst = bursts.last_mut().expect("just pushed");
    burst.end = e.ts.clone();
    *burst.counts.entry(e.kind.clone()).or_default() += 1;
    bursts.len() - 1
}

/// Sessions marked as not the user's work (e.g. Codex's approval reviewer), see engram-sessions.
fn hidden_sessions(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare("SELECT json_extract(payload, '$.session') FROM events WHERE kind = 'agent.session_hidden'")?;
    let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
    Ok(rows.filter_map(|r| r.ok().flatten()).collect())
}

fn memories(conn: &Connection, from: &str, to: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.type, m.title, m.status, m.observed_at, a.name,
                (SELECT json_group_array(d.title) FROM edges e JOIN memories d ON d.id = e.dst
                  WHERE e.src = m.id AND e.type = 'supersedes')
           FROM memories m LEFT JOIN actors a ON a.id = m.authored_by
          WHERE m.observed_at >= ?1 AND m.observed_at < ?2",
    )?;
    let rows = stmt.query_map(params![from, to], |r| {
        let replaced: String = r.get(6)?;
        Ok(json!({
            "kind": "memory", "id": r.get::<_, String>(0)?, "type": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
            "status": r.get::<_, String>(3)?, "start": r.get::<_, String>(4)?, "end": r.get::<_, String>(4)?,
            "author": r.get::<_, Option<String>>(5)?,
            "replaces": serde_json::from_str::<Value>(&replaced).unwrap_or(json!([])),
        }))
    })?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

/// One page of history, newest first: everything in `[before - days, before)`.
pub fn activity(conn: &Connection, root: &str, q: &ActivityQuery) -> Result<Value> {
    let to = q.before.as_deref().and_then(parse).unwrap_or_else(|| Utc::now() + Duration::minutes(1));
    let from = to - Duration::days(q.days.clamp(1, 30));
    let (from, to) = (iso(from), iso(to));

    let mut items: Vec<Value> = Vec::new();
    let mut sessions: Vec<Session> = Vec::new();
    let mut open: HashMap<String, usize> = HashMap::new();
    let (mut edits, mut system) = (Vec::<Burst>::new(), Vec::<Burst>::new());
    let mut hidden = 0;

    let hidden_sessions = hidden_sessions(conn)?;
    for e in events_between(conn, &from, &to)? {
        let p = &e.payload;
        match e.kind.as_str() {
            "agent.turn" | "agent.compacted" => {
                let id = p["session"].as_str().unwrap_or("").to_string();
                if hidden_sessions.contains(&id) {
                    continue;
                }
                let stale = open.get(&id).is_some_and(|&i| match (parse(&sessions[i].end), parse(&e.ts)) {
                    (Some(end), Some(ts)) => ts - end > SESSION_GAP,
                    _ => false,
                });
                if stale || !open.contains_key(&id) {
                    open.insert(id.clone(), sessions.len());
                    let agent = p["agent"].as_str().unwrap_or("agent").to_string();
                    sessions.push(Session { agent, session: id.clone(), start: e.ts.clone(), ..Default::default() });
                }
                sessions[open[&id]].add(&e, root);
            }
            "git.commit" => items.push(json!({
                "kind": "commit", "start": e.ts, "end": e.ts, "sha": p["sha"], "subject": p["subject"], "author": p["author"],
                "files": p["files"].as_array().map(|f| f.iter().take(30).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "fileCount": p["files"].as_array().map_or(0, Vec::len),
            })),
            "file.change" => {
                let i = add_to_bursts(&mut edits, &e);
                for f in p["files"].as_array().into_iter().flatten() {
                    if let Some(path) = f["path"].as_str().or_else(|| f.as_str()).filter(|p| !is_temp(p)) {
                        edits[i].files.insert(relative(path, root));
                    }
                }
            }
            kind if SYSTEM_KINDS.contains(&kind) => {
                if !q.system {
                    hidden += 1;
                    continue;
                }
                let i = add_to_bursts(&mut system, &e);
                if let Some(title) = p["title"].as_str() {
                    system[i].titles.insert(title.to_string());
                }
            }
            _ => {}
        }
    }

    items.extend(sessions.into_iter().map(Session::into_item));
    items.extend(edits.into_iter().filter(|b| !b.files.is_empty()).map(|b| json!({"kind": "edits", "start": b.start, "end": b.end, "files": b.files})));
    items.extend(system.into_iter().map(|b| json!({
        "kind": "system", "start": b.start, "end": b.end, "counts": b.counts, "titles": b.titles.into_iter().take(8).collect::<Vec<_>>(),
    })));
    items.extend(memories(conn, &from, &to)?);
    items.sort_by(|a, b| b["end"].as_str().cmp(&a["end"].as_str()));

    let earlier: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE ts < ?)", [&from], |r| r.get(0))?;
    Ok(json!({"items": items, "from": from, "to": to, "hiddenSystem": hidden, "earlier": earlier}))
}
