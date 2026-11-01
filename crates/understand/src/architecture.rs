//! The architecture in one place: which part of the system (frontend, backend) relies on which
//! service, and for what (hosting, data, auth, secrets), worked out once from the code so agents
//! do not have to rediscover it.
//!
//! - Code and config roll up from files to their part: `security.py` uses Auth0, so the backend does.
//! - A compose file deploys the parts it builds; its other services (Postgres) run alongside them,
//!   and the scripts and docs next to it are about those parts.
//! - Docs say where a part is hosted ("Cloudflare Pages builds `frontend/`"); a sentence counts
//!   for the one part it names, or for the deploy folder it sits in.
//!
//! Each part's relations are also kept as `fact` memories (scope `system:<area>:<role>`). When the
//! code changes (Oracle to AWS), the memory is superseded, so the move stays as history. Memories
//! people or agents wrote that the code now contradicts are flagged in `memory_flags`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use engram_code::areas::{Area, area_of};
use engram_core::memory::{NewMemory, create_memory, set_status, supersede};
use engram_core::rusqlite::{OptionalExtension, params};
use engram_core::{Connection, Result};
use regex::Regex;
use serde::Serialize;

use crate::infra::{Finding, named_services};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// The part's folder (`backend`).
    pub area: String,
    pub service: String,
    pub role: &'static str,
    /// Strongest evidence: `code`, `config`, `compose` (runs alongside the part) or `docs`.
    pub how: &'static str,
    /// "file:line why", strongest first.
    pub evidence: Vec<String>,
}

/// What a service is for, when a part relies on it.
pub fn role_of(service: &str) -> &'static str {
    match service {
        "Oracle" | "AWS" | "Google Cloud" | "Cloudflare" | "Vercel" | "Netlify" | "Fly.io" | "Heroku" | "Render" | "Railway" => "host",
        "PostgreSQL" | "MySQL" | "MongoDB" | "Redis" | "Elasticsearch" | "MinIO" | "Supabase" | "Firebase" => "data",
        "Auth0" | "Okta" | "Clerk" => "auth",
        "Infisical" => "secrets",
        "Ollama" | "OpenAI" | "Anthropic" => "llm",
        "Gmail" | "SendGrid" | "Resend" | "Twilio" => "mail",
        "Caddy" | "Nginx" | "Traefik" => "proxy",
        _ => "uses",
    }
}

/// How a role reads in a sentence: "backend is hosted on Oracle".
pub fn phrase(role: &str) -> &'static str {
    match role {
        "host" => "is hosted on",
        "data" => "stores data in",
        "auth" => "authenticates users with",
        "secrets" => "gets its secrets from",
        "llm" => "runs language models on",
        "mail" => "sends and reads mail through",
        "proxy" => "is served through",
        _ => "uses",
    }
}

fn title(role: &str) -> &'static str {
    match role {
        "host" => "hosting",
        "data" => "data store",
        "auth" => "authentication",
        "secrets" => "secrets",
        "llm" => "language models",
        "mail" => "mail",
        "proxy" => "proxy",
        _ => "services",
    }
}

fn rank(how: &str) -> u8 {
    match how {
        "code" => 0,
        "config" => 1,
        "compose" => 2,
        _ => 3,
    }
}

/// Evidence that a service runs in the same compose file as the part (`deploy/docker-compose.yml:31 (image postgres:17)`).
fn is_alongside(evidence: &str) -> bool {
    evidence.contains(" (image ")
}

fn alongside(l: &Link) -> bool {
    l.how == "compose" || l.evidence.iter().any(|e| is_alongside(e))
}

fn is_test(path: &str) -> bool {
    path.contains("/tests/") || path.starts_with("tests/") || path.contains("/test_") || path.contains(".spec.") || path.contains(".test.")
}

fn is_compose(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    (name.starts_with("docker-compose") || name.starts_with("compose")) && (name.ends_with(".yml") || name.ends_with(".yaml"))
}

static BUILD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*(?:build|context):\s*["']?([^"'\s#]+)"#).unwrap());
static DEFAULT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{\w+:?-([^}]*)\}").unwrap());

/// The parts a compose file builds (`build: ./backend`, `context: ${BACKEND_DIR:-./backend}`).
fn compose_builds(dir: &str, text: &str, areas: &[Area]) -> Vec<String> {
    let mut built = BTreeSet::new();
    for line in text.lines() {
        let Some(c) = BUILD.captures(line) else { continue };
        let raw = DEFAULT.replace_all(&c[1], "$1").to_string();
        if raw.is_empty() || raw == "." && dir.is_empty() {
            continue;
        }
        // Relative to the compose file; when that is not a part, match the folder name (the
        // path usually comes from a variable the deploy script sets).
        let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
        for seg in raw.split('/') {
            match seg {
                "" | "." => {}
                ".." => { parts.pop(); }
                s => parts.push(s),
            }
        }
        let joined = parts.join("/");
        let last = raw.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        if let Some(a) = areas.iter().find(|a| !a.path.is_empty() && a.path == joined) {
            built.insert(a.path.clone());
        } else if let Some(a) = areas.iter().find(|a| !a.path.is_empty() && a.path.rsplit('/').next() == Some(last)) {
            built.insert(a.path.clone());
        }
    }
    built.into_iter().collect()
}

struct Parts<'a> {
    areas: &'a [Area],
    /// Deploy folders (where a compose file is) and the parts they deploy.
    units: Vec<(String, Vec<String>)>,
    names: Vec<(Regex, String)>,
}

impl Parts<'_> {
    /// The parts a file belongs to: its own folder's part, else the parts its deploy folder deploys.
    fn of(&self, file: &str) -> Vec<String> {
        if let Some(a) = area_of(self.areas, file).filter(|a| !a.path.is_empty()) {
            return vec![a.path.clone()];
        }
        let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
        self.units
            .iter()
            .filter(|(unit, _)| dir == unit || dir.starts_with(&format!("{unit}/")))
            .flat_map(|(_, parts)| parts.clone())
            .collect()
    }

    /// The parts a sentence names (`frontend`, `/backend`, `backend/ml`).
    fn named(&self, text: &str) -> Vec<String> {
        self.names.iter().filter(|(re, _)| re.is_match(text)).map(|(_, p)| p.clone()).collect()
    }
}

fn parts<'a>(areas: &'a [Area], files: &[String], read: &impl Fn(&str) -> Option<String>) -> Parts<'a> {
    let units = files
        .iter()
        .filter(|f| is_compose(f))
        .filter_map(|f| {
            let dir = f.rsplit_once('/').map_or("", |(d, _)| d).to_string();
            let built = compose_builds(&dir, &read(f)?, areas);
            (!built.is_empty()).then_some((dir, built))
        })
        .collect();
    // Top-level parts by name ("the frontend"); nested ones only by their path.
    let names = areas
        .iter()
        .filter(|a| !a.path.is_empty())
        .map(|a| {
            let word = if a.path.contains('/') { regex::escape(&a.path) } else { regex::escape(&a.name) };
            (Regex::new(&format!(r"(?i)(?:^|[^\w-]){word}(?:$|[^\w-])")).unwrap(), a.path.clone())
        })
        .collect();
    Parts { areas, units, names }
}

/// Every part's relations to services. `code_uses` are (file, service, why) from imports and calls;
/// `findings` come from config, scripts and docs (see infra.rs).
pub fn derive(areas: &[Area], findings: &[Finding], code_uses: &[(String, String, String)], files: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<Link> {
    let p = parts(areas, files, &read);
    let mut links: BTreeMap<(String, String, &'static str), Link> = BTreeMap::new();
    let mut add = |area: &str, service: &str, role: &'static str, how: &'static str, evidence: String| {
        let link = links.entry((area.to_string(), service.to_string(), role)).or_insert_with(|| Link {
            area: area.to_string(), service: service.to_string(), role, how, evidence: vec![],
        });
        if rank(how) < rank(link.how) {
            link.how = how;
            link.evidence.insert(0, evidence);
        } else if (link.evidence.len() < 4 || is_alongside(&evidence)) && !link.evidence.contains(&evidence) {
            link.evidence.push(evidence);
        }
    };

    for (file, service, why) in code_uses {
        if is_test(file) {
            continue;
        }
        // Calling a cloud's API (an S3 bucket) is using it, not being hosted on it.
        let role = match role_of(service) { "host" => "uses", r => r };
        for area in p.of(file) {
            add(&area, service, role, "code", format!("{file} ({why})"));
        }
    }
    for f in findings {
        if is_test(&f.file) {
            continue;
        }
        let evidence = format!("{}:{} ({})", f.file, f.line, f.why);
        let role = role_of(f.service);
        if f.how == "config" {
            // A platform file or a deploy CLI says where the part is hosted; an API key does not.
            let deploys = f.why.starts_with("file ") || f.why.starts_with("runs ") || f.why.starts_with("action ");
            let role = if role == "host" && !deploys { "uses" } else { role };
            let how = if is_compose(&f.file) && f.why.starts_with("image ") { "compose" } else { "config" };
            for area in p.of(&f.file) {
                add(&area, f.service, role, how, evidence.clone());
            }
            continue;
        }
        // Prose is only trusted for where things run and where their secrets and data live,
        // and only when the sentence is about one part.
        if !matches!(role, "host" | "secrets" | "proxy" | "data") {
            continue;
        }
        let named = p.named(&f.context);
        let about = match named.len() {
            0 => p.of(&f.file),
            1 => named,
            _ => continue,
        };
        if about.len() == 1 {
            add(&about[0], f.service, role, "docs", evidence);
        }
    }
    links.into_values().collect()
}

/// Replace the stored links.
pub fn store(conn: &Connection, links: &[Link]) -> Result<()> {
    conn.execute("DELETE FROM system_links", [])?;
    for l in links {
        conn.execute(
            "INSERT INTO system_links (area, service, role, how, evidence) VALUES (?, ?, ?, ?, ?)",
            params![l.area, l.service, l.role, l.how, serde_json::to_string(&l.evidence)?],
        )?;
    }
    Ok(())
}

fn join(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One sentence per part and role: "backend stores data in PostgreSQL (runs alongside it)".
fn claim(area_name: &str, role: &str, links: &[&Link]) -> String {
    let names: Vec<String> = links
        .iter()
        .map(|l| if alongside(l) { format!("{} (runs alongside it)", l.service) } else { l.service.clone() })
        .collect();
    format!("{area_name} {} {}.", phrase(role), join(&names))
}

/// Keep one `fact` memory per part and role in step with the links. A changed relation
/// supersedes the old memory (Oracle → AWS keeps Oracle as history); a gone one expires.
pub fn sync_memories(conn: &Connection, links: &[Link], areas: &[Area]) -> Result<SyncStats> {
    let mut stats = SyncStats::default();
    let mut groups: BTreeMap<(String, &str), Vec<&Link>> = BTreeMap::new();
    for l in links {
        groups.entry((l.area.clone(), l.role)).or_default().push(l);
    }
    let mut wanted = BTreeSet::new();
    for ((area, role), group) in &groups {
        let name = areas.iter().find(|a| a.path == *area).map_or(area.as_str(), |a| a.name.as_str());
        let scope = format!("system:{area}:{role}");
        wanted.insert(scope.clone());
        let text = claim(name, role, group);
        // The services themselves, kept in `summary`: only a change there is history worth keeping.
        let services = group.iter().map(|l| l.service.as_str()).collect::<Vec<_>>().join(", ");
        let current: Option<(String, String, Option<String>)> = conn
            .query_row("SELECT id, claim, summary FROM memories WHERE scope = ? AND status = 'current'", [&scope], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        match &current {
            Some((id, c, None)) if *c == text => {
                // Written before the services were kept: record them now, so a later move shows.
                conn.execute("UPDATE memories SET summary = ? WHERE id = ?", params![services, id])?;
                continue;
            }
            Some((_, c, _)) if *c == text => continue,
            Some((id, _, Some(was))) if *was == services => {
                // Same services, new wording or evidence: update in place.
                conn.execute("UPDATE memories SET claim = ?, updated_at = ? WHERE id = ?", params![text, engram_core::db::now(), id])?;
                continue;
            }
            _ => {}
        }
        let evidence: Vec<&String> = group.iter().flat_map(|l| l.evidence.iter().take(2)).collect();
        let mut touches = vec![];
        for e in &evidence {
            let file = e.split([':', ' ']).next().unwrap_or("");
            if let Some(id) = conn.query_row("SELECT id FROM entities WHERE kind = 'file' AND qualified_name = ?", [file], |r| r.get::<_, String>(0)).optional()? {
                if !touches.contains(&id) {
                    touches.push(id);
                }
            }
        }
        let strongest = group.iter().map(|l| rank(l.how)).min().unwrap_or(3);
        let memory = NewMemory {
            scope: Some(scope),
            summary: Some(services),
            rationale: Some(format!("Detected from the code: {}.", evidence.iter().map(|e| e.as_str()).collect::<Vec<_>>().join("; "))),
            confidence: Some(if strongest < 3 { 0.9 } else { 0.65 }),
            salience: Some(0.6),
            touches,
            ..NewMemory::new("fact", &format!("{name} {}", title(role)), &text)
        };
        match current {
            Some((id, _, _)) => { supersede(conn, &id, memory)?; stats.changed += 1; }
            None => { create_memory(conn, memory)?; stats.created += 1; }
        }
    }
    let mut stmt = conn.prepare("SELECT id, scope FROM memories WHERE scope LIKE 'system:%' AND status = 'current'")?;
    let current: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (id, scope) in current {
        if !wanted.contains(&scope) {
            set_status(conn, &id, "expired")?;
            stats.expired += 1;
        }
    }
    Ok(stats)
}

#[derive(Debug, Default, Serialize)]
pub struct SyncStats {
    pub created: usize,
    pub changed: usize,
    pub expired: usize,
    pub flagged: usize,
}

/// Flag memories (not the system's own) that say a part relies on a service for hosting, data,
/// auth or secrets when the code now says another one. Each clause is read on its own, so
/// "backend is hosted on Oracle, the frontend on Cloudflare" checks both halves.
pub fn flag_contradictions(conn: &Connection, links: &[Link], areas: &[Area]) -> Result<usize> {
    conn.execute("DELETE FROM memory_flags", [])?;
    let p = Parts { areas, units: vec![], names: parts(areas, &[], &|_: &str| None::<String>).names };
    let mut stmt = conn.prepare("SELECT id, title || '. ' || claim FROM memories WHERE status = 'current' AND COALESCE(scope, '') NOT LIKE 'system:%'")?;
    let memories: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut flagged = 0;
    for (id, text) in memories {
        let mut reasons = vec![];
        for clause in text.split(['.', ',', ';', '\n']) {
            let named = p.named(clause);
            let [area] = named.as_slice() else { continue };
            for service in named_services(clause) {
                let role = role_of(service);
                if !matches!(role, "host" | "data" | "auth" | "secrets") {
                    continue;
                }
                let now: Vec<&Link> = links.iter().filter(|l| l.area == *area && l.role == role).collect();
                if now.is_empty() || now.iter().any(|l| l.service == service) {
                    continue;
                }
                let names: Vec<String> = now.iter().map(|l| l.service.clone()).collect();
                let name = areas.iter().find(|a| a.path == *area).map_or(area.as_str(), |a| a.name.as_str());
                reasons.push(format!(
                    "says {name} {} {service}, but the code now says {} ({})",
                    phrase(role), join(&names), now[0].evidence.first().map_or("", String::as_str)
                ));
            }
        }
        if !reasons.is_empty() {
            conn.execute("INSERT INTO memory_flags (memory_id, reason, flagged_at) VALUES (?, ?, ?)", params![id, reasons.join("; "), engram_core::db::now()])?;
            flagged += 1;
        }
    }
    Ok(flagged)
}

/// Per part, its relations as one short line: "hosted on Oracle (per docs) · data PostgreSQL (same host) · auth Auth0".
pub fn summary(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let order = ["host", "data", "auth", "secrets", "llm", "mail", "proxy", "uses"];
    let mut stmt = conn.prepare("SELECT area, service, role, how, evidence FROM system_links")?;
    let rows: Vec<(String, String, String, String, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut by_area: BTreeMap<String, BTreeMap<usize, Vec<String>>> = BTreeMap::new();
    for (area, service, role, how, evidence) in rows {
        let i = order.iter().position(|r| *r == role).unwrap_or(order.len() - 1);
        let label = if how == "compose" || is_alongside(&evidence) { format!("{service} (same host)") } else if how == "docs" { format!("{service} (per docs)") } else { service };
        by_area.entry(area).or_default().entry(i).or_default().push(label);
    }
    Ok(by_area
        .into_iter()
        .map(|(area, roles)| {
            let text = roles
                .into_iter()
                .map(|(i, names)| match order[i] {
                    "host" => format!("hosted on {}", names.join(", ")),
                    role => format!("{role} {}", names.join(", ")),
                })
                .collect::<Vec<_>>()
                .join(" · ");
            (area, text)
        })
        .collect())
}

/// Memories the code contradicts: (id, title, reason).
pub fn flags(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare("SELECT f.memory_id, m.title, f.reason FROM memory_flags f JOIN memories m ON m.id = f.memory_id WHERE m.status = 'current' ORDER BY f.flagged_at")?;
    Ok(stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?)
}

/// How the architecture changed in the last `days` days, newest first:
/// "2026-11-03: backend is hosted on AWS. (was: backend is hosted on Oracle.)".
pub fn changes(conn: &Connection, days: i64) -> Result<Vec<String>> {
    let since = format!("-{days} days");
    let mut stmt = conn.prepare(
        "SELECT substr(new.valid_from, 1, 10), new.claim, old.claim FROM edges e
           JOIN memories new ON new.id = e.src JOIN memories old ON old.id = e.dst
          WHERE e.type = 'supersedes' AND new.scope LIKE 'system:%' AND old.summary IS NOT NULL AND new.summary <> old.summary
            AND new.valid_from >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)
         UNION ALL
         SELECT substr(valid_to, 1, 10), 'no longer: ' || claim, NULL FROM memories
          WHERE scope LIKE 'system:%' AND status = 'expired' AND valid_to >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)
         ORDER BY 1 DESC",
    )?;
    let rows = stmt.query_map([&since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?;
    let mut out = vec![];
    for row in rows {
        let (day, now, was) = row?;
        out.push(match was { Some(was) => format!("{day}: {now} (was: {was})"), None => format!("{day}: {now}") });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::scan;

    fn area(path: &str, kind: &str) -> Area {
        Area { path: path.into(), name: path.rsplit('/').next().unwrap().into(), kind: kind.into(), manifest: String::new() }
    }

    fn repo() -> Vec<(&'static str, &'static str)> {
        vec![
            ("deploy/docker-compose.yml", "services:\n  api:\n    build:\n      context: ${BACKEND_DIR:-./backend}\n    environment:\n      DATABASE_URL: postgresql://x@db/x\n  db:\n    image: postgres:17\n"),
            ("deploy/refresh-secrets.sh", "infisical export --path=/backend\n"),
            ("deploy/README.md", "Everything runs in Docker on the Oracle server.\nThe frontend deploys itself: Cloudflare Pages builds `frontend/`.\nSync `/frontend` to Cloudflare Pages using Infisical. Never sync `/backend` there.\n"),
            ("README.md", "| DNS | Cloudflare |\n"),
            ("backend/app/security.py", ""),
            ("frontend/src/auth.ts", ""),
        ]
    }

    fn links_for(files: &[(&str, &str)]) -> Vec<(String, String, &'static str, &'static str)> {
        let areas = vec![area("backend", "FastAPI service"), area("frontend", "Angular app")];
        let list: Vec<String> = files.iter().map(|(p, _)| p.to_string()).collect();
        let read = |p: &str| files.iter().find(|(f, _)| *f == p).map(|(_, t)| t.to_string());
        let findings = scan(&list, read);
        let code = vec![
            ("backend/app/security.py".into(), "Auth0".into(), "sdk".into()),
            ("frontend/src/auth.ts".into(), "Auth0".into(), "sdk".into()),
        ];
        derive(&areas, &findings, &code, &list, read).into_iter().map(|l| (l.area, l.service, l.role, l.how)).collect()
    }

    #[test]
    fn parts_get_their_hosts_data_auth_and_secrets() {
        let got = links_for(&repo());
        let want = vec![
            ("backend".into(), "Auth0".into(), "auth", "code"),
            ("backend".into(), "Infisical".into(), "secrets", "config"),
            ("backend".into(), "Oracle".into(), "host", "docs"),
            ("backend".into(), "PostgreSQL".into(), "data", "config"),
            ("frontend".into(), "Auth0".into(), "auth", "code"),
            ("frontend".into(), "Cloudflare".into(), "host", "docs"),
            ("frontend".into(), "Infisical".into(), "secrets", "docs"),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn a_move_supersedes_the_fact_and_flags_what_people_wrote() {
        let conn = engram_core::db::open_memory().unwrap();
        let areas = vec![area("backend", "FastAPI service"), area("frontend", "Angular app")];
        let link = |service: &str| Link { area: "backend".into(), service: service.into(), role: "host", how: "docs", evidence: vec!["deploy/README.md:1 (x)".into()] };
        let said = create_memory(&conn, NewMemory::new("fact", "Hosting", "The backend is hosted on oracle, the frontend on Cloudflare")).unwrap();

        sync_memories(&conn, &[link("Oracle")], &areas).unwrap();
        assert_eq!(flag_contradictions(&conn, &[link("Oracle")], &areas).unwrap(), 0);

        let stats = sync_memories(&conn, &[link("AWS")], &areas).unwrap();
        assert_eq!(stats.changed, 1);
        let claims: Vec<(String, String)> = conn
            .prepare("SELECT claim, status FROM memories WHERE scope = 'system:backend:host' ORDER BY version").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(claims, vec![
            ("backend is hosted on Oracle.".to_string(), "superseded".to_string()),
            ("backend is hosted on AWS.".to_string(), "current".to_string()),
        ]);
        assert_eq!(flag_contradictions(&conn, &[link("AWS")], &areas).unwrap(), 1);
        let reason: String = conn.query_row("SELECT reason FROM memory_flags WHERE memory_id = ?", [&said.id], |r| r.get(0)).unwrap();
        assert!(reason.contains("hosted on Oracle, but the code now says AWS"), "{reason}");

        let changed = changes(&conn, 30).unwrap();
        assert!(changed[0].ends_with("backend is hosted on AWS. (was: backend is hosted on Oracle.)"), "{changed:?}");
        assert_eq!(flags(&conn).unwrap()[0].0, said.id);

        assert_eq!(sync_memories(&conn, &[], &areas).unwrap().expired, 1);
    }
}
