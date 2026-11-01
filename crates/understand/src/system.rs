//! The system map: areas (frontend, backend, crates, apps), the HTTP endpoints between them, and
//! the external services they use, linked to the files and functions on each side.
//!
//! Raw facts are stored per file at index time (`store_file`); the map is rebuilt from them after
//! each index pass (`rebuild_map`), so it never drifts from the code. Edges:
//! - `in_area`: file → area
//! - `serves`: handler (function or file) → endpoint
//! - `calls`: caller (function or file) → endpoint or service
//! - `uses`: file → service (through an SDK import, or config that sets it up)
//! - `mentions`: doc or comment → service it only names

use std::collections::{BTreeMap, HashSet};

use engram_code::areas::{Area, area_of, detect};
use engram_code::{Import, Net, Symbol};
use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::rusqlite::{OptionalExtension, params};
use engram_core::{Connection, Result};
use serde::Serialize;

/// Packages and modules that mean "this code talks to that service".
const SDKS: &[(&str, &str)] = &[
    ("ollama", "Ollama"), ("openai", "OpenAI"), ("anthropic", "Anthropic"), ("@anthropic-ai/sdk", "Anthropic"),
    ("stripe", "Stripe"), ("boto3", "AWS"), ("@aws-sdk", "AWS"), ("aws-sdk", "AWS"), ("googleapiclient", "Google APIs"),
    ("google", "Google APIs"), ("@google-cloud", "Google Cloud"), ("firebase", "Firebase"), ("firebase_admin", "Firebase"),
    ("@supabase", "Supabase"), ("supabase", "Supabase"), ("@auth0", "Auth0"), ("auth0", "Auth0"), ("redis", "Redis"),
    ("ioredis", "Redis"), ("psycopg", "PostgreSQL"), ("psycopg2", "PostgreSQL"), ("asyncpg", "PostgreSQL"), ("pg", "PostgreSQL"),
    ("tokio_postgres", "PostgreSQL"), ("pymongo", "MongoDB"), ("mongodb", "MongoDB"), ("twilio", "Twilio"), ("sendgrid", "SendGrid"),
    ("@sendgrid", "SendGrid"), ("resend", "Resend"), ("sentry_sdk", "Sentry"), ("@sentry", "Sentry"), ("posthog", "PostHog"),
];
/// Well-known local ports, for calls to `localhost:<port>` that no route in the project serves.
const PORTS: &[(&str, &str)] = &[("11434", "Ollama"), ("5432", "PostgreSQL"), ("6379", "Redis"), ("27017", "MongoDB"), ("9200", "Elasticsearch")];

/// Hosts of well-known APIs, matched as a suffix, so calls to them join the service's node.
const HOSTS: &[(&str, &str)] = &[
    ("gmail.googleapis.com", "Gmail"), ("oauth2.googleapis.com", "Google OAuth"), ("accounts.google.com", "Google OAuth"),
    ("googleapis.com", "Google APIs"), ("auth0.com", "Auth0"), ("api.stripe.com", "Stripe"), ("api.openai.com", "OpenAI"),
    ("api.anthropic.com", "Anthropic"), ("api.cloudflare.com", "Cloudflare"), ("supabase.co", "Supabase"), ("api.github.com", "GitHub"),
    ("api.twilio.com", "Twilio"), ("api.sendgrid.com", "SendGrid"), ("api.resend.com", "Resend"), ("infisical.com", "Infisical"),
];

fn host_service(host: &str) -> Option<&'static str> {
    let h = host.split(':').next().unwrap_or(host);
    HOSTS.iter().find(|(suffix, _)| h == *suffix || h.ends_with(&format!(".{suffix}"))).map(|(_, s)| *s)
}

fn sdk_service(import: &Import) -> Option<&'static str> {
    let name = match import {
        Import::Python { module, level: 0 } => module.split('.').next().unwrap_or(module).to_string(),
        Import::Js { specifier } if !specifier.starts_with('.') => {
            let mut parts = specifier.split('/');
            match parts.next() {
                Some(scope) if scope.starts_with('@') => scope.to_string(),
                Some(name) => name.to_string(),
                None => return None,
            }
        }
        Import::Rust { path, .. } => path.first()?.clone(),
        _ => return None,
    };
    SDKS.iter().find(|(pkg, _)| *pkg == name).map(|(_, service)| *service)
}

/// `settings.ollama_base_url` → "Ollama"; `env.STRIPE_API_URL` → "Stripe";
/// `cfg.meta_base_url.clone()` → "Meta". None when no part of it names a service (`self.url`).
fn service_from_setting(setting: &str) -> Option<String> {
    let last = setting
        .rsplit('.')
        .map(str::to_lowercase)
        .find(|s| ["url", "host", "domain", "endpoint", "base"].iter().any(|k| s.contains(k)))?;
    let mut core = last.as_str();
    for suffix in ["_base_url", "_api_url", "_url", "_host", "_domain", "_endpoint", "_api", "baseurl", "url"] {
        core = core.strip_suffix(suffix).unwrap_or(core);
    }
    let core = core.trim_matches('_');
    if core.is_empty() || ["api", "base", "server", "backend", "app"].contains(&core) {
        return None;
    }
    Some(SDKS.iter().find(|(pkg, _)| *pkg == core).map_or_else(
        || {
            let mut c = core.chars();
            c.next().map_or(core.to_string(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        },
        |(_, s)| s.to_string(),
    ))
}

/// Store what one file serves, calls and uses (replacing what was stored for it before).
pub fn store_file(conn: &Connection, file: &str, net: &Net, symbols: &[Symbol], imports: &[Import]) -> Result<()> {
    for table in ["net_routes", "net_calls", "net_services"] {
        conn.execute(&format!("DELETE FROM {table} WHERE file = ?"), [file])?;
    }
    // The innermost symbol spanning a line: the function that serves or calls.
    let enclosing = |line: usize, prefer_next: bool| -> Option<String> {
        let inside = symbols.iter().filter(|s| s.start_line <= line && line <= s.end_line).min_by_key(|s| s.end_line - s.start_line);
        // A decorator sits on the lines just above the function it decorates.
        let below = || symbols.iter().filter(|s| s.start_line > line && s.start_line <= line + 3).min_by_key(|s| s.start_line);
        if prefer_next { below().or(inside) } else { inside }.map(|s| s.name.clone())
    };
    for r in &net.routes {
        conn.execute(
            "INSERT INTO net_routes (file, symbol, method, path, prefix, line) VALUES (?, ?, ?, ?, ?, ?)",
            params![file, enclosing(r.line, true), r.method, r.path, r.prefix as i64, r.line as i64],
        )?;
    }
    for c in &net.calls {
        conn.execute(
            "INSERT INTO net_calls (file, symbol, method, path, host, line) VALUES (?, ?, ?, ?, ?, ?)",
            params![file, enclosing(c.line, false), c.method, c.path, c.host, c.line as i64],
        )?;
    }
    let mut seen = HashSet::new();
    for service in imports.iter().filter_map(sdk_service) {
        if seen.insert(service) {
            conn.execute("INSERT INTO net_services (file, service, via) VALUES (?, ?, 'sdk')", params![file, service])?;
        }
    }
    for base in &net.client_bases {
        if let Some(service) = service_from_setting(base) {
            conn.execute("INSERT INTO net_services (file, service, via) VALUES (?, ?, ?)", params![file, service, format!("client:{base}")])?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct MapStats {
    pub calls: crate::calls::CallStats,
    pub areas: usize,
    pub endpoints: usize,
    pub served: usize,
    pub calls_linked: usize,
    pub calls_unmatched: usize,
    pub services: usize,
}

struct RouteRow {
    file: String,
    symbol: Option<String>,
    method: String,
    path: String,
    prefix: bool,
}

fn methods_match(route: &str, call: &str) -> bool {
    route == call || route == "ANY" || call == "ANY"
}

fn segments(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

/// Same path, segment by segment, where `{}` on either side matches any one segment
/// (`/jobs/nope` reaches `/jobs/{}`). Returns how many segments matched literally.
fn path_match(route: &str, call: &str) -> Option<usize> {
    let (r, c): (Vec<&str>, Vec<&str>) = (route.split('/').collect(), call.split('/').collect());
    if r.len() != c.len() {
        return None;
    }
    let mut literal = 0;
    for (a, b) in r.iter().zip(&c) {
        if a == b {
            literal += 1;
        } else if *a != "{}" && *b != "{}" {
            return None;
        }
    }
    Some(literal)
}

fn path_ends_with(call: &str, route: &str) -> bool {
    let (c, r) = (call.split('/').collect::<Vec<_>>(), route.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>());
    c.len() >= r.len() && path_match(&format!("/{}", r.join("/")), &format!("/{}", c[c.len() - r.len()..].join("/"))).is_some()
}

/// The route a call reaches: exact path first, then the longest route the call's path ends with
/// (callers often add a prefix the server's router never sees, like `/p/{id}`), then a prefix route.
fn best_route<'a>(routes: &'a [RouteRow], method: &str, path: &str) -> Option<&'a RouteRow> {
    let compatible = || routes.iter().filter(|r| methods_match(&r.method, method));
    compatible()
        .filter(|r| !r.prefix)
        .filter_map(|r| path_match(&r.path, path).map(|lit| (lit, r)))
        .max_by_key(|(lit, _)| *lit)
        .map(|(_, r)| r)
        .or_else(|| compatible().filter(|r| !r.prefix && segments(&r.path) >= 2 && path_ends_with(path, &r.path)).max_by_key(|r| r.path.len()))
        .or_else(|| compatible().filter(|r| r.prefix && (path == r.path || path.starts_with(&format!("{}/", r.path)))).max_by_key(|r| r.path.len()))
}

fn is_local(host: &str) -> bool {
    let h = host.split(':').next().unwrap_or(host);
    ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"].contains(&h)
}

/// The entity for a function if indexed, else for its file.
fn actor_node(conn: &Connection, file: &str, symbol: Option<&str>) -> Result<String> {
    if let Some(s) = symbol {
        let found: Option<String> = conn
            .query_row("SELECT id FROM entities WHERE qualified_name = ? AND status = 'active' LIMIT 1", [format!("{file}::{s}")], |r| r.get(0))
            .optional()?;
        if let Some(id) = found {
            return Ok(id);
        }
    }
    Ok(upsert_entity(conn, "file", file, Some(file), None)?.id)
}

fn endpoint_name(method: &str, path: &str, prefix: bool) -> String {
    format!("{method} {path}{}", if prefix { "/*" } else { "" })
}

/// Rebuild areas, endpoints, services and their edges from the stored per-file facts.
/// `files` are the repository's files (to find manifests); `read` returns a file's text.
pub fn rebuild_map(conn: &Connection, repo_name: &str, files: &[String], read: impl Fn(&str) -> Option<String>) -> Result<MapStats> {
    let mut stats = MapStats { calls: crate::calls::link_calls(conn)?, ..Default::default() };
    let areas: Vec<Area> = detect(files, repo_name, &read);

    // Start from a clean slate for derived nodes and edges.
    conn.execute(
        "DELETE FROM edges WHERE type IN ('in_area','serves','calls','uses')
            OR src IN (SELECT id FROM entities WHERE kind IN ('area','endpoint','service'))
            OR dst IN (SELECT id FROM entities WHERE kind IN ('area','endpoint','service'))",
        [],
    )?;
    conn.execute("DELETE FROM entities WHERE kind IN ('area','endpoint','service')", [])?;
    conn.execute("DELETE FROM areas", [])?;

    let mut area_ids: BTreeMap<String, String> = BTreeMap::new();
    for a in &areas {
        conn.execute("INSERT INTO areas (path, name, kind, manifest) VALUES (?, ?, ?, ?)", params![a.path, a.name, a.kind, a.manifest])?;
        let id = upsert_entity(conn, "area", if a.path.is_empty() { "." } else { &a.path }, Some(&a.path), None)?.id;
        area_ids.insert(a.path.clone(), id);
        stats.areas += 1;
    }
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'file' AND status = 'active'")?;
    let file_rows: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (id, path) in &file_rows {
        if let Some(area) = area_of(&areas, path) {
            add_edge(conn, id, &area_ids[&area.path], "in_area", EdgeOpts { weight: Some(0.3), ..Default::default() })?;
        }
    }

    // Endpoints and their handlers.
    let mut stmt = conn.prepare("SELECT file, symbol, method, path, prefix FROM net_routes")?;
    let routes: Vec<RouteRow> = stmt
        .query_map([], |r| Ok(RouteRow { file: r.get(0)?, symbol: r.get(1)?, method: r.get(2)?, path: r.get(3)?, prefix: r.get::<_, i64>(4)? != 0 }))?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    for r in &routes {
        let endpoint = upsert_entity(conn, "endpoint", &endpoint_name(&r.method, &r.path, r.prefix), Some(&r.file), None)?;
        let handler = actor_node(conn, &r.file, r.symbol.as_deref())?;
        add_edge(conn, &handler, &endpoint.id, "serves", EdgeOpts { weight: Some(0.9), ..Default::default() })?;
        stats.served += 1;
    }
    stats.endpoints = routes.len();

    // Calls: to a local route, to an external service, or to an endpoint nothing here serves.
    let bases: BTreeMap<String, String> = {
        let mut stmt = conn.prepare("SELECT file, service FROM net_services WHERE via LIKE 'client:%'")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?
    };
    let mut stmt = conn.prepare("SELECT file, symbol, method, path, host FROM net_calls")?;
    let calls: Vec<(String, Option<String>, String, String, Option<String>)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (file, symbol, method, path, host) in &calls {
        let caller = actor_node(conn, file, symbol.as_deref())?;
        let local_route = match host {
            Some(h) if !is_local(h) => None,
            _ => best_route(&routes, method, path),
        };
        let target = match (local_route, host) {
            // The method is decided elsewhere (Swift sets it on the request): every route on the path.
            (Some(r), _) if method == "ANY" => {
                stats.calls_linked += 1;
                for other in routes.iter().filter(|o| o.path == r.path && o.prefix == r.prefix) {
                    let id = upsert_entity(conn, "endpoint", &endpoint_name(&other.method, &other.path, other.prefix), Some(&other.file), None)?.id;
                    let _ = add_edge(conn, &caller, &id, "calls", EdgeOpts { weight: Some(0.9), ..Default::default() });
                }
                continue;
            }
            (Some(r), _) => {
                stats.calls_linked += 1;
                upsert_entity(conn, "endpoint", &endpoint_name(&r.method, &r.path, r.prefix), Some(&r.file), None)?.id
            }
            (None, Some(h)) => {
                let port = h.rsplit(':').next().unwrap_or("");
                let name = PORTS
                    .iter()
                    .find(|(p, _)| is_local(h) && *p == port)
                    .map(|(_, s)| *s)
                    .or_else(|| host_service(h))
                    .map_or(h.clone(), str::to_string);
                upsert_entity(conn, "service", &name, None, None)?.id
            }
            (None, None) => match bases.get(file) {
                Some(service) => upsert_entity(conn, "service", service, None, None)?.id,
                // A file the app loads from its own assets (`/maps/us.json`), not an API.
                None if path.rsplit('/').next().is_some_and(|s| s.contains('.')) => continue,
                None => {
                    stats.calls_unmatched += 1;
                    upsert_entity(conn, "endpoint", &endpoint_name(method, path, false), None, None)?.id
                }
            },
        };
        let _ = add_edge(conn, &caller, &target, "calls", EdgeOpts { weight: Some(0.9), ..Default::default() });
    }

    // Services set up in config, scripts and env templates, or only named in docs (see infra.rs).
    // Stored with their line as evidence: `config:12:image postgres:17`.
    conn.execute("DELETE FROM net_services WHERE via LIKE 'config:%' OR via LIKE 'docs:%'", [])?;
    for f in crate::infra::scan(files, &read) {
        conn.execute("INSERT INTO net_services (file, service, via) VALUES (?, ?, ?)", params![f.file, f.service, format!("{}:{}:{}", f.how, f.line, f.why)])?;
    }

    let mut stmt = conn.prepare("SELECT DISTINCT file, service, via LIKE 'docs:%' FROM net_services")?;
    let uses: Vec<(String, String, bool)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (file, service, docs) in &uses {
        let file_id = upsert_entity(conn, "file", file, Some(file), None)?.id;
        let service_id = upsert_entity(conn, "service", service, None, None)?.id;
        let (kind, weight) = if *docs { ("mentions", 0.3) } else { ("uses", 0.7) };
        let _ = add_edge(conn, &file_id, &service_id, kind, EdgeOpts { weight: Some(weight), ..Default::default() });
    }
    stats.services = conn.query_row("SELECT COUNT(*) FROM entities WHERE kind = 'service'", [], |r| r.get::<_, i64>(0))? as usize;
    Ok(stats)
}
