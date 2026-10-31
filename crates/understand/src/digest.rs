use std::collections::HashMap;

use engram_capture::scrub::scrub;
use engram_core::events::StoredEvent;

pub struct Digest {
    pub text: String,
    /// What the model was shown for each event; grounding checks quotes against this.
    pub by_seq: HashMap<i64, String>,
}

/// Supplies the diff for a commit sha at extraction time. Diffs are read from git, never stored.
pub type DiffProvider<'a> = &'a dyn Fn(&str) -> String;

const MAX_DIFF_CHARS: usize = 4000;
const MAX_DIGEST_CHARS: usize = 24_000;

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn render(e: &StoredEvent, diff: &str) -> String {
    let p = &e.payload;
    match e.kind.as_str() {
        "git.commit" => {
            let sha: String = p["sha"].as_str().unwrap_or("").chars().take(8).collect();
            let tail = if diff.is_empty() { String::new() } else { format!("\n{diff}") };
            format!("commit {sha}: {}\nfiles: {}{tail}", p["subject"].as_str().unwrap_or(""), strings(&p["files"]).join(", "))
        }
        "file.change" => {
            let files: Vec<String> = p["files"]
                .as_array()
                .map(|a| a.iter().map(|f| format!("{} ({})", f["path"].as_str().unwrap_or(""), f["change"].as_str().unwrap_or(""))).collect())
                .unwrap_or_default();
            format!("files changed: {}", files.join(", "))
        }
        "agent.note" => format!("agent note: {p}"),
        "agent.turn" => {
            let who = if p["role"] == "user" { "user" } else { p["agent"].as_str().unwrap_or("agent") };
            let tools: Vec<String> = p["tools"].as_array().into_iter().flatten().map(|t| {
                let files: Vec<&str> = t["files"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).collect();
                format!("{} {}", t["name"].as_str().unwrap_or("tool"), files.join(", ")).trim().to_string()
            }).collect();
            let tail = if tools.is_empty() { String::new() } else { format!(" [tools: {}]", tools.join("; ")) };
            format!("{who}: {}{tail}", p["text"].as_str().unwrap_or(""))
        }
        other => format!("{other}: {}", p.to_string().chars().take(500).collect::<String>()),
    }
}

pub fn build_digest(events: &[StoredEvent], get_diff: Option<DiffProvider>) -> Digest {
    let mut by_seq = HashMap::new();
    let mut lines = vec![];
    let mut used = 0;
    for e in events.iter().filter(|e| e.kind != "snapshot.created") {
        let diff = match (e.kind.as_str(), e.payload["sha"].as_str(), get_diff) {
            ("git.commit", Some(sha), Some(provider)) => provider(sha).chars().take(MAX_DIFF_CHARS).collect(),
            _ => String::new(),
        };
        let body = scrub(&render(e, &diff));
        let line = format!("[#{} {}] {body}", e.seq, e.ts);
        by_seq.insert(e.seq, body);
        if used + line.len() > MAX_DIGEST_CHARS {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    Digest { text: lines.join("\n\n"), by_seq }
}
