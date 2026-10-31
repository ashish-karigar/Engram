use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{claude, codex, muse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Muse,
}

pub const AGENTS: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Muse];

impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Muse => "muse",
        }
    }

    /// Same actor names the connectors use, so notes and turns land on one actor.
    pub fn actor_name(self) -> &'static str {
        match self {
            Agent::Claude => "claude-code",
            Agent::Codex => "codex",
            Agent::Muse => "muse-code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Muse => "Muse Code",
        }
    }
}

/// Where each agent keeps its logs. Defaults to the standard locations; tests point elsewhere.
#[derive(Debug, Clone)]
pub struct Sources {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub muse: PathBuf,
}

impl Default for Sources {
    fn default() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        Sources { claude: home.join(".claude/projects"), codex: home.join(".codex/sessions"), muse: home.join(".local/share/muse/sessions") }
    }
}

/// Claude Code names a project's log folder after its path, with every non-alphanumeric char as `-`.
pub fn claude_slug(root: &Path) -> String {
    root.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth > 0 {
            walk(&path, depth - 1, out, keep);
        } else if keep(&path) {
            out.push(path);
        }
    }
}

pub fn within(cwd: &str, root: &Path) -> bool {
    let root = root.to_string_lossy();
    cwd == root || cwd.starts_with(&format!("{root}/"))
}

/// The first match of `find` in the opening `lines` lines of a log.
fn first_match(path: &Path, lines: usize, find: impl Fn(&str) -> Option<String>) -> Option<String> {
    let reader = BufReader::new(File::open(path).ok()?);
    reader.lines().take(lines).map_while(Result::ok).find_map(|l| find(&l))
}

/// What a Codex log's opening lines say: its folder and whether it is a reviewer sub-session.
/// Fixed once written, and those lines are large (they carry Codex's instructions), so each file
/// is read once per process.
static CODEX_META: Mutex<Option<HashMap<PathBuf, (Option<String>, bool)>>> = Mutex::new(None);

fn codex_meta(path: &Path) -> (Option<String>, bool) {
    if let Some(known) = CODEX_META.lock().unwrap().get_or_insert_with(HashMap::new).get(path) {
        return known.clone();
    }
    let cwd = first_match(path, 5, codex::cwd_of);
    let review = first_match(path, 3, |l| codex::is_review(l).then(String::new)).is_some();
    // A log still being created has no session_meta yet; look again next time.
    if cwd.is_some() {
        CODEX_META.lock().unwrap().get_or_insert_with(HashMap::new).insert(path.to_path_buf(), (cwd.clone(), review));
    }
    (cwd, review)
}

fn is_codex_review(path: &Path) -> bool {
    codex_meta(path).1
}

impl Sources {
    /// Codex logs whose session ran inside `root`, reviewer sub-sessions included.
    fn codex_files(&self, root: &Path) -> Vec<PathBuf> {
        let mut files = vec![];
        walk(&self.codex, 4, &mut files, &|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rollout-")) && p.extension().is_some_and(|e| e == "jsonl"));
        files.retain(|f| codex_meta(f).0.is_some_and(|cwd| within(&cwd, root)));
        files
    }

    /// Codex reviewer sub-sessions inside `root` (see `codex::is_review`).
    pub fn codex_reviews_for(&self, root: &Path) -> Vec<PathBuf> {
        self.codex_files(root).into_iter().filter(|f| is_codex_review(f)).collect()
    }

    /// Log files belonging to sessions that ran inside `root` (or a folder below it).
    pub fn files_for(&self, agent: Agent, root: &Path) -> Vec<PathBuf> {
        let mut files = vec![];
        match agent {
            Agent::Claude => {
                let slug = claude_slug(root);
                walk(&self.claude, 1, &mut files, &|p| {
                    let dir = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    // The project's own folder, folders below it, or a parent folder where a session
                    // started before moving in (its turns are filtered by their cwd during sync).
                    p.extension().is_some_and(|e| e == "jsonl")
                        && (dir == slug || dir.starts_with(&format!("{slug}-")) || slug.starts_with(&format!("{dir}-")))
                });
                // A session started in a parent folder belongs here only if it worked inside the
                // project at some point; then the whole conversation counts, since the planning
                // usually happens before the agent moves into the folder.
                files.retain(|f| {
                    let dir = f.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    dir == slug || dir.starts_with(&format!("{slug}-")) || mentions_cwd_within(f, root)
                });
            }
            Agent::Codex => {
                files = self.codex_files(root);
                files.retain(|f| !is_codex_review(f));
            }
            Agent::Muse => {
                walk(&self.muse, 5, &mut files, &|p| p.file_name().is_some_and(|n| n == "session.jsonl"));
                files.retain(|f| {
                    first_match(f, 60, |l| muse::records(l).iter().find_map(muse::cwd_of)).is_some_and(|cwd| within(&cwd, root))
                });
            }
        }
        files.sort();
        files
    }
}

/// Per (log, project): how far the log has been checked, and whether it worked inside the project.
/// Session logs only grow, so a yes is final and a no only needs the bytes added since.
static SCANNED: Mutex<Option<HashMap<(PathBuf, PathBuf), (u64, bool)>>> = Mutex::new(None);

fn mentions_cwd_within(path: &Path, root: &Path) -> bool {
    let key = (path.to_path_buf(), root.to_path_buf());
    let (from, found) = SCANNED.lock().unwrap().get_or_insert_with(HashMap::new).get(&key).copied().unwrap_or((0, false));
    if found {
        return true;
    }
    let Ok(mut file) = File::open(path) else { return false };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < from { 0 } else { from };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let needle = root.to_string_lossy().to_string();
    let (mut at, mut found) = (start, false);
    for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
        at += line.len() as u64 + 1;
        // Most lines never mention the folder; only those are parsed.
        let Ok(line) = std::str::from_utf8(&line) else { continue };
        if line.contains(&needle) && claude::cwd_of(line).is_some_and(|c| within(&c, root)) {
            found = true;
            break;
        }
    }
    SCANNED.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, (at.min(len), found));
    found
}
