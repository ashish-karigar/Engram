//! Muse Code: `~/.local/share/muse/sessions/YYYY/MM/DD/<session>/session.jsonl`. An event log:
//! records may be nested in `children[].record_json` frames. User prompts are `run.started`,
//! replies `assistant_message_committed`, tool calls `assistant_tool_calls_committed`.
use serde_json::Value;

use crate::turn::{Role, ToolUse, Turn, epoch_to_iso, files_from_input};

/// Flatten a line into its records (a frame holds several).
pub fn records(line: &str) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    match v["children"].as_array() {
        Some(children) => children.iter().filter_map(|c| c["record_json"].as_str()).filter_map(|j| serde_json::from_str(j).ok()).collect(),
        None => vec![v],
    }
}

pub fn cwd_of(record: &Value) -> Option<String> {
    let r = &record["payload"]["record"];
    match record["payload"]["kind"].as_str()? {
        "route_facts" => r["cwd"].as_str().map(String::from),
        "metadata" | "workspace_branch" => r["workspace_root"].as_str().map(String::from),
        _ => None,
    }
}

pub fn parse_record(record: &Value) -> Option<Turn> {
    let session_id = record["stream"]["id"].as_str().unwrap_or_default().to_string();
    let ts = record["recorded_at"].as_i64().map(epoch_to_iso).unwrap_or_default();
    let p = &record["payload"];
    let e = &p["event"];
    let kind = e["kind"].as_str().unwrap_or_default();
    if p["kind"] != "run" {
        return None;
    }
    let turn = |role, text: &str, tools| Turn { session_id: session_id.clone(), ts: ts.clone(), role, text: text.to_string(), tools, compaction: false };
    match kind {
        "started" => e["prompt"].as_str().filter(|t| !t.trim().is_empty()).map(|t| turn(Role::User, t, vec![])),
        "assistant_message_committed" => e["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| turn(Role::Assistant, t, vec![])),
        "assistant_tool_calls_committed" => {
            let tools = e["tool_calls"].as_array()?.iter().map(|c| {
                let args: Value = c["args"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
                ToolUse { name: c["name"].as_str().unwrap_or("tool").to_string(), files: files_from_input(&args) }
            });
            Some(turn(Role::Assistant, "", tools.collect()))
        }
        k if k.contains("compact") => Some(Turn { compaction: true, ..turn(Role::Assistant, "", vec![]) }),
        _ => None,
    }
}
