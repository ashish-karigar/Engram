use std::path::Path;
use std::process::Command;

use engram_core::{Error, Result};

/// Run git in `cwd` and return trimmed stdout.
pub fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    git_env(cwd, args, &[])
}

pub fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(cwd).envs(env.iter().copied()).output()?;
    if !out.status.success() {
        return Err(Error::msg(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_repo(cwd: &Path) -> bool {
    git(cwd, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out == "true")
}

/// Engram requires git. Initialise a repository if the folder has none. Returns true if it did.
pub fn ensure_repo(cwd: &Path) -> Result<bool> {
    if is_repo(cwd) {
        return Ok(false);
    }
    git(cwd, &["init"])?;
    Ok(true)
}

pub fn head_sha(cwd: &Path) -> Option<String> {
    git(cwd, &["rev-parse", "--verify", "-q", "HEAD"]).ok()
}

pub fn git_dir(cwd: &Path) -> Result<String> {
    git(cwd, &["rev-parse", "--absolute-git-dir"])
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub date: String,
    pub author: String,
    pub subject: String,
    pub files: Vec<String>,
}

const RECORD: char = '\u{1e}';
const FIELD: char = '\u{1f}';

/// Commits newer than `since` (exclusive), oldest first.
pub fn list_commits(cwd: &Path, since: Option<&str>, limit: usize) -> Vec<CommitInfo> {
    if head_sha(cwd).is_none() {
        return vec![];
    }
    let range = since.map(|s| format!("{s}..HEAD"));
    let max = format!("--max-count={limit}");
    let format = format!("--format={RECORD}%H{FIELD}%aI{FIELD}%an{FIELD}%s{FIELD}");
    let mut args = vec!["log"];
    args.extend(range.as_deref());
    args.extend([max.as_str(), "--reverse", "--name-only", format.as_str()]);
    // A stale `since` (rebased away) makes the range invalid; fall back to a plain listing.
    let out = git(cwd, &args).or_else(|_| {
        git(cwd, &["log", max.as_str(), "--reverse", "--name-only", format.as_str()])
    });
    out.unwrap_or_default().split(RECORD).filter(|c| !c.trim().is_empty()).map(parse_commit).collect()
}

fn parse_commit(chunk: &str) -> CommitInfo {
    let mut parts = chunk.split(FIELD);
    let mut next = || parts.next().unwrap_or("").trim().to_string();
    let (sha, date, author, subject) = (next(), next(), next(), next());
    let files = parts.next().unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    CommitInfo { sha, date, author, subject, files }
}
