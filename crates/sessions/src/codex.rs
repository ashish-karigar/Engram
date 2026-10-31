//! Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. A `session_meta` record carries the
//! session id and working directory; `response_item` records carry messages and tool calls.
use serde_json::Value;

use crate::turn::{Role, ToolUse, Turn, files_from_input, is_injected};

pub fn cwd_of(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    (v["type"] == "session_meta" || v["type"] == "turn_context").then(|| v["payload"]["cwd"].as_str().map(String::from)).flatten()
}

/// Codex's automatic approval reviewer ("guardian") runs as a sub-session that replays the user's
/// conversation for a safety check. It is not the user's work, so its logs are skipped.
pub fn is_review(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return false };
    let p = &v["payload"];
    v["type"] == "session_meta" && (p["thread_source"] == "guardian_review" || p["source"]["subagent"]["other"] == "guardian")
}

pub fn session_id_of(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    (v["type"] == "session_meta").then(|| v["payload"]["id"].as_str().or(v["payload"]["session_id"].as_str()).map(String::from)).flatten()
}

pub fn parse_line(line: &str, session_id: &str) -> Option<Turn> {
    let v: Value = serde_json::from_str(line).ok()?;
    let ts = v["timestamp"].as_str().unwrap_or_default().to_string();
    let p = &v["payload"];
    let turn = |role, text: String, tools, compaction| Turn { session_id: session_id.to_string(), ts: ts.clone(), role, text, tools, compaction, cwd: None };
    match (v["type"].as_str()?, p["type"].as_str()) {
        ("compacted", _) => Some(turn(Role::Assistant, String::new(), vec![], true)),
        ("response_item", Some("message")) => {
            let role = match p["role"].as_str()? {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            let text = p["content"].as_array()?.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
            (!text.trim().is_empty() && !(role == Role::User && is_injected(&text))).then(|| turn(role, text, vec![], false))
        }
        ("response_item", Some("function_call")) => {
            let args: Value = p["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
            let tool = ToolUse { name: p["name"].as_str().unwrap_or("tool").to_string(), files: files_from_input(&args) };
            Some(turn(Role::Assistant, String::new(), vec![tool], false))
        }
        ("response_item", Some("custom_tool_call")) => {
            let tool = ToolUse { name: p["name"].as_str().unwrap_or("tool").to_string(), files: files_from_input(&p["input"]) };
            Some(turn(Role::Assistant, String::new(), vec![tool], false))
        }
        _ => None,
    }
}
