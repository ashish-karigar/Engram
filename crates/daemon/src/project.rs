use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::Utc;
use engram_capture::git::{ensure_repo, git};
use engram_capture::{Capture, CaptureOptions, start_capture};
use engram_core::db::{SharedDb, open, shared};
use engram_core::decay::archive_stale;
use engram_core::{Connection, Result};
use engram_serve::ProjectInfo;
use engram_understand::codeindex::index_python;
use engram_understand::llm::LlmClient;
use engram_understand::pipeline::{PipelineOptions, run_pipeline};

use crate::paths::{ProjectPaths, project_paths};

/// Builds the extraction model. A factory, so each project thread owns its own client.
pub type LlmFactory = Arc<dyn Fn() -> Option<Box<dyn LlmClient>> + Send + Sync>;

#[derive(Clone)]
pub struct RuntimeOptions {
    pub llm: LlmFactory,
    pub pipeline_every: Duration,
    pub decay_every: Duration,
    pub capture_debounce: Duration,
    pub snapshot_interval: Duration,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        RuntimeOptions {
            llm: Arc::new(engram_understand::llm::client_from_env),
            pipeline_every: Duration::from_secs(60),
            decay_every: Duration::from_secs(6 * 3600),
            capture_debounce: Duration::from_millis(500),
            snapshot_interval: Duration::from_secs(15),
        }
    }
}

/// One watched project: its database, file capture, and background upkeep thread.
pub struct RunningProject {
    pub info: ProjectInfo,
    pub paths: ProjectPaths,
    pub db: SharedDb,
    capture: Option<Capture>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

fn list_python(root: &Path) -> Vec<String> {
    git(root, &["ls-files", "--cached", "--others", "--exclude-standard", "*.py"])
        .map(|out| out.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn reindex(conn: &Connection, root: &Path, changed: Option<&[String]>) {
    let all = list_python(root);
    let known: HashSet<String> = all.iter().cloned().collect();
    let paths: Vec<String> = match changed {
        Some(c) => c.iter().filter(|p| known.contains(*p)).cloned().collect(),
        None => all,
    };
    if !paths.is_empty() {
        let _ = index_python(conn, root, &paths, &known, None);
    }
}

impl RunningProject {
    pub fn start(root: &Path, base: &Path, opts: &RuntimeOptions) -> Result<RunningProject> {
        ensure_repo(root)?;
        let paths = project_paths(root, base)?;
        let db = shared(open(&paths.db)?);
        reindex(&db.lock().unwrap(), &paths.root, None);

        let mut capture_opts = CaptureOptions::new(db.clone(), &paths.root);
        capture_opts.debounce = opts.capture_debounce;
        capture_opts.snapshot_interval = opts.snapshot_interval;
        let (db_path, change_root) = (paths.db.clone(), paths.root.clone());
        capture_opts.on_change = Some(Box::new(move |changes| {
            // A separate connection keeps indexing from blocking agent requests.
            let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
            if let Ok(conn) = open(&db_path) {
                reindex(&conn, &change_root, Some(&paths));
            }
        }));
        let capture = start_capture(capture_opts)?;

        let stop = Arc::new(AtomicBool::new(false));
        let worker = spawn_upkeep(&paths, opts.clone(), stop.clone());
        let info = ProjectInfo { id: paths.id.clone(), name: paths.name.clone(), root: paths.root.to_string_lossy().into() };
        Ok(RunningProject { info, paths, db, capture: Some(capture), stop, worker: Some(worker) })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(capture) = self.capture.take() {
            capture.snapshot_now();
            capture.stop();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Background thread: extract memories from new events and archive stale ones.
/// It uses its own connection, so a slow model call never blocks agent requests.
fn spawn_upkeep(paths: &ProjectPaths, opts: RuntimeOptions, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    let (db_path, root) = (paths.db.clone(), paths.root.clone());
    std::thread::spawn(move || {
        let Ok(conn) = open(&db_path) else { return };
        let llm = (opts.llm)();
        let (mut last_pipeline, mut last_decay) = (Instant::now(), Instant::now());
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(llm) = llm.as_deref().filter(|_| last_pipeline.elapsed() >= opts.pipeline_every) {
                last_pipeline = Instant::now();
                let diff = |sha: &str| git(&root, &["show", "--stat", "--patch", "--format=", sha]).unwrap_or_default();
                let mut pipeline = PipelineOptions::new(llm);
                pipeline.get_diff = Some(&diff);
                let _ = run_pipeline(&conn, &pipeline);
            }
            if last_decay.elapsed() >= opts.decay_every {
                last_decay = Instant::now();
                let _ = archive_stale(&conn, Utc::now());
            }
        }
    })
}
