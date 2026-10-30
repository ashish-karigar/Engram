use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engram_core::Result;
use engram_core::db::SharedDb;
use engram_core::events::ensure_actor;

use crate::git::{ensure_repo, head_sha};
use crate::ingest::{record_file_changes, record_snapshot, sync_commits};
use crate::snapshot::{SNAPSHOT_REF, snapshot_worktree};
use crate::watcher::{FileChange, Signal, WatchHandle, watch_project};

pub struct CaptureOptions {
    pub db: SharedDb,
    pub root: PathBuf,
    pub debounce: Duration,
    /// Minimum gap between snapshots so busy editing does not create a commit per keystroke.
    pub snapshot_interval: Duration,
    /// Called after file changes are recorded, e.g. to re-index code.
    pub on_change: Option<Box<dyn Fn(&[FileChange]) + Send>>,
}

impl CaptureOptions {
    pub fn new(db: SharedDb, root: &Path) -> Self {
        CaptureOptions {
            db,
            root: root.to_path_buf(),
            debounce: Duration::from_millis(500),
            snapshot_interval: Duration::from_secs(15),
            on_change: None,
        }
    }
}

enum Msg {
    Signal(Signal),
    SnapshotNow(Sender<()>),
    Stop,
}

pub struct Capture {
    tx: Sender<Msg>,
    watch: Option<WatchHandle>,
    worker: Option<JoinHandle<()>>,
}

impl Capture {
    /// Force a snapshot now, e.g. before shutdown. Blocks until it is done.
    pub fn snapshot_now(&self) {
        let (done, wait) = channel();
        if self.tx.send(Msg::SnapshotNow(done)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(30));
        }
    }

    pub fn stop(mut self) {
        if let Some(watch) = self.watch.take() {
            watch.close();
        }
        let _ = self.tx.send(Msg::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Start watching. One worker thread owns all writes, so ordering is simple and race-free.
pub fn start_capture(opts: CaptureOptions) -> Result<Capture> {
    ensure_repo(&opts.root)?;
    let actor = ensure_actor(&opts.db.lock().unwrap(), "system", "engram-watcher")?.id;
    sync_commits(&opts.db.lock().unwrap(), &opts.root, &actor, 200)?;

    let (tx, rx) = channel::<Msg>();
    let signal_tx = tx.clone();
    let watch = watch_project(&opts.root, opts.debounce, move |s| {
        let _ = signal_tx.send(Msg::Signal(s));
    })?;
    let worker = std::thread::spawn(move || run_worker(opts, actor, rx));
    Ok(Capture { tx, watch: Some(watch), worker: Some(worker) })
}

fn run_worker(opts: CaptureOptions, actor: String, rx: Receiver<Msg>) {
    let mut dirty_since: Option<Instant> = None;
    let mut last_snapshot = Instant::now() - opts.snapshot_interval;

    let snapshot = |last: &mut Instant| {
        *last = Instant::now();
        if let Ok(Some(snap)) = snapshot_worktree(&opts.root, SNAPSHOT_REF) {
            let _ = record_snapshot(&opts.db.lock().unwrap(), &actor, &snap);
        }
    };

    loop {
        let wait = match dirty_since {
            Some(_) => opts.snapshot_interval.saturating_sub(last_snapshot.elapsed()).max(Duration::from_millis(50)),
            None => Duration::from_secs(3600),
        };
        match rx.recv_timeout(wait) {
            Ok(Msg::Signal(Signal::Files(changes))) => {
                let sha = head_sha(&opts.root);
                let _ = record_file_changes(&opts.db.lock().unwrap(), &actor, &changes, sha);
                dirty_since.get_or_insert_with(Instant::now);
                if let Some(cb) = &opts.on_change {
                    cb(&changes);
                }
            }
            Ok(Msg::Signal(Signal::GitActivity)) => {
                let _ = sync_commits(&opts.db.lock().unwrap(), &opts.root, &actor, 200);
            }
            Ok(Msg::SnapshotNow(done)) => {
                snapshot(&mut last_snapshot);
                dirty_since = None;
                let _ = done.send(());
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if dirty_since.is_some() && last_snapshot.elapsed() >= opts.snapshot_interval {
                    snapshot(&mut last_snapshot);
                    dirty_since = None;
                }
            }
        }
    }
}
