use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use engram_capture::git::git;
use engram_capture::snapshot::snapshot_changes;
use engram_core::events::list_events;
use engram_daemon::connectors::{Agent, Endpoint, codex_plan, merge_mcp_server, muse_plan, upsert_block};
use engram_daemon::project::RuntimeOptions;
use engram_daemon::secrets::ensure_token;
use engram_daemon::{DaemonOptions, start_daemon};
use engram_understand::llm::{LlmClient, ScriptedClient};
use serde_json::{Value, json};
use tempfile::TempDir;

fn sh(dir: &Path, args: &[&str]) {
    assert!(Command::new("git").args(args).current_dir(dir).output().unwrap().status.success(), "git {args:?}");
}

fn repo(files: &[(&str, &str)], subject: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    sh(p, &["init", "-q"]);
    sh(p, &["config", "user.email", "t@e.com"]);
    sh(p, &["config", "user.name", "T"]);
    for (path, body) in files {
        let full = p.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    sh(p, &["add", "-A"]);
    sh(p, &["commit", "-q", "-m", subject]);
    dir
}

#[test]
fn connector_edits_are_idempotent_and_preserve_existing_settings() {
    let once = upsert_block("model = \"x\"\n", "[mcp_servers.engram]\nurl = \"a\"");
    let twice = upsert_block(&once, "[mcp_servers.engram]\nurl = \"b\"");
    assert_eq!(twice.matches(">>> engram").count(), 1);
    assert!(twice.contains("url = \"b\"") && twice.contains("model = \"x\""));

    let merged: Value = serde_json::from_str(&merge_mcp_server(r#"{"model":"m","mcpServers":{"other":{"url":"u"}}}"#, json!({"url": "e"})).unwrap()).unwrap();
    assert_eq!(merged["model"], "m");
    assert!(merged["mcpServers"]["other"].is_object() && merged["mcpServers"]["engram"].is_object());
}

#[test]
fn connecting_codex_and_muse_writes_only_the_target_file() {
    let dir = tempfile::tempdir().unwrap();
    let ep = || Endpoint { base_url: "http://127.0.0.1:1/p/abc".into(), token: "egm_secret".into() };

    let toml = dir.path().join("codex.toml");
    std::fs::write(&toml, "model = \"o\"\n").unwrap();
    let plan = codex_plan(ep(), toml.clone());
    assert!(!plan.preview.contains("egm_secret"), "preview must not leak the token");
    plan.apply().unwrap();
    let written = std::fs::read_to_string(&toml).unwrap();
    assert!(written.contains("model = \"o\"") && written.contains("/p/abc/mcp") && written.contains("egm_secret"));
    assert!(dir.path().join("codex.toml.engram.bak").exists());

    let json_file = dir.path().join("settings.json");
    muse_plan(ep(), json_file.clone()).apply().unwrap();
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(&json_file).unwrap()).unwrap();
    assert_eq!(settings["mcpServers"]["engram"]["url"], "http://127.0.0.1:1/p/abc/mcp");
    assert_eq!(Agent::parse("muse").unwrap().actor_name(), "muse-code");
}

struct Api {
    url: String,
    admin: String,
}

impl Api {
    async fn get(&self, token: &str, path: &str) -> Value {
        reqwest::Client::new().get(format!("{}{path}", self.url)).bearer_auth(token).send().await.unwrap().json().await.unwrap_or(Value::Null)
    }
    async fn post(&self, token: &str, path: &str, body: Value) -> Value {
        reqwest::Client::new().post(format!("{}{path}", self.url)).bearer_auth(token).json(&body).send().await.unwrap().json().await.unwrap_or(Value::Null)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn end_to_end_two_projects_two_agents_recovery() {
    let home = tempfile::tempdir().unwrap();
    let drive = repo(
        &[
            ("perception/lidar.py", "def voxel_filter(points): pass\n"),
            ("perception/fusion.py", "from perception.lidar import voxel_filter\nclass Fusion: pass\n"),
            ("docs/design.md", "# Perception design\n\n## Lidar\n\nlidar.py downsamples with a voxel grid because radius filtering missed the 100ms frame budget.\n"),
        ],
        "Use voxel grid filter instead of radius filter for lidar downsampling",
    );
    let web = repo(&[("site/app.py", "def serve(): pass\n")], "Add site");

    let reply = json!({"memories": [{
        "type": "decision", "title": "Lidar downsampling", "claim": "Downsample lidar with a voxel grid filter",
        "rationale": "radius filter was too slow", "quote": "voxel grid filter instead of radius filter",
        "evidence": [1], "files": ["perception/lidar.py"], "confidence": 0.9
    }]})
    .to_string();
    let mut runtime = RuntimeOptions::default();
    // Only the first project to ask gets the scripted model; the web project has nothing to extract.
    runtime.llm = Arc::new(move || Some(Box::new(ScriptedClient::new(vec![reply.clone()])) as Box<dyn LlmClient>));
    runtime.pipeline_every = Duration::from_millis(200);
    runtime.capture_debounce = Duration::from_millis(50);
    runtime.snapshot_interval = Duration::from_millis(100);
    let running = start_daemon(DaemonOptions { base: home.path().to_path_buf(), port: 0, runtime: Some(runtime) }).await.unwrap();
    let api = Api { url: running.url.clone(), admin: running.admin_token.clone() };

    let drive_p = api.post(&api.admin, "/api/projects", json!({"root": drive.path()})).await;
    let web_p = api.post(&api.admin, "/api/projects", json!({"root": web.path()})).await;
    let (drive_id, web_id) = (drive_p["id"].as_str().unwrap().to_string(), web_p["id"].as_str().unwrap().to_string());
    assert_ne!(drive_id, web_id);
    assert_eq!(api.get(&api.admin, "/api/projects").await.as_array().unwrap().len(), 2);

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let graph = api.get(&api.admin, &format!("/p/{drive_id}/api/graph")).await;
    let labels: Vec<&str> = graph["nodes"].as_array().unwrap().iter().map(|n| n["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"perception/fusion.py"), "python indexed: {labels:?}");
    assert!(graph["edges"].as_array().unwrap().iter().any(|e| e["type"] == "imports"));
    assert!(labels.contains(&"Lidar downsampling"), "memory extracted from the commit: {labels:?}");
    let web_graph = api.get(&api.admin, &format!("/p/{web_id}/api/graph")).await;
    assert!(!web_graph["nodes"].as_array().unwrap().iter().any(|n| n["label"] == "perception/fusion.py"), "projects are isolated");

    // Agents get their own tokens through the connect flow and share the project's memory.
    let claude_preview = api.post(&api.admin, &format!("/p/{drive_id}/api/connect"), json!({"agent": "claude"})).await;
    assert!(claude_preview["preview"].as_str().unwrap().contains("claude mcp add"));
    let paths = engram_daemon::paths::project_paths(drive.path(), home.path()).unwrap();
    let token = {
        let project = running.daemon.get_for_test(&drive_id);
        ensure_token(&project.lock().unwrap(), &paths.tokens, "agent", "claude-code").unwrap()
    };
    api.post(&token, &format!("/p/{drive_id}/api/note"), json!({"type": "dead_end", "title": "Radius filter", "claim": "Radius outlier removal took 80ms per frame", "files": ["perception/lidar.py"]})).await;
    let answer = api.post(&api.admin, &format!("/p/{drive_id}/api/search"), json!({"query": "why not radius filter lidar"})).await;
    let text = answer["text"].as_str().unwrap();
    assert!(text.contains("voxel grid") && text.contains("80ms"), "{text}");

    // Project documents and code are indexed as searchable chunks, linked to the code they mention.
    let docs = api.get(&api.admin, &format!("/p/{drive_id}/api/documents")).await;
    let titles: Vec<&str> = docs.as_array().unwrap().iter().map(|d| d["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"docs/design.md") && titles.contains(&"perception/lidar.py"), "{titles:?}");
    let grounded = api.post(&api.admin, &format!("/p/{drive_id}/api/search"), json!({"query": "lidar frame budget", "mode": "documents"})).await;
    assert!(grounded["text"].as_str().unwrap().contains("Perception design > Lidar"));

    // Uncommitted work survives on the hidden ref.
    std::fs::write(drive.path().join("perception/planner.py"), "def plan(): pass\n").unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(snapshot_changes(drive.path(), "refs/engram/snapshots"), ["perception/planner.py"]);
    assert!(git(drive.path(), &["show", "refs/engram/snapshots:perception/planner.py"]).unwrap().contains("def plan"));

    let log = api.get(&api.admin, &format!("/p/{drive_id}/api/requests")).await;
    assert!(log.as_array().unwrap().iter().any(|r| r["actor"] == "claude-code" && r["tool"] == "note"));

    // Removing a project stops it; the other keeps running.
    let removed = reqwest::Client::new().delete(format!("{}/api/projects/{web_id}", api.url)).bearer_auth(&api.admin).send().await.unwrap();
    assert!(removed.status().is_success());
    assert_eq!(api.get(&api.admin, "/api/projects").await.as_array().unwrap().len(), 1);
    let events = running.daemon.get_for_test(&drive_id);
    assert!(!list_events(&events.lock().unwrap(), 0, Some("snapshot.created"), 10).unwrap().is_empty());

    running.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_saved_masked_validated_and_applied() {
    let home = tempfile::tempdir().unwrap();
    let repo = repo(&[("notes.md", "# Notes\n\nhello")], "init");
    let running = start_daemon(DaemonOptions { base: home.path().to_path_buf(), port: 0, runtime: None }).await.unwrap();
    let api = Api { url: running.url.clone(), admin: running.admin_token.clone() };
    api.post(&api.admin, "/api/projects", json!({"root": repo.path()})).await;

    let mut s = api.get(&api.admin, "/api/settings").await;
    assert_eq!(s["embeddings"]["provider"], "hash");
    s["llm"]["provider"] = json!("meta");
    s["llm"]["metaApiKey"] = json!("sk-meta-secret-1234");
    s["indexing"]["ignore"] = json!(["*.md"]);
    let saved = api.post(&api.admin, "/api/settings", s.clone()).await;
    assert_eq!(saved["llm"]["metaApiKey"], "••••1234", "the key never comes back in full");
    let on_disk = std::fs::read_to_string(home.path().join("settings.json")).unwrap();
    assert!(on_disk.contains("sk-meta-secret-1234"));

    // Sending the masked value back keeps the stored key.
    let again = api.post(&api.admin, "/api/settings", saved.clone()).await;
    assert_eq!(again["llm"]["metaApiKey"], "••••1234");

    let mut bad = saved.clone();
    bad["embeddings"]["provider"] = json!("magic");
    let res = reqwest::Client::new().post(format!("{}/api/settings", api.url)).bearer_auth(&api.admin).json(&bad).send().await.unwrap();
    assert_eq!(res.status(), 400);

    let ok = api.post(&api.admin, "/api/settings/test", json!({"target": "embeddings"})).await;
    assert!(ok["detail"].as_str().unwrap().contains("384"));

    // The ignore rule applied on restart: the markdown file left the index.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let projects = api.get(&api.admin, "/api/projects").await;
    let id = projects[0]["id"].as_str().unwrap();
    let docs = api.get(&api.admin, &format!("/p/{id}/api/documents")).await;
    assert!(docs.as_array().unwrap().is_empty(), "{docs}");
    running.stop().await;
}
