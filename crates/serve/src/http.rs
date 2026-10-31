use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use engram_core::events::{Actor, ensure_actor};
use engram_core::{Connection, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::activity::{ActivityQuery, activity};
use crate::api::{documents, graph_data, node_detail, request_log, review_queue, stats, timeline};
use crate::mcp::{call_tool, handle_body, handle_body_with, shared_tool_definitions};
use crate::ops::{Ctx, OpResult, op_add_document, op_brief, op_expand, op_note, op_profile, op_search};
use crate::project::{ProjectManager, ProjectRef};
use crate::route::{pick_project, with_conn};
use engram_understand::quality::{set_archived, weak_memories};
use crate::tokens::verify_token;

struct AppState {
    manager: Arc<dyn ProjectManager>,
    admin_token: String,
}

pub struct RunningServer {
    pub url: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RunningServer {
    pub async fn close(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
}

/// Serve every project on one port. Agents connect once to `/mcp`, which places each call in a
/// project (see `route`); `/p/<project>/mcp` and `/p/<project>/api/...` address one project directly.
/// The UI uses the admin token, which also manages the project list and agent connections.
pub async fn start_server(manager: Arc<dyn ProjectManager>, admin_token: String, port: u16) -> std::io::Result<RunningServer> {
    let state = Arc::new(AppState { manager, admin_token });
    let app = Router::new().fallback(dispatch).with_state(state);
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let url = format!("http://{}", listener.local_addr()?);
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await;
    });
    Ok(RunningServer { url, shutdown: Some(tx), task })
}

fn reply(status: StatusCode, body: &Value) -> Response {
    let mut res = Response::new(Body::from(body.to_string()));
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    h.insert("access-control-allow-headers", HeaderValue::from_static("authorization, content-type, mcp-session-id, mcp-protocol-version"));
    h.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, DELETE, OPTIONS"));
    res
}

fn fail(status: StatusCode, message: &str) -> Response {
    reply(status, &json!({"error": message}))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").or_else(|| headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("bearer "))
}

enum Principal {
    Admin,
    Actor(Actor),
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).map(percent_decode)
}

/// `%XX` escapes and `+` as space, as browsers encode query strings.
fn percent_decode(v: &str) -> String {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (bytes[i], escaped) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (b, _) => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn dispatch(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    if parts.method == Method::OPTIONS {
        return reply(StatusCode::NO_CONTENT, &Value::Null);
    }
    let Some(token) = bearer(&parts.headers).map(String::from) else {
        return fail(StatusCode::UNAUTHORIZED, "missing or invalid token");
    };
    let is_admin = token == state.admin_token;
    let bytes = to_bytes(body, 10 * 1024 * 1024).await.unwrap_or_default();
    let json_body: Value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
    let path = parts.uri.path().to_string();
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let query = parts.uri.query();

    let result = match (&parts.method, segments.as_slice()) {
        (_, ["api", "health"]) => Ok(reply(StatusCode::OK, &json!({"ok": true}))),
        (m, ["api", "projects", rest @ ..]) if is_admin => handle_projects(state.clone(), m.clone(), rest, json_body).await,
        (m, ["api", "settings", rest @ ..]) if is_admin => {
            let (manager, method, test) = (state.manager.clone(), m.clone(), rest.first() == Some(&"test"));
            tokio::task::spawn_blocking(move || match (method, test) {
                (Method::GET, false) => manager.settings(),
                (Method::POST, false) => manager.update_settings(json_body),
                (Method::POST, true) => manager.test_settings(json_body["target"].as_str().unwrap_or("")),
                _ => Err(Error::msg("not found")),
            })
            .await
            .map_err(|e| Error::msg(e.to_string()))
            .and_then(|r| r)
            .map(|v| reply(StatusCode::OK, &v))
        }
        (&Method::POST, ["mcp"]) => {
            let st = state.clone();
            tokio::task::spawn_blocking(move || shared_mcp(&st, &token, is_admin, &json_body))
                .await
                .unwrap_or_else(|e| Err(Error::msg(e.to_string())))
        }
        (_, ["mcp"]) => Ok(fail(StatusCode::METHOD_NOT_ALLOWED, "use POST")),
        (&Method::POST, ["api", "connect"]) if is_admin => {
            let manager = state.manager.clone();
            tokio::task::spawn_blocking(move || connect(manager.as_ref(), &json_body))
                .await
                .unwrap_or_else(|e| Err(Error::msg(e.to_string())))
        }
        (_, ["p", id, tail @ ..]) => {
            let st = state.clone();
            let (id, tail, method) = (id.to_string(), tail.iter().map(|s| s.to_string()).collect::<Vec<_>>(), parts.method.clone());
            let query = query.map(String::from);
            tokio::task::spawn_blocking(move || project_route(&st, &token, is_admin, &id, &tail, &method, query.as_deref(), json_body))
                .await
                .unwrap_or_else(|e| Err(Error::msg(e.to_string())))
        }
        _ => Ok(fail(StatusCode::NOT_FOUND, "not found")),
    };
    result.unwrap_or_else(|e| fail(StatusCode::BAD_REQUEST, &e.to_string()))
}

#[derive(Deserialize)]
struct AddProject {
    root: PathBuf,
}

async fn handle_projects(state: Arc<AppState>, method: Method, rest: &[&str], body: Value) -> Result<Response> {
    let id = rest.first().map(|s| s.to_string());
    match (method, id) {
        (Method::GET, None) => {
            let manager = state.manager.clone();
            let list = tokio::task::spawn_blocking(move || manager.list().iter().map(project_summary).collect::<Vec<Value>>())
                .await
                .map_err(|e| Error::msg(e.to_string()))?;
            Ok(reply(StatusCode::OK, &Value::Array(list)))
        }
        (Method::POST, None) => {
            let req: AddProject = serde_json::from_value(body).map_err(|e| Error::msg(e.to_string()))?;
            let manager = state.manager.clone();
            let project = tokio::task::spawn_blocking(move || manager.add(&req.root)).await.map_err(|e| Error::msg(e.to_string()))??;
            Ok(reply(StatusCode::OK, &project_summary(&project)))
        }
        (Method::DELETE, Some(id)) => {
            let manager = state.manager.clone();
            tokio::task::spawn_blocking(move || manager.remove(&id)).await.map_err(|e| Error::msg(e.to_string()))??;
            Ok(reply(StatusCode::OK, &json!({"ok": true})))
        }
        _ => Ok(fail(StatusCode::NOT_FOUND, "not found")),
    }
}

fn project_summary(p: &ProjectRef) -> Value {
    let stats = match p.db_path.as_deref().map(engram_core::db::open) {
        Some(Ok(conn)) => stats(&conn).unwrap_or(Value::Null),
        _ => stats(&p.db.lock().unwrap()).unwrap_or(Value::Null),
    };
    json!({"id": p.info.id, "name": p.info.name, "root": p.info.root, "stats": stats})
}

fn parse<T: serde::de::DeserializeOwned>(body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|e| Error::msg(e.to_string()))
}

/// Admin, a token issued by this project, or an agent's shared-endpoint token.
fn authenticate(state: &AppState, conn: &Connection, token: &str, is_admin: bool) -> Result<Option<Principal>> {
    if is_admin {
        return Ok(Some(Principal::Admin));
    }
    if let Some(actor) = verify_token(conn, token)? {
        return Ok(Some(Principal::Actor(actor)));
    }
    match state.manager.agent_for_token(token) {
        Some(agent) => Ok(Some(Principal::Actor(ensure_actor(conn, "agent", &agent)?))),
        None => Ok(None),
    }
}

fn connect(manager: &dyn ProjectManager, body: &Value) -> Result<Response> {
    let outcome = manager.connect(body["agent"].as_str().unwrap_or(""), body["apply"].as_bool().unwrap_or(false))?;
    Ok(reply(StatusCode::OK, &serde_json::to_value(outcome)?))
}

/// The endpoint agents are connected to. Each tool call is placed in a project by the agent's
/// `cwd` or its latest activity, and the answer says which project it came from.
fn shared_mcp(state: &AppState, token: &str, is_admin: bool, body: &Value) -> Result<Response> {
    let (kind, agent) = match (is_admin, state.manager.agent_for_token(token)) {
        (true, _) => ("user", "engram-ui".to_string()),
        (false, Some(agent)) => ("agent", agent),
        (false, None) => return Ok(fail(StatusCode::UNAUTHORIZED, "missing or invalid token")),
    };
    let call = |name: &str, mut args: Value| -> Result<OpResult> {
        let cwd = args.as_object_mut().and_then(|o| o.remove("cwd")).and_then(|v| v.as_str().map(String::from));
        let project = pick_project(&state.manager.list(), &agent, cwd.as_deref())?;
        with_conn(&project, |conn| {
            let actor = ensure_actor(conn, kind, &agent)?;
            let out = call_tool(&Ctx { conn, actor: &actor, embedder: project.embedder.as_ref() }, name, args)?;
            Ok(OpResult { text: format!("[project: {} · {}]\n{}", project.info.name, project.info.root, out.text), data: out.data })
        })
    };
    Ok(match handle_body_with(&shared_tool_definitions(), &call, body) {
        Some(out) => reply(StatusCode::OK, &out),
        None => reply(StatusCode::ACCEPTED, &Value::Null),
    })
}

#[allow(clippy::too_many_arguments)]
fn project_route(
    state: &AppState,
    token: &str,
    is_admin: bool,
    id: &str,
    tail: &[String],
    method: &Method,
    query: Option<&str>,
    body: Value,
) -> Result<Response> {
    let Some(project) = state.manager.get(id) else { return Ok(fail(StatusCode::NOT_FOUND, "unknown project")) };
    let own = project.db_path.as_deref().map(engram_core::db::open).transpose()?;
    let shared = if own.is_none() { Some(project.db.lock().unwrap()) } else { None };
    let conn: &Connection = own.as_ref().or(shared.as_deref()).expect("a connection");
    let actor = match authenticate(state, conn, token, is_admin)? {
        Some(Principal::Admin) => ensure_actor(&conn, "user", "engram-ui")?,
        Some(Principal::Actor(a)) => a,
        None => return Ok(fail(StatusCode::UNAUTHORIZED, "missing or invalid token")),
    };
    let tail: Vec<&str> = tail.iter().map(String::as_str).collect();
    let limit = query_param(query, "limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    let ctx = Ctx { conn, actor: &actor, embedder: project.embedder.as_ref() };
    let op = |r: Result<crate::ops::OpResult>| r.map(|o| reply(StatusCode::OK, &json!({"text": o.text, "data": o.data})));
    let json_ok = |r: Result<Value>| r.map(|v| reply(StatusCode::OK, &v));

    match (method, tail.as_slice()) {
        (&Method::POST, ["mcp"]) => Ok(match handle_body(&ctx, &body) {
            Some(out) => reply(StatusCode::OK, &out),
            None => reply(StatusCode::ACCEPTED, &Value::Null),
        }),
        (_, ["mcp"]) => Ok(fail(StatusCode::METHOD_NOT_ALLOWED, "use POST")),
        (&Method::POST, ["api", "brief"]) => op(op_brief(&ctx, parse(if body.is_null() { json!({}) } else { body })?)),
        (&Method::POST, ["api", "search"]) => op(op_search(&ctx, parse(body)?)),
        (&Method::POST, ["api", "expand"]) => op(op_expand(&ctx, parse(body)?)),
        (&Method::POST, ["api", "note"]) => op(op_note(&ctx, parse(body)?)),
        (&Method::POST, ["api", "documents"]) => op(op_add_document(&ctx, parse(body)?)),
        (&Method::POST, ["api", "documents", "delete"]) => {
            engram_rag::document::delete_document(&conn, body["id"].as_str().unwrap_or(""))?;
            Ok(reply(StatusCode::OK, &json!({"ok": true})))
        }
        (&Method::GET, ["api", "documents"]) => json_ok(documents(&conn, limit)),
        (&Method::GET, ["api", "profile"]) => op(op_profile(&ctx)),
        (&Method::GET, ["api", "review"]) => json_ok(review_queue(&conn)),
        (&Method::POST, ["api", "review"]) => {
            let memory = engram_core::memory::review(&conn, body["id"].as_str().unwrap_or(""), body["action"].as_str().unwrap_or(""))?;
            Ok(reply(StatusCode::OK, &serde_json::to_value(memory)?))
        }
        (&Method::POST, ["api", "connect"]) if is_admin => connect(state.manager.as_ref(), &body),
        (&Method::GET, ["api", "weak"]) => json_ok(weak_memories(conn).and_then(|w| Ok(serde_json::to_value(w)?))),
        (&Method::POST, ["api", "memories", "archive"]) if is_admin => {
            let ids: Vec<String> = serde_json::from_value(body["ids"].clone()).unwrap_or_default();
            let changed = set_archived(conn, &ids, !body["restore"].as_bool().unwrap_or(false))?;
            Ok(reply(StatusCode::OK, &json!({"changed": changed})))
        }
        (&Method::GET, ["api", "stats"]) => json_ok(stats(&conn)),
        (&Method::GET, ["api", "graph"]) => json_ok(graph_data(&conn, query_param(query, "archived").as_deref() == Some("1"), 300)),
        (&Method::GET, ["api", "node"]) => json_ok(node_detail(&conn, &query_param(query, "id").unwrap_or_default())),
        (&Method::GET, ["api", "events"]) => json_ok(timeline(&conn, limit)),
        (&Method::GET, ["api", "activity"]) => {
            let q = ActivityQuery {
                before: query_param(query, "before"),
                days: query_param(query, "days").and_then(|d| d.parse().ok()).unwrap_or(3),
                system: query_param(query, "system").as_deref() == Some("1"),
            };
            json_ok(activity(conn, &project.info.root, &q))
        }
        (&Method::GET, ["api", "requests"]) => json_ok(request_log(&conn, limit)),
        _ => Ok(fail(StatusCode::NOT_FOUND, "not found")),
    }
}
