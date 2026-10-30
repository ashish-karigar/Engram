use std::path::Path;

use engram_core::Result;
use uuid::Uuid;

use crate::git::{git, git_env, head_sha};

pub const SNAPSHOT_REF: &str = "refs/engram/snapshots";

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Engram"),
    ("GIT_AUTHOR_EMAIL", "engram@localhost"),
    ("GIT_COMMITTER_NAME", "Engram"),
    ("GIT_COMMITTER_EMAIL", "engram@localhost"),
];

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub reference: String,
    pub sha: String,
    pub base: Option<String>,
}

fn resolve(cwd: &Path, rev: &str) -> Option<String> {
    git(cwd, &["rev-parse", "--verify", "-q", rev]).ok()
}

/// Save the working tree (tracked + untracked, honouring .gitignore) as a commit on a hidden ref.
/// A temporary index is used, so the user's index, branches and working tree are never touched.
/// Returns None when nothing changed since HEAD or since the previous snapshot.
pub fn snapshot_worktree(cwd: &Path, reference: &str) -> Result<Option<Snapshot>> {
    let base = head_sha(cwd);
    let dir = std::env::temp_dir().join(format!("engram-idx-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir)?;
    let index = dir.join("index");
    let index = index.to_string_lossy().to_string();
    let result = build_snapshot(cwd, reference, base, &index);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn build_snapshot(cwd: &Path, reference: &str, base: Option<String>, index: &str) -> Result<Option<Snapshot>> {
    let mut env = IDENTITY.to_vec();
    env.push(("GIT_INDEX_FILE", index));
    if base.is_some() {
        git_env(cwd, &["read-tree", "HEAD"], &env)?;
    }
    git_env(cwd, &["add", "-A", "--", "."], &env)?;
    let tree = git_env(cwd, &["write-tree"], &env)?;

    if base.is_some() && resolve(cwd, "HEAD^{tree}").as_deref() == Some(tree.as_str()) {
        return Ok(None);
    }
    let previous = resolve(cwd, reference);
    if previous.is_some() && resolve(cwd, &format!("{reference}^{{tree}}")).as_deref() == Some(tree.as_str()) {
        return Ok(None);
    }
    let message = format!("engram snapshot\n\nbase: {}", base.as_deref().unwrap_or("none"));
    let mut args = vec!["commit-tree", tree.as_str()];
    if let Some(parent) = previous.as_deref().or(base.as_deref()) {
        args.extend(["-p", parent]);
    }
    args.extend(["-m", message.as_str()]);
    let sha = git_env(cwd, &args, &env)?;
    git(cwd, &["update-ref", reference, &sha])?;
    Ok(Some(Snapshot { reference: reference.into(), sha, base }))
}

/// Files that differ between a snapshot and HEAD: what a snapshot can recover.
pub fn snapshot_changes(cwd: &Path, reference: &str) -> Vec<String> {
    git(cwd, &["diff", "--name-only", "HEAD", reference])
        .map(|out| out.lines().map(String::from).collect())
        .unwrap_or_default()
}
