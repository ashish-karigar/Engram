use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use engram_core::{Error, Result};
use notify::{EventKind, RecursiveMode, Watcher};

use crate::git::git_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Add,
    Change,
    Unlink,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Add => "add",
            ChangeKind::Change => "change",
            ChangeKind::Unlink => "unlink",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub change: ChangeKind,
}

const IGNORED_DIRS: &[&str] =
    &[".git", "node_modules", "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache", ".idea", "dist", "build", "target"];
const IGNORED_SUFFIXES: &[&str] = &[".pyc", ".pyo", ".DS_Store", ".swp", "~"];

pub fn is_ignored(rel: &str) -> bool {
    rel.split('/').any(|part| IGNORED_DIRS.contains(&part)) || IGNORED_SUFFIXES.iter().any(|s| rel.ends_with(s))
}

/// Collapse a burst of events into one change per path: latest kind wins, add then unlink cancels.
pub fn merge_change(pending: &mut HashMap<String, ChangeKind>, path: &str, change: ChangeKind) {
    match (pending.get(path).copied(), change) {
        (Some(ChangeKind::Add), ChangeKind::Unlink) => {
            pending.remove(path);
        }
        (Some(ChangeKind::Add), ChangeKind::Change) => {}
        _ => {
            pending.insert(path.to_string(), change);
        }
    }
}

pub enum Signal {
    Files(Vec<FileChange>),
    /// HEAD moved: commit, checkout, rebase.
    GitActivity,
}

pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl WatchHandle {
    pub fn close(mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn classify(kind: &EventKind) -> Option<ChangeKind> {
    match kind {
        EventKind::Create(_) => Some(ChangeKind::Add),
        EventKind::Modify(_) => Some(ChangeKind::Change),
        EventKind::Remove(_) => Some(ChangeKind::Unlink),
        _ => None,
    }
}

/// Watch a project folder. Bursts are debounced and sent to `emit` as one batch.
pub fn watch_project(root: &Path, debounce: Duration, emit: impl Fn(Signal) + Send + 'static) -> Result<WatchHandle> {
    let (tx, rx) = channel::<notify::Event>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })
    .map_err(|e| Error::msg(e.to_string()))?;
    watcher.watch(root, RecursiveMode::Recursive).map_err(|e| Error::msg(e.to_string()))?;

    let head_log = PathBuf::from(git_dir(root)?).join("logs/HEAD");
    let (stop, stop_rx) = channel::<()>();
    let root = root.to_path_buf();
    let thread = std::thread::spawn(move || run_debounce(&root, &head_log, debounce, rx, stop_rx, emit));
    Ok(WatchHandle { _watcher: watcher, stop, thread: Some(thread) })
}

fn run_debounce(
    root: &Path,
    head_log: &Path,
    debounce: Duration,
    rx: Receiver<notify::Event>,
    stop: Receiver<()>,
    emit: impl Fn(Signal),
) {
    let mut pending: HashMap<String, ChangeKind> = HashMap::new();
    let mut git_moved = false;
    loop {
        if stop.try_recv().is_ok() {
            return;
        }
        match rx.recv_timeout(debounce) {
            Ok(event) => {
                let Some(kind) = classify(&event.kind) else { continue };
                for path in &event.paths {
                    if path == head_log {
                        git_moved = true;
                    } else if let Ok(rel) = path.strip_prefix(root) {
                        let rel = rel.to_string_lossy();
                        if !rel.is_empty() && !is_ignored(&rel) && !path.is_dir() {
                            merge_change(&mut pending, &rel, kind);
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if git_moved {
                    git_moved = false;
                    emit(Signal::GitActivity);
                }
                if !pending.is_empty() {
                    let changes = pending.drain().map(|(path, change)| FileChange { path, change }).collect();
                    emit(Signal::Files(changes));
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
