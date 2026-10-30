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

use crate::api::{graph_data, node_detail, request_log, stats, timeline};
use crate::mcp::handle_body;
use crate::ops::{op_brief, op_expand, op_note, op_search};
use crate::project::{ProjectManager, ProjectRef};
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

/// Serve every project on one port. Agents use `/p/<project>/mcp` or `/p/<project>/api/...` with a
/// per-agent token; the UI uses the admin token, which also manages the project list.
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
    query?.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).map(|v| v.replace("%2F", "/").replace("%20", " "))
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
            let list: Vec<Value> = state.manager.list().iter().map(project_summary).collect();
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
    let db = p.db.lock().unwrap();
    json!({"id": p.info.id, "name": p.info.name, "root": p.info.root, "stats": stats(&db).unwrap_or(Value::Null)})
}

fn parse<T: serde::de::DeserializeOwned>(body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|e| Error::msg(e.to_string()))
}

fn authenticate(conn: &Connection, token: &str, is_admin: bool) -> Result<Option<Principal>> {
    if is_admin {
        return Ok(Some(Principal::Admin));
    }
    Ok(verify_token(conn, token)?.map(Principal::Actor))
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
    let conn = project.db.lock().unwrap();
    let actor = match authenticate(&conn, token, is_admin)? {
        Some(Principal::Admin) => ensure_actor(&conn, "user", "engram-ui")?,
        Some(Principal::Actor(a)) => a,
        None => return Ok(fail(StatusCode::UNAUTHORIZED, "missing or invalid token")),
    };
    let tail: Vec<&str> = tail.iter().map(String::as_str).collect();
    let limit = query_param(query, "limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    let op = |r: Result<crate::ops::OpResult>| r.map(|o| reply(StatusCode::OK, &json!({"text": o.text, "data": o.data})));
    let json_ok = |r: Result<Value>| r.map(|v| reply(StatusCode::OK, &v));

    match (method, tail.as_slice()) {
        (&Method::POST, ["mcp"]) => Ok(match handle_body(&conn, &actor, &body) {
            Some(out) => reply(StatusCode::OK, &out),
            None => reply(StatusCode::ACCEPTED, &Value::Null),
        }),
        (_, ["mcp"]) => Ok(fail(StatusCode::METHOD_NOT_ALLOWED, "use POST")),
        (&Method::POST, ["api", "brief"]) => op(op_brief(&conn, &actor, parse(if body.is_null() { json!({}) } else { body })?)),
        (&Method::POST, ["api", "search"]) => op(op_search(&conn, &actor, parse(body)?)),
        (&Method::POST, ["api", "expand"]) => op(op_expand(&conn, &actor, parse(body)?)),
        (&Method::POST, ["api", "note"]) => op(op_note(&conn, &actor, parse(body)?)),
        (&Method::POST, ["api", "connect"]) => {
            drop(conn);
            let agent = body["agent"].as_str().unwrap_or("");
            let outcome = state.manager.connect(id, agent, body["apply"].as_bool().unwrap_or(false))?;
            Ok(reply(StatusCode::OK, &serde_json::to_value(outcome)?))
        }
        (&Method::GET, ["api", "stats"]) => json_ok(stats(&conn)),
        (&Method::GET, ["api", "graph"]) => json_ok(graph_data(&conn, query_param(query, "archived").as_deref() == Some("1"), 300)),
        (&Method::GET, ["api", "node"]) => json_ok(node_detail(&conn, &query_param(query, "id").unwrap_or_default())),
        (&Method::GET, ["api", "events"]) => json_ok(timeline(&conn, limit)),
        (&Method::GET, ["api", "requests"]) => json_ok(request_log(&conn, limit)),
        _ => Ok(fail(StatusCode::NOT_FOUND, "not found")),
    }
}
