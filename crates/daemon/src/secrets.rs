use std::collections::BTreeMap;
use std::path::Path;

use engram_core::{Connection, Result};
use engram_serve::tokens::{issue_token, verify_token};
use uuid::Uuid;

type TokenFile = BTreeMap<String, String>;

fn read(file: &Path) -> TokenFile {
    std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Write a file only the current user can read.
pub fn write_private(file: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Raw tokens must reach agent configs, so they are kept in a 0600 file next to the database.
/// The database itself only stores hashes.
pub fn ensure_token(conn: &Connection, file: &Path, kind: &str, name: &str) -> Result<String> {
    let mut tokens = read(file);
    if let Some(existing) = tokens.get(name) {
        if verify_token(conn, existing)?.is_some() {
            return Ok(existing.clone());
        }
    }
    let (token, _) = issue_token(conn, kind, name)?;
    tokens.insert(name.to_string(), token.clone());
    write_private(file, &serde_json::to_string_pretty(&tokens)?)?;
    Ok(token)
}

/// Agents connect once to the shared endpoint, with one token each, kept in `agents.json` (0600)
/// so connectors can write it into the agent's config.
pub fn ensure_agent_token(base: &Path, agent: &str) -> std::io::Result<String> {
    let file = base.join("agents.json");
    let mut tokens = read(&file);
    if let Some(existing) = tokens.get(agent) {
        return Ok(existing.clone());
    }
    let token = format!("egm_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    tokens.insert(agent.to_string(), token.clone());
    write_private(&file, &serde_json::to_string_pretty(&tokens).map_err(std::io::Error::other)?)?;
    Ok(token)
}

/// Replace an agent's token. The old one stops working at once: tokens are checked against this file.
pub fn reset_agent_token(base: &Path, agent: &str) -> std::io::Result<String> {
    let file = base.join("agents.json");
    let mut tokens = read(&file);
    tokens.remove(agent);
    write_private(&file, &serde_json::to_string_pretty(&tokens).map_err(std::io::Error::other)?)?;
    ensure_agent_token(base, agent)
}

/// The agent a shared-endpoint token belongs to.
pub fn agent_for_token(base: &Path, token: &str) -> Option<String> {
    read(&base.join("agents.json")).into_iter().find(|(_, t)| t == token).map(|(agent, _)| agent)
}

/// One admin token for the whole daemon; the desktop app uses it to manage projects.
pub fn ensure_admin_token(base: &Path) -> std::io::Result<String> {
    let file = base.join("admin_token");
    if let Ok(existing) = std::fs::read_to_string(&file) {
        if !existing.trim().is_empty() {
            return Ok(existing.trim().to_string());
        }
    }
    let token = format!("adm_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    write_private(&file, &token)?;
    Ok(token)
}
