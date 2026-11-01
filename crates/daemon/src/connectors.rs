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
    /// The daemon's base URL, e.g. `http://127.0.0.1:41741`; agents use its shared `/mcp`.
    pub base_url: String,
    pub token: String,
}

impl Endpoint {
    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }
}

/// The name agents know the server by. It was `engram` before the app became MemoryOS.
pub const SERVER: &str = "memoryos";
const OLD_SERVER: &str = "engram";

const BEGIN: &str = "# >>> memoryos >>>";
const END: &str = "# <<< memoryos <<<";
const OLD_BEGIN: &str = "# >>> engram >>>";
const OLD_END: &str = "# <<< engram <<<";

fn cut_block(text: &str, begin: &str, end: &str) -> Option<(usize, usize)> {
    match (text.find(begin), text.find(end)) {
        (Some(start), Some(stop)) if stop > start => Some((start, stop + end.len())),
        _ => None,
    }
}

/// Insert or replace a marked block, so re-connecting never duplicates the entry. A block under
/// the old name is replaced too.
pub fn upsert_block(text: &str, block: &str) -> String {
    let wrapped = format!("{BEGIN}\n{block}\n{END}");
    let mut text = text.to_string();
    if let Some((start, stop)) = cut_block(&text, OLD_BEGIN, OLD_END) {
        text.replace_range(start..stop, "");
    }
    match cut_block(&text, BEGIN, END) {
        Some((start, stop)) => format!("{}{wrapped}{}", &text[..start], &text[stop..]),
        None => format!("{}\n\n{wrapped}\n", text.trim_end()),
    }
}

/// Merge the server's entry into a JSON settings file's `mcpServers`, leaving everything else
/// intact apart from an entry under the old name.
pub fn merge_mcp_server(json_text: &str, entry: Value) -> Result<String> {
    let mut data: Value = if json_text.trim().is_empty() { json!({}) } else { serde_json::from_str(json_text)? };
    let servers = data.as_object_mut().ok_or_else(|| Error::msg("settings file is not a JSON object"))?
        .entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| Error::msg("mcpServers is not an object"))?;
    servers.remove(OLD_SERVER);
    servers.insert(SERVER.into(), entry);
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
        std::fs::copy(file, format!("{}.memoryos.bak", file.display()))?;
    } else if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn plan_connect(agent: Agent, e: Endpoint) -> ConnectPlan {
    match agent {
        Agent::Codex => codex_plan(e, home().join(".codex/config.toml"), home().join(".codex/hooks.json")),
        Agent::Muse => muse_plan(e, home().join(".config/muse/settings.json")),
        Agent::Claude => claude_plan(e),
    }
}

fn codex_block(url: &str, token: &str) -> String {
    format!("[mcp_servers.{SERVER}]\nurl = \"{url}\"\nhttp_headers = {{ Authorization = \"Bearer {token}\" }}")
}

/// Like Muse, Codex listed the tools and answered from `rg` without calling them, so it gets the
/// same SessionStart hook. Codex's hooks.json uses Claude Code's format; Codex runs a new hook only
/// after the user trusts it once (`/hooks`).
pub fn codex_plan(e: Endpoint, file: PathBuf, hooks: PathBuf) -> ConnectPlan {
    let command = hook_command("codex");
    let preview = format!(
        "1. Add to {}:\n{}\n2. Add a SessionStart hook to {} (startup, resume and after compaction):\n   {command}\n   It injects the project brief at the start of every session.",
        file.display(),
        codex_block(&e.mcp_url(), "<token>"),
        hooks.display()
    );
    ConnectPlan {
        agent: Agent::Codex,
        preview,
        apply: Box::new(move || {
            backup(&file)?;
            let current = std::fs::read_to_string(&file).unwrap_or_default();
            std::fs::write(&file, upsert_block(&current, &codex_block(&e.mcp_url(), &e.token)))?;
            backup(&hooks)?;
            let current = std::fs::read_to_string(&hooks).unwrap_or_default();
            std::fs::write(&hooks, merge_session_hook(&current, Agent::Codex, &command)?)?;
            Ok(format!(
                "Updated {} and {} (backups: .memoryos.bak). Restart Codex, then trust the MemoryOS hook once with /hooks.",
                file.display(),
                hooks.display()
            ))
        }),
    }
}

fn mcp_entry(url: &str, token: &str) -> Value {
    json!({"type": "http", "url": url, "headers": {"Authorization": format!("Bearer {token}")}})
}

/// The MCP server alone is not enough: in the suite E trials Muse listed the tools and never
/// called them. The SessionStart hook puts the brief in front of the model, as for Claude Code.
pub fn muse_plan(e: Endpoint, file: PathBuf) -> ConnectPlan {
    let command = hook_command("muse");
    let preview = format!(
        "In {}:\n1. Add mcpServers.{SERVER}:\n{}\n2. Add a SessionStart hook (startup, resume and after compaction):\n   {command}\n   It injects the project brief at the start of every session.",
        file.display(),
        serde_json::to_string_pretty(&mcp_entry(&e.mcp_url(), "<token>")).unwrap_or_default()
    );
    ConnectPlan {
        agent: Agent::Muse,
        preview,
        apply: Box::new(move || {
            backup(&file)?;
            let current = std::fs::read_to_string(&file).unwrap_or_default();
            let with_server = merge_mcp_server(&current, mcp_entry(&e.mcp_url(), &e.token))?;
            std::fs::write(&file, merge_session_hook(&with_server, Agent::Muse, &command)?)?;
            Ok(format!(
                "Updated {} (backup: {}.memoryos.bak). Restart Muse. The entry format follows Meta's streamable-HTTP convention; verify with /mcp inside Muse.",
                file.display(),
                file.display()
            ))
        }),
    }
}

/// The command an agent hook runs: this same binary with `hook <agent>`.
pub fn hook_command(agent: &str) -> String {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "memoryos".into());
    format!("\"{exe}\" hook {agent}")
}

/// Add the SessionStart hook to an agent's JSON settings (Claude Code and Muse share the format;
/// it runs on startup, resume and after compaction). Existing hooks are kept; an older one of
/// ours is replaced, never duplicated.
pub fn merge_session_hook(json_text: &str, agent: Agent, command: &str) -> Result<String> {
    let mut data: Value = if json_text.trim().is_empty() { json!({}) } else { serde_json::from_str(json_text)? };
    let root = data.as_object_mut().ok_or_else(|| Error::msg("settings file is not a JSON object"))?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| Error::msg("hooks is not an object"))?;
    let starts = hooks.entry("SessionStart").or_insert_with(|| json!([])).as_array_mut().ok_or_else(|| Error::msg("SessionStart is not a list"))?;
    let ours = format!(" hook {}", agent.id());
    starts.retain(|group| !group["hooks"].as_array().into_iter().flatten().any(|h| h["command"].as_str().is_some_and(|c| c.ends_with(&ours))));
    starts.push(json!({"matcher": "startup|resume|compact", "hooks": [{"type": "command", "command": command, "timeout": 10}]}));
    Ok(serde_json::to_string_pretty(&data)? + "\n")
}

fn claude_plan(e: Endpoint) -> ConnectPlan {
    let url = e.mcp_url();
    let settings = home().join(".claude/settings.json");
    let command = hook_command("claude");
    let user_config = home().join(".claude.json");
    let preview = format!(
        "1. Run: claude mcp add --transport http --scope user {SERVER} {url} --header \"Authorization: Bearer <token>\"\n   \
         (without the `claude` command, the same entry goes into mcpServers in {})\n\
         2. Add a SessionStart hook to {} (startup, resume and after compaction):\n   {command}\n   It injects the project brief so Claude re-orients after compacting.",
        user_config.display(),
        settings.display()
    );
    ConnectPlan {
        agent: Agent::Claude,
        preview,
        apply: Box::new(move || {
            backup(&settings)?;
            let current = std::fs::read_to_string(&settings).unwrap_or_default();
            std::fs::write(&settings, merge_session_hook(&current, Agent::Claude, &command)?)?;
            let hook_note = format!("Hook added to {} (backup: {}.memoryos.bak).", settings.display(), settings.display());

            let run = |args: &[&str]| Command::new("claude").args(args).output();
            for name in [OLD_SERVER, SERVER] {
                let _ = run(&["mcp", "remove", "--scope", "user", name]);
            }
            let header = format!("Authorization: Bearer {}", e.token);
            match run(&["mcp", "add", "--transport", "http", "--scope", "user", SERVER, &url, "--header", &header]) {
                Ok(out) if out.status.success() => Ok(format!("Registered with Claude Code (user scope). {hook_note} Start a new session.")),
                Ok(out) => Err(Error::msg(format!("{hook_note} But `claude mcp add` failed: {}", String::from_utf8_lossy(&out.stderr).trim()))),
                // The desktop app ships Claude Code without the CLI on PATH: write what `mcp add` writes.
                Err(_) => {
                    backup(&user_config)?;
                    let current = std::fs::read_to_string(&user_config).unwrap_or_default();
                    std::fs::write(&user_config, merge_mcp_server(&current, mcp_entry(&url, &e.token))?)?;
                    Ok(format!("Added {SERVER} to mcpServers in {} (backup: .memoryos.bak). {hook_note} Start a new session.", user_config.display()))
                }
            }
        }),
    }
}
