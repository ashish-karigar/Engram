use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use engram_core::db::{open_memory, shared};
use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::memory::{NewMemory, create_memory};
use engram_core::{Connection, Error, Result};
use engram_serve::budget::{Packable, estimate_tokens, pack_by_density};
use engram_serve::recall::{RecallOpts, recall};
use engram_serve::tokens::{issue_token, verify_token};
use engram_serve::{ConnectOutcome, ProjectInfo, ProjectManager, ProjectRef, start_server};
use serde_json::{Value, json};

/// Two in-memory projects, enough to test routing and isolation without a daemon.
struct TestManager {
    projects: Mutex<HashMap<String, ProjectRef>>,
}

impl ProjectManager for TestManager {
    fn list(&self) -> Vec<ProjectRef> {
        self.projects.lock().unwrap().values().cloned().collect()
    }
    fn get(&self, id: &str) -> Option<ProjectRef> {
        self.projects.lock().unwrap().get(id).cloned()
    }
    fn add(&self, root: &Path) -> Result<ProjectRef> {
        let id = format!("p{}", self.projects.lock().unwrap().len() + 1);
        let info = ProjectInfo { id: id.clone(), name: root.to_string_lossy().into(), root: root.to_string_lossy().into() };
        let project = ProjectRef { info, db: shared(open_memory()?) };
        self.projects.lock().unwrap().insert(id, project.clone());
        Ok(project)
    }
    fn remove(&self, id: &str) -> Result<()> {
        self.projects.lock().unwrap().remove(id).map(|_| ()).ok_or_else(|| Error::msg("unknown project"))
    }
    fn connect(&self, _id: &str, agent: &str, apply: bool) -> Result<ConnectOutcome> {
        Ok(ConnectOutcome { agent: agent.into(), preview: "claude mcp add ...".into(), result: apply.then(|| "done".into()) })
    }
}

fn seed(conn: &Connection) {
    let auth = upsert_entity(conn, "file", "backend/auth.py", Some("backend/auth.py"), None).unwrap();
    let web = upsert_entity(conn, "file", "web/login.py", Some("web/login.py"), None).unwrap();
    add_edge(conn, &web.id, &auth.id, "imports", EdgeOpts::default()).unwrap();
    let mut m = NewMemory::new("decision", "Auth strategy", "Use signed session cookies");
    m.rationale = Some("Both sites are server rendered".into());
    m.touches = vec![auth.id.clone()];
    create_memory(conn, m).unwrap();
    let mut m = NewMemory::new("dead_end", "Login redirect loop", "Cookie SameSite=Strict broke the redirect from site B");
    m.touches = vec![web.id.clone()];
    create_memory(conn, m).unwrap();
    create_memory(conn, NewMemory::new("dead_end", "Radius outlier filter", "Radius outlier removal took 80ms per frame on lidar")).unwrap();
    create_memory(conn, NewMemory::new("fact", "Lidar mount", "Lidar is mounted on the roof")).unwrap();
    create_memory(conn, NewMemory::new("fact", "Billing", "Stripe webhooks are verified in billing.py")).unwrap();
}

struct Fixture {
    url: String,
    admin: String,
    claude: String,
    codex: String,
    manager: Arc<TestManager>,
    _server: engram_serve::RunningServer,
}

async fn fixture() -> Fixture {
    let db = open_memory().unwrap();
    seed(&db);
    let (claude, _) = issue_token(&db, "agent", "claude-code").unwrap();
    let (codex, _) = issue_token(&db, "agent", "codex").unwrap();
    let info = ProjectInfo { id: "p1".into(), name: "drive".into(), root: "/tmp/drive".into() };
    let manager = Arc::new(TestManager { projects: Mutex::new(HashMap::from([("p1".to_string(), ProjectRef { info, db: shared(db) })])) });
    let server = start_server(manager.clone(), "admin-secret".into(), 0).await.unwrap();
    Fixture { url: server.url.clone(), admin: "admin-secret".into(), claude, codex, manager, _server: server }
}

async fn post(f: &Fixture, token: &str, path: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new().post(format!("{}{path}", f.url)).bearer_auth(token).json(&body).send().await.unwrap();
    (res.status().as_u16(), res.json().await.unwrap_or(Value::Null))
}

async fn get(f: &Fixture, token: &str, path: &str) -> (u16, Value) {
    let res = reqwest::Client::new().get(format!("{}{path}", f.url)).bearer_auth(token).send().await.unwrap();
    (res.status().as_u16(), res.json().await.unwrap_or(Value::Null))
}

#[test]
fn tokens_are_per_agent_and_hashed() {
    let db = open_memory().unwrap();
    let (token, actor) = issue_token(&db, "agent", "claude-code").unwrap();
    assert_eq!(verify_token(&db, &token).unwrap().unwrap().id, actor.id);
    assert!(verify_token(&db, "egm_wrong").unwrap().is_none());
    let stored: String = db.query_row("SELECT token_hash FROM tokens", [], |r| r.get(0)).unwrap();
    assert_ne!(stored, token);
}

#[test]
fn packing_prefers_dense_items_and_respects_the_budget() {
    let items = vec![
        Packable { item: "big", value: 5.0, cost: 100 },
        Packable { item: "dense", value: 4.0, cost: 10 },
        Packable { item: "ok", value: 2.0, cost: 10 },
    ];
    let (packed, used) = pack_by_density(items, 25);
    assert_eq!(packed, ["dense", "ok"]);
    assert_eq!(used, 20);
    assert_eq!(estimate_tokens("abcdefgh"), 2);
}

#[test]
fn recall_reaches_memories_through_the_graph_and_ignores_question_words() {
    let db = open_memory().unwrap();
    seed(&db);
    let titles = |q: &str| recall(&db, q, &RecallOpts::default()).unwrap().into_iter().map(|i| i.memory.title).collect::<Vec<_>>();
    let auth = titles("backend auth");
    assert_eq!(auth[0], "Auth strategy");
    assert!(auth.contains(&"Login redirect loop".to_string()), "reached via web/login.py imports backend/auth.py");
    assert!(!auth.contains(&"Billing".to_string()));
    assert_eq!(titles("why not radius filter lidar")[0], "Radius outlier filter");
}

#[tokio::test]
async fn requests_need_a_valid_token() {
    let f = fixture().await;
    let client = reqwest::Client::new();
    assert_eq!(client.get(format!("{}/p/p1/api/stats", f.url)).send().await.unwrap().status(), 401);
    assert_eq!(get(&f, "nope", "/p/p1/api/stats").await.0, 401);
    assert_eq!(get(&f, &f.claude, "/p/p1/api/stats").await.0, 200);
}

#[tokio::test]
async fn http_ops_work_and_every_call_is_attributed() {
    let f = fixture().await;
    let (status, body) = post(&f, &f.codex, "/p/p1/api/search", json!({"query": "why session cookies"})).await;
    assert_eq!(status, 200);
    assert!(body["text"].as_str().unwrap().contains("Auth strategy"));
    let (_, log) = get(&f, &f.admin, "/p/p1/api/requests").await;
    assert_eq!(log[0]["actor"], "codex");
    assert_eq!(log[0]["tool"], "search");
    assert!(!log[0]["returned"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn two_agents_share_one_memory_over_mcp() {
    let f = fixture().await;
    let rpc = |token: String, id: i64, method: &str, params: Value| {
        let (url, method) = (format!("{}/p/p1/mcp", f.url), method.to_string());
        async move {
            let res = reqwest::Client::new()
                .post(url)
                .bearer_auth(token)
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
                .send()
                .await
                .unwrap();
            res.json::<Value>().await.unwrap()
        }
    };
    let init = rpc(f.claude.clone(), 1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "engram");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    let tools = rpc(f.claude.clone(), 2, "tools/list", json!({})).await;
    let mut names: Vec<_> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    assert_eq!(names, ["brief", "expand", "note", "search"]);

    let args = json!({"type": "decision", "title": "Token expiry", "claim": "Access tokens last 15 minutes", "rationale": "limit blast radius", "files": ["backend/auth.py"]});
    rpc(f.claude.clone(), 3, "tools/call", json!({"name": "note", "arguments": args})).await;
    let found = rpc(f.codex.clone(), 4, "tools/call", json!({"name": "search", "arguments": {"query": "token expiry"}})).await;
    let text = found["result"]["content"][0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("15 minutes"));

    let brief = rpc(f.codex.clone(), 5, "tools/call", json!({"name": "brief", "arguments": {"task": "change auth from session to token"}})).await;
    let brief_text = brief["result"]["content"][0]["text"].as_str().unwrap();
    assert!(brief_text.contains("Auth strategy") && brief_text.contains("What to know"));

    let id = text.split_whitespace().find_map(|w| w.trim_matches(|c| c == '(' || c == ',').strip_prefix("mem_").map(|_| w.trim_matches(|c| c == '(' || c == ','))).unwrap().to_string();
    let open = rpc(f.codex.clone(), 6, "tools/call", json!({"name": "expand", "arguments": {"id": id}})).await;
    assert!(open["result"]["content"][0]["text"].as_str().unwrap().contains("backend/auth.py"));

    let bad = rpc(f.codex.clone(), 7, "tools/call", json!({"name": "nope", "arguments": {}})).await;
    assert_eq!(bad["result"]["isError"], true);

    let (_, log) = get(&f, &f.admin, "/p/p1/api/requests").await;
    let who: Vec<String> = log.as_array().unwrap().iter().map(|r| format!("{}:{}", r["actor"].as_str().unwrap(), r["tool"].as_str().unwrap())).collect();
    assert!(who.contains(&"claude-code:note".to_string()) && who.contains(&"codex:search".to_string()));
}

#[tokio::test]
async fn projects_are_isolated_and_manageable() {
    let f = fixture().await;
    assert_eq!(get(&f, &f.claude, "/api/projects").await.0, 404, "agent tokens cannot list projects");
    let (_, list) = get(&f, &f.admin, "/api/projects").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["stats"]["memories"], 5);

    let (status, added) = post(&f, &f.admin, "/api/projects", json!({"root": "/tmp/other"})).await;
    assert_eq!(status, 200);
    let other = added["id"].as_str().unwrap().to_string();
    let (_, stats) = get(&f, &f.admin, &format!("/p/{other}/api/stats")).await;
    assert_eq!(stats["memories"], 0, "a new project starts empty");
    assert_eq!(get(&f, &f.claude, &format!("/p/{other}/api/stats")).await.0, 401, "tokens do not cross projects");

    let (_, connect) = post(&f, &f.admin, "/p/p1/api/connect", json!({"agent": "codex"})).await;
    assert!(connect["preview"].as_str().unwrap().contains("claude mcp add"));
    f.manager.remove(&other).unwrap();
    assert_eq!(get(&f, &f.admin, &format!("/p/{other}/api/stats")).await.0, 404);
}

#[tokio::test]
async fn ui_endpoints_return_graph_node_detail_and_timeline() {
    let f = fixture().await;
    let (_, graph) = get(&f, &f.admin, "/p/p1/api/graph").await;
    assert!(graph["nodes"].as_array().unwrap().len() >= 5 && !graph["edges"].as_array().unwrap().is_empty());
    let node = graph["nodes"].as_array().unwrap().iter().find(|n| n["label"] == "Auth strategy").unwrap();
    let (_, detail) = get(&f, &f.admin, &format!("/p/p1/api/node?id={}", node["id"].as_str().unwrap())).await;
    assert_eq!(detail["memory"]["title"], "Auth strategy");
    assert_eq!(detail["memory"]["type"], "decision");
}
