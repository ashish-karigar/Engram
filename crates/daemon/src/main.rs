use std::path::PathBuf;

use clap::{Parser, Subcommand};
use engram_daemon::connectors::AGENTS;
use engram_daemon::paths::{app_data_dir, project_paths};
use engram_daemon::{DaemonOptions, start_daemon};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "engram", about = "Shared memory for coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Engram daemon (watches every added project).
    Daemon {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Start watching a project folder.
    Add { dir: PathBuf },
    /// Stop watching a project folder.
    Remove { dir: PathBuf },
    /// List watched projects.
    List,
    /// Print the project brief for an agent hook (reads the hook's JSON on stdin). Never fails.
    Hook { agent: String },
    /// Connect an agent (claude, codex, muse) to a project. Shows the change; --apply makes it.
    Connect {
        agent: String,
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        apply: bool,
    },
}

struct Client {
    url: String,
    token: String,
}

impl Client {
    fn discover() -> Result<Client, String> {
        let text = std::fs::read_to_string(app_data_dir().join("runtime.json")).map_err(|_| "Engram daemon is not running. Start it with: engram daemon")?;
        let info: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(Client { url: info["url"].as_str().unwrap_or("").into(), token: info["adminToken"].as_str().unwrap_or("").into() })
    }

    fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let mut req = reqwest::blocking::Client::new().request(method, format!("{}{path}", self.url)).bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().map_err(|e| e.to_string())?;
        let ok = res.status().is_success();
        let value: Value = res.json().unwrap_or(Value::Null);
        if ok { Ok(value) } else { Err(value["error"].as_str().unwrap_or("request failed").into()) }
    }
}

fn project_id(dir: &PathBuf) -> Result<String, String> {
    project_paths(dir, &app_data_dir()).map(|p| p.id).map_err(|e| format!("{}: {e}", dir.display()))
}

#[tokio::main]
async fn main() {
    if let Err(message) = run(Cli::parse().command).await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Daemon { port } => {
            let daemon = start_daemon(DaemonOptions { port, ..Default::default() }).await.map_err(|e| e.to_string())?;
            println!("Engram daemon at {}", daemon.url);
            println!("ENGRAM_READY {}", json!({"url": daemon.url, "adminToken": daemon.admin_token}));
            tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
            daemon.stop().await;
            Ok(())
        }
        Command::Hook { agent } => {
            let code = tokio::task::spawn_blocking(move || engram_daemon::hook::run_hook(&agent)).await.unwrap_or(0);
            std::process::exit(code);
        }
        other => tokio::task::spawn_blocking(move || client_command(other)).await.map_err(|e| e.to_string())?,
    }
}

fn client_command(command: Command) -> Result<(), String> {
    let client = Client::discover()?;
    match command {
        Command::Add { dir } => {
            let root = dir.canonicalize().map_err(|e| e.to_string())?;
            let added = client.call(reqwest::Method::POST, "/api/projects", Some(json!({"root": root})))?;
            println!("watching {} ({})", added["name"], added["id"]);
        }
        Command::Remove { dir } => {
            client.call(reqwest::Method::DELETE, &format!("/api/projects/{}", project_id(&dir)?), None)?;
            println!("stopped watching {}", dir.display());
        }
        Command::List => {
            for p in client.call(reqwest::Method::GET, "/api/projects", None)?.as_array().into_iter().flatten() {
                println!("{}  {}  {} memories", p["id"].as_str().unwrap_or(""), p["root"].as_str().unwrap_or(""), p["stats"]["memories"]);
            }
        }
        Command::Connect { agent, dir, apply } => {
            if !AGENTS.iter().any(|a| a.id() == agent) {
                return Err(format!("unknown agent {agent}; use claude, codex or muse"));
            }
            let id = project_id(&dir)?;
            let out = client.call(reqwest::Method::POST, &format!("/p/{id}/api/connect"), Some(json!({"agent": agent, "apply": apply})))?;
            println!("{}", out["preview"].as_str().unwrap_or(""));
            match out["result"].as_str() {
                Some(done) => println!("\n{done}"),
                None => println!("\nNothing changed. Re-run with --apply to make this change."),
            }
        }
        Command::Daemon { .. } | Command::Hook { .. } => unreachable!(),
    }
    Ok(())
}
