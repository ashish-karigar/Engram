use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_rag::{DocumentInput, ingest};
use serde_json::{Value, json};

/// Keep transcripts bounded; older turns stay in the event log. Counted in raw turns (tool-only
/// turns included), so this is generous: they fold away when rendering.
const MAX_TURNS: i64 = 5000;

fn label(agent: &str) -> &str {
    match agent {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "muse-code" => "Muse Code",
        other => other,
    }
}

/// Render a session as Markdown (one `##` section per turn) and index it as a document, so the
/// reasoning in agent conversations is searchable next to memories and project docs.
pub fn rebuild_transcript(conn: &Connection, embedder: &dyn Embedder, external: &str) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT e.ts, e.kind, e.payload FROM events e JOIN sessions s ON s.id = e.session_id
         WHERE s.external_ref = ?1 AND e.kind IN ('agent.turn', 'agent.compacted')
         ORDER BY e.ts DESC, e.seq DESC LIMIT ?2",
    )?;
    let mut rows: Vec<(String, String, Value)> = stmt
        .query_map(params![external, MAX_TURNS], |r| {
            let payload: String = r.get(2)?;
            Ok((r.get(0)?, r.get(1)?, serde_json::from_str(&payload).unwrap_or(Value::Null)))
        })?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    rows.reverse();
    let Some((first_ts, _, first)) = rows.first() else { return Ok(()) };
    let agent = label(first["agent"].as_str().unwrap_or("agent")).to_string();
    let day = first_ts.get(..10).unwrap_or(first_ts);
    let opening = rows.iter().find(|(_, k, p)| k == "agent.turn" && p["role"] == "user" && !p["text"].as_str().unwrap_or("").is_empty());
    let topic: String = opening.and_then(|(_, _, p)| p["text"].as_str()).unwrap_or("session").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();

    // One section per spoken turn. Tool calls are folded into the turn they belong to, so a run of
    // tool use never becomes a chunk of its own.
    let mut sections: Vec<(String, String, Vec<String>)> = vec![];
    for (ts, kind, p) in &rows {
        let time = ts.get(11..16).unwrap_or("").to_string();
        if kind == "agent.compacted" {
            sections.push((format!("Context compacted · {time}"), "The agent compacted its context here.".into(), vec![]));
            continue;
        }
        let who = if p["role"] == "user" { "You".to_string() } else { agent.clone() };
        let text = p["text"].as_str().unwrap_or("").trim().to_string();
        let tools: Vec<String> = p["tools"].as_array().into_iter().flatten().map(|t| {
            let files: Vec<&str> = t["files"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if files.is_empty() { t["name"].as_str().unwrap_or("tool").to_string() } else { format!("{} {}", t["name"].as_str().unwrap_or("tool"), files.join(", ")) }
        }).collect();
        match sections.last_mut() {
            Some((heading, _, used)) if text.is_empty() && heading.starts_with(&who) => used.extend(tools),
            _ if text.is_empty() => {}
            _ => sections.push((format!("{who} · {time}"), text, tools)),
        }
    }
    let mut md = String::new();
    for (heading, text, tools) in &sections {
        md.push_str(&format!("## {heading}\n\n{text}\n\n"));
        if !tools.is_empty() {
            let mut unique: Vec<&String> = vec![];
            for t in tools { if !unique.contains(&t) { unique.push(t); } }
            md.push_str(&format!("_Tools: {}_\n\n", unique.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; ")));
        }
    }
    let start = first_ts.get(11..16).unwrap_or("");

    let input = DocumentInput {
        external_id: Some(format!("session:{external}")),
        title: format!("{agent} session · {day} {start}"),
        source: "session".into(),
        path: None,
        file_name: "transcript.md".into(),
        bytes: md.into_bytes(),
        metadata: json!({"agent": first["agent"], "session": external, "topic": topic, "started": first_ts}),
    };
    ingest(conn, embedder, input)?;
    Ok(())
}
