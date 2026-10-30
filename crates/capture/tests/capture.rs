use std::path::Path;
use std::process::Command;
use std::time::Duration;

use engram_capture::git::{ensure_repo, git, list_commits};
use engram_capture::ingest::sync_commits;
use engram_capture::scrub::scrub;
use engram_capture::snapshot::{SNAPSHOT_REF, snapshot_changes, snapshot_worktree};
use engram_capture::watcher::{ChangeKind, is_ignored, merge_change};
use engram_capture::{CaptureOptions, start_capture};
use engram_core::db::{open_memory, shared};
use engram_core::events::{ensure_actor, list_events};
use tempfile::TempDir;

fn sh(dir: &Path, args: &[&str]) {
    assert!(Command::new("git").args(args).current_dir(dir).output().unwrap().status.success(), "git {args:?}");
}

fn temp_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    sh(dir.path(), &["init", "-q"]);
    sh(dir.path(), &["config", "user.email", "t@example.com"]);
    sh(dir.path(), &["config", "user.name", "T"]);
    dir
}

fn commit(dir: &Path, msg: &str) {
    sh(dir, &["add", "-A"]);
    sh(dir, &["commit", "-q", "-m", msg]);
}

#[test]
fn scrub_removes_common_secrets_and_keeps_normal_text() {
    let text = format!("key=AKIAABCDEFGHIJKLMNOP token: ghp_{} password = hunter2hunter2 ok", "a".repeat(36));
    let out = scrub(&text);
    assert!(!out.contains("AKIA") && !out.contains("ghp_") && !out.contains("hunter2"), "{out}");
    assert!(out.ends_with("ok"));
}

#[test]
fn ignore_rules_skip_noise() {
    assert!(is_ignored("node_modules/x/index.js"));
    assert!(is_ignored("pkg/__pycache__/a.pyc"));
    assert!(!is_ignored("src/lidar/filter.py"));
}

#[test]
fn merge_change_collapses_bursts() {
    let mut m = std::collections::HashMap::new();
    merge_change(&mut m, "a.py", ChangeKind::Add);
    merge_change(&mut m, "a.py", ChangeKind::Change);
    assert_eq!(m["a.py"], ChangeKind::Add);
    merge_change(&mut m, "a.py", ChangeKind::Unlink);
    assert!(m.is_empty());
}

#[test]
fn ensure_repo_initialises_a_plain_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert!(ensure_repo(dir.path()).unwrap());
    assert!(!ensure_repo(dir.path()).unwrap());
}

#[test]
fn snapshot_saves_uncommitted_work_without_touching_index_or_branch() {
    let dir = temp_repo();
    let p = dir.path();
    std::fs::write(p.join("a.py"), "print(1)\n").unwrap();
    commit(p, "init");
    assert!(snapshot_worktree(p, SNAPSHOT_REF).unwrap().is_none());

    std::fs::write(p.join("a.py"), "print(2)\n").unwrap();
    std::fs::write(p.join("new.py"), "x = 1\n").unwrap();
    assert!(snapshot_worktree(p, SNAPSHOT_REF).unwrap().is_some());
    let mut changed = snapshot_changes(p, SNAPSHOT_REF);
    changed.sort();
    assert_eq!(changed, ["a.py", "new.py"]);
    assert_eq!(git(p, &["show", &format!("{SNAPSHOT_REF}:new.py")]).unwrap(), "x = 1");
    assert_eq!(git(p, &["status", "--porcelain", "--untracked-files=no"]).unwrap(), "M a.py");
    assert_eq!(git(p, &["diff", "--cached", "--name-only"]).unwrap(), "");
    assert!(snapshot_worktree(p, SNAPSHOT_REF).unwrap().is_none());
}

#[test]
fn sync_commits_is_incremental_and_idempotent() {
    let dir = temp_repo();
    let p = dir.path();
    let db = open_memory().unwrap();
    let actor = ensure_actor(&db, "system", "watcher").unwrap().id;
    std::fs::write(p.join("a.py"), "1\n").unwrap();
    commit(p, "first");
    assert_eq!(sync_commits(&db, p, &actor, 200).unwrap().len(), 1);
    std::fs::write(p.join("b.py"), "2\n").unwrap();
    commit(p, "second");
    assert_eq!(sync_commits(&db, p, &actor, 200).unwrap().len(), 1);
    assert_eq!(sync_commits(&db, p, &actor, 200).unwrap().len(), 0);
    let subjects: Vec<_> = list_events(&db, 0, Some("git.commit"), 10)
        .unwrap()
        .iter()
        .map(|e| e.payload["subject"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(subjects, ["first", "second"]);
    assert_eq!(list_commits(p, None, 10).len(), 2);
}

#[test]
fn capture_records_files_commits_and_a_snapshot_end_to_end() {
    let dir = temp_repo();
    let p = dir.path().canonicalize().unwrap();
    std::fs::write(p.join("a.py"), "1\n").unwrap();
    commit(&p, "init");
    let db = shared(open_memory().unwrap());
    let mut opts = CaptureOptions::new(db.clone(), &p);
    opts.debounce = Duration::from_millis(50);
    opts.snapshot_interval = Duration::from_millis(50);
    let capture = start_capture(opts).unwrap();

    std::fs::create_dir(p.join("pkg")).unwrap();
    std::fs::write(p.join("pkg/lidar.py"), "def f(): pass\n").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    capture.stop();

    let kinds: Vec<String> = list_events(&db.lock().unwrap(), 0, None, 100).unwrap().into_iter().map(|e| e.kind).collect();
    for expected in ["git.commit", "file.change", "snapshot.created"] {
        assert!(kinds.iter().any(|k| k == expected), "missing {expected}: {kinds:?}");
    }
}
