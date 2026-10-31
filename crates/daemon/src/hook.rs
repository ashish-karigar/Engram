//! `engram hook <agent>`: run by an agent's lifecycle hook (Claude Code's SessionStart, including
//! after compaction). Prints the project's brief as context the agent sees before its next turn.
//! It always fails open: any problem prints nothing and exits 0, so the agent is never blocked.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::paths::app_data_dir;
use crate::secrets::ensure_agent_token;

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder().timeout(Duration::from_secs(4)).build().ok()
}

/// The watched project that contains `cwd` (the deepest one, if projects are nested).
fn project_for(url: &str, admin: &str, cwd: &Path) -> Option<String> {
    let projects: Value = client()?.get(format!("{url}/api/projects")).bearer_auth(admin).send().ok()?.json().ok()?;
    projects
        .as_array()?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?.to_string(), PathBuf::from(p["root"].as_str()?))))
        .filter(|(_, root)| cwd.starts_with(root))
        .max_by_key(|(_, root)| root.as_os_str().len())
        .map(|(id, _)| id)
}

fn brief(agent: &str, input: &Value) -> Option<String> {
    let base = app_data_dir();
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(base.join("runtime.json")).ok()?).ok()?;
    let (url, admin) = (runtime["url"].as_str()?, runtime["adminToken"].as_str()?);
    let cwd = input["cwd"].as_str().map(PathBuf::from).or_else(|| std::env::current_dir().ok())?;
    let id = project_for(url, admin, &cwd)?;
    // The agent's own token, so the request log attributes the brief to it.
    let token = ensure_agent_token(&base, agent).ok()?;
    let res: Value = client()?.post(format!("{url}/p/{id}/api/brief")).bearer_auth(&token).json(&json!({})).send().ok()?.json().ok()?;
    let text = res["text"].as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Returns the JSON Claude Code expects from a SessionStart hook, or None to stay silent.
pub fn hook_output(agent: &str, input: &Value) -> Option<String> {
    let actor = match agent {
        "claude" => "claude-code",
        "codex" => "codex",
        "muse" => "muse-code",
        other => other,
    };
    let text = brief(actor, input)?;
    let event = input["hook_event_name"].as_str().unwrap_or("SessionStart");
    let why = if input["source"] == "compact" { "Context was just compacted; this is what the project remembers." } else { "Shared project memory from Engram." };
    let context = format!("{why} Use the engram MCP tools (search, expand, note) for more.\n\n{text}");
    Some(json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}}).to_string())
}

/// Entry point for the CLI and the desktop binary. Never fails.
pub fn run_hook(agent: &str) -> i32 {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if let Some(out) = hook_output(agent, &input) {
        println!("{out}");
    }
    0
}
