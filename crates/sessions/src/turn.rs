use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolUse {
    pub name: String,
    /// Project files the tool read or changed, when the log says so.
    pub files: Vec<String>,
}

/// One step of an agent conversation, normalized across agents.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Turn {
    pub session_id: String,
    /// ISO 8601.
    pub ts: String,
    pub role: Role,
    pub text: String,
    pub tools: Vec<ToolUse>,
    /// The agent compacted its context here: the moment Engram's brief matters most.
    pub compaction: bool,
    /// Working directory recorded with this turn, when the log has one per turn.
    #[serde(skip)]
    pub cwd: Option<String>,
}

impl Turn {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.tools.is_empty() && !self.compaction
    }
}

/// Text that agents inject into the user role but that the user did not write.
pub fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    ["<environment_context", "<user_instructions", "<system-reminder", "<command-", "<local-command", "# AGENTS.md", "<permissions"]
        .iter()
        .any(|p| t.starts_with(p))
}

/// Pull file paths out of common tool inputs (`file_path`, `path`, `paths`, apply_patch headers).
fn collect_strings<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match v {
        serde_json::Value::String(s) => out.push(s),
        serde_json::Value::Array(items) => items.iter().for_each(|i| collect_strings(i, out)),
        serde_json::Value::Object(map) => map.values().for_each(|i| collect_strings(i, out)),
        _ => {}
    }
}

pub fn files_from_input(input: &serde_json::Value) -> Vec<String> {
    let mut out = vec![];
    for key in ["file_path", "path", "notebook_path", "filePath"] {
        if let Some(p) = input[key].as_str() {
            out.push(p.to_string());
        }
    }
    for p in input["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str()) {
        out.push(p.to_string());
    }
    // Patches can sit anywhere in a tool's input: `input`, `patch`, or a shell `cmd` running
    // `apply_patch` (Codex's exec tool). Scan every string.
    let mut texts = vec![];
    collect_strings(input, &mut texts);
    for text in texts {
        for line in text.lines() {
            for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                if let Some(p) = line.strip_prefix(marker) {
                    out.push(p.trim().to_string());
                }
            }
        }
    }
    out
}

/// Microseconds, milliseconds or seconds since the epoch, to ISO 8601.
pub fn epoch_to_iso(n: i64) -> String {
    let millis = match n {
        n if n > 100_000_000_000_000 => n / 1000,
        n if n > 100_000_000_000 => n,
        n => n * 1000,
    };
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
