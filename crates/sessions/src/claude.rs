//! Claude Code: `~/.claude/projects/<slug>/<session>.jsonl`, one record per message.
use serde_json::Value;

use crate::turn::{Role, ToolUse, Turn, files_from_input, is_injected};

pub fn cwd_of(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line).ok()?["cwd"].as_str().map(String::from)
}

pub fn parse_line(line: &str) -> Option<Turn> {
    let v: Value = serde_json::from_str(line).ok()?;
    let role = match v["type"].as_str()? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        _ => return None,
    };
    if v["isMeta"].as_bool() == Some(true) {
        return None;
    }
    let session_id = v["sessionId"].as_str().unwrap_or_default().to_string();
    let ts = v["timestamp"].as_str().unwrap_or_default().to_string();
    if v["isCompactSummary"].as_bool() == Some(true) {
        return Some(Turn { session_id, ts, role, text: String::new(), tools: vec![], compaction: true, cwd: v["cwd"].as_str().map(String::from) });
    }
    let content = &v["message"]["content"];
    let mut texts: Vec<&str> = content.as_str().into_iter().collect();
    let mut tools = vec![];
    for block in content.as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => texts.extend(block["text"].as_str()),
            Some("tool_use") => tools.push(ToolUse {
                name: block["name"].as_str().unwrap_or("tool").to_string(),
                files: files_from_input(&block["input"]),
            }),
            _ => {}
        }
    }
    let text = texts.join("\n");
    let text = if role == Role::User && is_injected(&text) { String::new() } else { text };
    let turn = Turn { session_id, ts, role, text, tools, compaction: false, cwd: v["cwd"].as_str().map(String::from) };
    (!turn.is_empty()).then_some(turn)
}
