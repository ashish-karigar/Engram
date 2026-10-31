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
use engram_embed::{Embedder, embedder_from_env};
use engram_embed::store::{chunks_missing, embed_and_store, memories_missing};
use engram_rag::project::{ingest_file, should_index};
use engram_sessions::{Sources, sync_sessions};
use engram_serve::{ProjectInfo, ProjectRef};
use engram_code::{Lang, Resolver};
use engram_understand::codeindex::index_code;
use engram_understand::llm::LlmClient;
use engram_understand::pipeline::{PipelineOptions, run_pipeline};

use crate::paths::{ProjectPaths, project_paths};
use crate::settings::{IndexSettings, ignored};

/// How often agent session logs are checked for new turns. Reads only appended bytes.
const SESSIONS_EVERY: Duration = Duration::from_secs(10);
/// How often new memories are checked for a missing vector (search also catches up on demand).
const EMBED_EVERY: Duration = Duration::from_secs(5);

/// A local model holds most of the machine's memory while loaded, so extraction runs in short
/// bursts: at most `LOCAL_BURST` tasks, then every project rests for `LOCAL_REST` while the model
/// unloads. Hosted models have no such limit.
const LOCAL_BURST: usize = 3;
const LOCAL_REST: Duration = Duration::from_secs(5 * 60);

/// Whose turn it is with the local model: one project at a time, and nobody while it rests.
struct LocalTurn {
    busy: bool,
    resumes_at: Option<Instant>,
}
static LOCAL_TURN: std::sync::Mutex<LocalTurn> = std::sync::Mutex::new(LocalTurn { busy: false, resumes_at: None });

fn claim_local_turn() -> bool {
    let mut turn = LOCAL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    if turn.busy || turn.resumes_at.is_some_and(|t| Instant::now() < t) {
        return false;
    }
    turn.busy = true;
    true
}

/// End a turn; if the model was used, everyone rests while it unloads.
fn release_local_turn(used_model: bool) {
    let mut turn = LOCAL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    turn.busy = false;
    if used_model {
        turn.resumes_at = Some(Instant::now() + LOCAL_REST);
    }
}

/// Builds the extraction model. A factory, so each project thread owns its own client.
pub type LlmFactory = Arc<dyn Fn() -> Option<Box<dyn LlmClient>> + Send + Sync>;

#[derive(Clone)]
pub struct RuntimeOptions {
    pub llm: LlmFactory,
    /// Embedding model for documents and memories (`ENGRAM_EMBED`, default: built-in hashing).
    pub embedder: Arc<dyn Embedder>,
    pub pipeline_every: Duration,
    pub decay_every: Duration,
    pub capture_debounce: Duration,
    pub snapshot_interval: Duration,
    pub index: IndexSettings,
    /// Where agents keep their session logs.
    pub sources: Sources,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        RuntimeOptions {
            llm: Arc::new(engram_understand::llm::client_from_env),
            embedder: Arc::from(embedder_from_env()),
            pipeline_every: Duration::from_secs(60),
            decay_every: Duration::from_secs(6 * 3600),
            capture_debounce: Duration::from_millis(500),
            snapshot_interval: Duration::from_secs(15),
            index: IndexSettings::default(),
            sources: Sources::default(),
        }
    }
}

/// One watched project: its database, file capture, and background upkeep thread.
pub struct RunningProject {
    pub info: ProjectInfo,
    pub paths: ProjectPaths,
    pub db: SharedDb,
    pub embedder: Arc<dyn Embedder>,
    capture: Option<Capture>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

/// Files git would track (committed or new, minus ignored), optionally filtered by pathspec.
fn list_files(root: &Path, pathspec: Option<&str>) -> Vec<String> {
    let mut args = vec!["ls-files", "--cached", "--others", "--exclude-standard"];
    args.extend(pathspec);
    git(root, &args).map(|out| out.lines().map(String::from).collect()).unwrap_or_default()
}


/// Keep the document index in step with the folder: new and changed files are (re)ingested,
/// deleted ones removed. Unchanged files are skipped by content hash.
fn sync_documents(conn: &Connection, embedder: &dyn Embedder, root: &Path, paths: &[String], index: &IndexSettings) {
    let wanted = |p: &String| {
        let code = engram_rag::extract::is_code(p);
        should_index(p) && !ignored(p, &index.ignore) && if code { index.index_code } else { index.index_docs }
    };
    for path in paths {
        if !wanted(path) {
            let _ = engram_rag::project::remove_file(conn, path);
            continue;
        }
        if let Err(err) = ingest_file(conn, embedder, root, path, index.max_file_kb * 1024) {
            eprintln!("engram: could not index {path}: {err}");
        }
    }
}

/// Rebuild the code graph for changed source files (all of them when `changed` is None).
/// Cargo manifests are read so `use other_crate::...` resolves inside a workspace.
fn reindex(conn: &Connection, root: &Path, changed: Option<&[String]>) {
    let all = list_files(root, None);
    let known: HashSet<String> = all.iter().filter(|p| Lang::from_path(p).is_some()).cloned().collect();
    let paths: Vec<String> = match changed {
        Some(c) => c.iter().filter(|p| known.contains(*p)).cloned().collect(),
        None => known.iter().cloned().collect(),
    };
    if paths.is_empty() {
        return;
    }
    let manifests: Vec<(String, String)> = all
        .iter()
        .filter(|p| p.rsplit('/').next() == Some("Cargo.toml"))
        .filter_map(|p| Some((p.clone(), std::fs::read_to_string(root.join(p)).ok()?)))
        .collect();
    if let Err(err) = index_code(conn, root, &paths, &Resolver::new(&known, &manifests), None) {
        eprintln!("engram: code indexing failed: {err}");
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
        let (db_path, change_root, change_embedder, change_index) = (paths.db.clone(), paths.root.clone(), opts.embedder.clone(), opts.index.clone());
        capture_opts.on_change = Some(Box::new(move |changes| {
            // A separate connection keeps indexing from blocking agent requests.
            let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
            if let Ok(conn) = open(&db_path) {
                reindex(&conn, &change_root, Some(&paths));
                sync_documents(&conn, change_embedder.as_ref(), &change_root, &paths, &change_index);
            }
        }));
        let capture = start_capture(capture_opts)?;

        let stop = Arc::new(AtomicBool::new(false));
        let worker = spawn_upkeep(&paths, opts.clone(), stop.clone());
        let info = ProjectInfo { id: paths.id.clone(), name: paths.name.clone(), root: paths.root.to_string_lossy().into() };
        Ok(RunningProject { info, paths, db, embedder: opts.embedder.clone(), capture: Some(capture), stop, worker: Some(worker) })
    }

    pub fn handle(&self) -> ProjectRef {
        ProjectRef { info: self.info.clone(), db: self.db.clone(), db_path: Some(self.paths.db.clone()), embedder: self.embedder.clone() }
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
        // First pass: index the project's documents and code as searchable chunks.
        sync_documents(&conn, opts.embedder.as_ref(), &root, &list_files(&root, None), &opts.index);
        // After a model switch, chunks whose files did not change still need vectors for the new model.
        if let Ok(missing) = chunks_missing(&conn, &opts.embedder.model_id(), 10_000) {
            let _ = embed_and_store(&conn, opts.embedder.as_ref(), "chunk", &missing);
        }
        let llm = (opts.llm)();
        let (mut last_pipeline, mut last_decay) = (Instant::now(), Instant::now());
        let mut last_sessions = Instant::now() - SESSIONS_EVERY;
        let mut last_embed = Instant::now();
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            let due = last_pipeline.elapsed() >= opts.pipeline_every;
            if let Some(llm) = llm.as_deref().filter(|l| due && (!l.is_local() || claim_local_turn())) {
                last_pipeline = Instant::now();
                let diff = |sha: &str| git(&root, &["show", "--stat", "--patch", "--format=", sha]).unwrap_or_default();
                let mut pipeline = PipelineOptions::new(llm);
                pipeline.get_diff = Some(&diff);
                if llm.is_local() {
                    pipeline.max_tasks = Some(LOCAL_BURST);
                }
                let report = run_pipeline(&conn, &pipeline);
                if llm.is_local() {
                    release_local_turn(report.is_ok_and(|r| r.tasks > 0 || r.error.is_some()));
                }
            }
            if opts.index.sessions && last_sessions.elapsed() >= SESSIONS_EVERY {
                last_sessions = Instant::now();
                if let Err(err) = sync_sessions(&conn, opts.embedder.as_ref(), &root, &opts.sources) {
                    eprintln!("engram: session sync failed: {err}");
                }
            }
            if last_embed.elapsed() >= EMBED_EVERY {
                last_embed = Instant::now();
                if let Ok(missing) = memories_missing(&conn, &opts.embedder.model_id(), 200) {
                    if !missing.is_empty() {
                        let _ = embed_and_store(&conn, opts.embedder.as_ref(), "memory", &missing);
                    }
                }
            }
            if last_decay.elapsed() >= opts.decay_every {
                last_decay = Instant::now();
                let _ = archive_stale(&conn, Utc::now());
            }
        }
    })
}
