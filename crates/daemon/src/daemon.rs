use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use engram_core::{Error, Result};
use engram_serve::{ConnectOutcome, ProjectManager, ProjectRef, RunningServer, start_server};
use engram_understand::llm::LlmRequest;
use serde_json::{Value, json};

use crate::connectors::{Agent, Endpoint, plan_connect};
use crate::paths::{app_data_dir, project_paths};
use crate::project::{RunningProject, RuntimeOptions};
use crate::registry::Registry;
use crate::secrets::{agent_for_token, ensure_admin_token, ensure_agent_token, write_private};
use crate::settings::Settings;

/// Agents' configs hold the endpoint URL, so the port must survive restarts.
/// `ENGRAM_PORT` overrides it; if it is taken, a free port is used and agents must reconnect.
pub const DEFAULT_PORT: u16 = 41741;

pub fn configured_port() -> u16 {
    std::env::var("ENGRAM_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

pub struct DaemonOptions {
    pub base: PathBuf,
    /// 0 picks a free port.
    pub port: u16,
    /// Overrides the saved settings (tests). None means: use `settings.json`.
    pub runtime: Option<RuntimeOptions>,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        DaemonOptions { base: app_data_dir(), port: configured_port(), runtime: None }
    }
}

/// Owns every running project and implements what the HTTP server asks of it.
pub struct Daemon {
    base: PathBuf,
    settings: RwLock<Settings>,
    runtime: RwLock<RuntimeOptions>,
    registry: Registry,
    projects: Mutex<HashMap<String, RunningProject>>,
    url: RwLock<String>,
}

impl Daemon {
    fn new(opts: &DaemonOptions) -> Daemon {
        let settings = Settings::load(&opts.base);
        let runtime = opts.runtime.clone().unwrap_or_else(|| settings.runtime());
        Daemon {
            base: opts.base.clone(),
            settings: RwLock::new(settings),
            runtime: RwLock::new(runtime),
            registry: Registry::new(&opts.base),
            projects: Mutex::new(HashMap::new()),
            url: RwLock::new(String::new()),
        }
    }

    fn start_project(&self, root: &Path) -> Result<ProjectRef> {
        let paths = project_paths(root, &self.base)?;
        if let Some(existing) = self.get(&paths.id) {
            return Ok(existing);
        }
        let runtime = self.runtime.read().unwrap().clone();
        let project = RunningProject::start(root, &self.base, &runtime)?;
        let handle = project.handle();
        self.projects.lock().unwrap().insert(project.info.id.clone(), project);
        Ok(handle)
    }

    /// Database handle of a running project. For tests that need to inspect state directly.
    pub fn get_for_test(&self, id: &str) -> engram_core::db::SharedDb {
        self.get(id).expect("project is running").db
    }

    fn stop_all(&self) {
        let running: Vec<_> = self.projects.lock().unwrap().drain().map(|(_, p)| p).collect();
        for p in running {
            p.stop();
        }
    }
}

impl ProjectManager for Daemon {
    fn list(&self) -> Vec<ProjectRef> {
        let mut all: Vec<_> =
            self.projects.lock().unwrap().values().map(RunningProject::handle).collect();
        all.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        all
    }

    fn get(&self, id: &str) -> Option<ProjectRef> {
        self.projects.lock().unwrap().get(id).map(RunningProject::handle)
    }

    fn add(&self, root: &Path) -> Result<ProjectRef> {
        let root = root.canonicalize().map_err(|e| Error::msg(format!("{}: {e}", root.display())))?;
        if !root.is_dir() {
            return Err(Error::msg(format!("not a folder: {}", root.display())));
        }
        let project = self.start_project(&root)?;
        self.registry.add(&root);
        Ok(project)
    }

    fn remove(&self, id: &str) -> Result<()> {
        let project = self.projects.lock().unwrap().remove(id).ok_or_else(|| Error::msg("unknown project"))?;
        self.registry.remove(&project.paths.root);
        project.stop();
        Ok(())
    }

    fn connect(&self, agent: &str, apply: bool) -> Result<ConnectOutcome> {
        let agent = Agent::parse(agent).ok_or_else(|| Error::msg(format!("unknown agent: {agent}")))?;
        let token = ensure_agent_token(&self.base, agent.actor_name())?;
        let endpoint = Endpoint { base_url: self.url.read().unwrap().clone(), token };
        let plan = plan_connect(agent, endpoint);
        let preview = plan.preview.clone();
        let result = if apply { Some(plan.apply()?) } else { None };
        Ok(ConnectOutcome { agent: agent.id().into(), preview, result })
    }

    fn agent_for_token(&self, token: &str) -> Option<String> {
        agent_for_token(&self.base, token)
    }

    fn settings(&self) -> Result<Value> {
        let mut view = self.settings.read().unwrap().public();
        view["dataDir"] = json!(self.base);
        Ok(view)
    }

    /// Save, then restart every project so the new models and intervals take effect.
    /// Switching embedding models re-embeds chunks on restart (vectors are stored per model).
    fn update_settings(&self, incoming: Value) -> Result<Value> {
        let next = self.settings.read().unwrap().merged(incoming)?;
        next.save(&self.base)?;
        *self.runtime.write().unwrap() = next.runtime();
        *self.settings.write().unwrap() = next;
        let running: Vec<RunningProject> = self.projects.lock().unwrap().drain().map(|(_, p)| p).collect();
        for project in running {
            let root = project.paths.root.clone();
            project.stop();
            if let Err(err) = self.start_project(&root) {
                eprintln!("engram: could not restart {}: {err}", root.display());
            }
        }
        self.settings()
    }

    fn test_settings(&self, target: &str, draft: Value) -> Result<Value> {
        let saved = self.settings.read().unwrap().clone();
        let settings = if draft.is_null() { saved } else { saved.merged(draft)? };
        let started = Instant::now();
        let detail = match target {
            "llm" => {
                let client = settings.llm_client().ok_or_else(|| Error::msg("No model is configured (provider is none or the API key is missing)."))?;
                // Reasoning models spend part of max_tokens thinking, so a tiny budget can come back empty.
                let reply = client.complete(&LlmRequest { system: "Reply with the single word OK.".into(), prompt: "ping".into(), max_tokens: 1000 })?;
                format!("{} replied: {}", client.name(), reply.trim().chars().take(40).collect::<String>())
            }
            "embeddings" => {
                let embedder = settings.embedder();
                let v = embedder.embed(&["hello world".into()])?;
                format!("{} returned a {}-dimensional vector", embedder.model_id(), v.first().map_or(0, Vec::len))
            }
            other => return Err(Error::msg(format!("unknown test target: {other}"))),
        };
        Ok(json!({"ok": true, "detail": detail, "latencyMs": started.elapsed().as_millis() as u64}))
    }
}

pub struct RunningDaemon {
    pub url: String,
    pub admin_token: String,
    pub daemon: Arc<Daemon>,
    server: RunningServer,
    base: PathBuf,
}

impl RunningDaemon {
    pub async fn stop(self) {
        self.server.close().await;
        let daemon = self.daemon.clone();
        let _ = tokio::task::spawn_blocking(move || daemon.stop_all()).await;
        let _ = std::fs::remove_file(self.base.join("runtime.json"));
    }
}

/// Start the server, restore remembered projects, and publish `runtime.json` so the CLI can find us.
pub async fn start_daemon(opts: DaemonOptions) -> std::io::Result<RunningDaemon> {
    std::fs::create_dir_all(&opts.base)?;
    let admin_token = ensure_admin_token(&opts.base)?;
    let daemon = Arc::new(Daemon::new(&opts));
    let manager: Arc<dyn ProjectManager> = daemon.clone();
    let server = match start_server(manager.clone(), admin_token.clone(), opts.port).await {
        Err(err) if opts.port != 0 => {
            eprintln!("engram: port {} is busy ({err}); using a free port. Reconnect agents to pick it up.", opts.port);
            start_server(manager, admin_token.clone(), 0).await?
        }
        other => other?,
    };
    *daemon.url.write().unwrap() = server.url.clone();

    let restore = daemon.clone();
    let roots = restore.registry.load();
    let _ = tokio::task::spawn_blocking(move || {
        for root in roots {
            if let Err(err) = restore.start_project(&root) {
                eprintln!("skipping {}: {err}", root.display());
            }
        }
    })
    .await;

    let info = serde_json::json!({"url": server.url, "adminToken": admin_token, "pid": std::process::id()});
    write_private(&opts.base.join("runtime.json"), &serde_json::to_string_pretty(&info)?)?;
    Ok(RunningDaemon { url: server.url.clone(), admin_token, daemon, server, base: opts.base })
}
