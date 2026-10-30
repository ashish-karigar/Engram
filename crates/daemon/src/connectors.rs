use std::path::{Path, PathBuf};
use std::process::Command;

use engram_core::{Error, Result};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Muse,
}

pub const AGENTS: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Muse];

impl Agent {
    pub fn parse(name: &str) -> Option<Agent> {
        AGENTS.into_iter().find(|a| a.id() == name)
    }

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Muse => "muse",
        }
    }

    /// The actor name requests from this agent are attributed to.
    pub fn actor_name(self) -> &'static str {
        match self {
            Agent::Claude => "claude-code",
            Agent::Codex => "codex",
            Agent::Muse => "muse-code",
        }
    }
}

pub struct Endpoint {
    /// Project-scoped base, e.g. `http://127.0.0.1:4000/p/abc123`.
    pub base_url: String,
    pub token: String,
}

impl Endpoint {
    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }
}

const BEGIN: &str = "# >>> engram >>>";
const END: &str = "# <<< engram <<<";

/// Insert or replace a marked block, so re-connecting never duplicates the entry.
pub fn upsert_block(text: &str, block: &str) -> String {
    let wrapped = format!("{BEGIN}\n{block}\n{END}");
    match (text.find(BEGIN), text.find(END)) {
        (Some(start), Some(end)) if end > start => format!("{}{wrapped}{}", &text[..start], &text[end + END.len()..]),
        _ => format!("{}\n\n{wrapped}\n", text.trim_end()),
    }
}

/// Merge an `engram` entry into a JSON settings file's `mcpServers`, leaving everything else intact.
pub fn merge_mcp_server(json_text: &str, entry: Value) -> Result<String> {
    let mut data: Value = if json_text.trim().is_empty() { json!({}) } else { serde_json::from_str(json_text)? };
    let servers = data.as_object_mut().ok_or_else(|| Error::msg("settings file is not a JSON object"))?
        .entry("mcpServers").or_insert_with(|| json!({}));
    servers.as_object_mut().ok_or_else(|| Error::msg("mcpServers is not an object"))?.insert("engram".into(), entry);
    Ok(serde_json::to_string_pretty(&data)? + "\n")
}

pub struct ConnectPlan {
    pub agent: Agent,
    /// What will change, shown before anything is applied. Never contains the token.
    pub preview: String,
    apply: Box<dyn FnOnce() -> Result<String> + Send>,
}

impl ConnectPlan {
    pub fn apply(self) -> Result<String> {
        (self.apply)()
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn backup(file: &Path) -> Result<()> {
    if file.exists() {
        std::fs::copy(file, format!("{}.engram.bak", file.display()))?;
    } else if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn plan_connect(agent: Agent, e: Endpoint) -> ConnectPlan {
    match agent {
        Agent::Codex => codex_plan(e, home().join(".codex/config.toml")),
        Agent::Muse => muse_plan(e, home().join(".config/muse/settings.json")),
        Agent::Claude => claude_plan(e),
    }
}

fn codex_block(url: &str, token: &str) -> String {
    format!("[mcp_servers.engram]\nurl = \"{url}\"\nhttp_headers = {{ Authorization = \"Bearer {token}\" }}")
}

pub fn codex_plan(e: Endpoint, file: PathBuf) -> ConnectPlan {
    let preview = format!("Add to {}:\n{}", file.display(), codex_block(&e.mcp_url(), "<token>"));
    ConnectPlan {
        agent: Agent::Codex,
        preview,
        apply: Box::new(move || {
            backup(&file)?;
            let current = std::fs::read_to_string(&file).unwrap_or_default();
            std::fs::write(&file, upsert_block(&current, &codex_block(&e.mcp_url(), &e.token)))?;
            Ok(format!("Updated {} (backup: {}.engram.bak). Restart Codex.", file.display(), file.display()))
        }),
    }
}

fn mcp_entry(url: &str, token: &str) -> Value {
    json!({"type": "http", "url": url, "headers": {"Authorization": format!("Bearer {token}")}})
}

pub fn muse_plan(e: Endpoint, file: PathBuf) -> ConnectPlan {
    let preview = format!(
        "Add mcpServers.engram to {}:\n{}",
        file.display(),
        serde_json::to_string_pretty(&mcp_entry(&e.mcp_url(), "<token>")).unwrap_or_default()
    );
    ConnectPlan {
        agent: Agent::Muse,
        preview,
        apply: Box::new(move || {
            backup(&file)?;
            let current = std::fs::read_to_string(&file).unwrap_or_default();
            std::fs::write(&file, merge_mcp_server(&current, mcp_entry(&e.mcp_url(), &e.token))?)?;
            Ok(format!(
                "Updated {} (backup: {}.engram.bak). Restart Muse. The entry format follows Meta's streamable-HTTP convention; verify with /mcp inside Muse.",
                file.display(),
                file.display()
            ))
        }),
    }
}

fn claude_plan(e: Endpoint) -> ConnectPlan {
    let url = e.mcp_url();
    let preview = format!("Run: claude mcp add --transport http --scope user engram {url} --header \"Authorization: Bearer <token>\"");
    ConnectPlan {
        agent: Agent::Claude,
        preview,
        apply: Box::new(move || {
            let run = |args: &[&str]| Command::new("claude").args(args).output();
            let _ = run(&["mcp", "remove", "--scope", "user", "engram"]);
            let header = format!("Authorization: Bearer {}", e.token);
            let out = run(&["mcp", "add", "--transport", "http", "--scope", "user", "engram", &url, "--header", &header])
                .map_err(|err| Error::msg(format!("could not run `claude` ({err}); run the command above yourself")))?;
            if !out.status.success() {
                return Err(Error::msg(String::from_utf8_lossy(&out.stderr).trim().to_string()));
            }
            Ok("Registered with Claude Code (user scope). Start a new session.".into())
        }),
    }
}
