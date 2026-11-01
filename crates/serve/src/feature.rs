//! "How does auth work?" in one call: everything in the code that belongs to a concept, across
//! the frontend/backend boundary, so an agent knows which files to read or change without
//! searching the repository.
//!
//! The concept's words (with related ones: `auth` also means login, token, access) are matched
//! against file paths, function names, endpoint paths and services (by name or by what they are
//! for: Auth0 is the `auth` service). Matching endpoints bring their handlers and callers on both
//! sides; matching services bring the files that use them and the config that sets them up.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use engram_core::{Connection, Result};
use serde::Serialize;

/// Words that mean the same concept in code, and the service role they point to.
const CONCEPTS: &[(&[&str], Option<&str>)] = &[
    // Not `session` or `oauth`: a database session and a mail account's OAuth are other concerns.
    (&["auth", "authentication", "authenticate", "authorization", "login", "logout", "signin", "signup", "access", "token", "jwt", "security", "guard", "permission", "permissions"], Some("auth")),
    (&["email", "mail", "gmail", "inbox", "smtp", "notification", "notifications"], Some("mail")),
    (&["payment", "payments", "billing", "checkout", "invoice", "subscription", "stripe"], None),
    (&["database", "db", "data", "storage", "sql", "postgres", "postgresql", "migration", "migrations", "model", "models"], Some("data")),
    (&["secret", "secrets", "env", "settings", "config", "configuration"], Some("secrets")),
    (&["deploy", "deployment", "hosting", "host", "server", "infra", "infrastructure"], Some("host")),
    (&["llm", "ai", "ollama", "openai", "prompt", "prompts", "extract", "extraction"], Some("llm")),
    (&["proxy", "https", "tls", "caddy", "nginx"], Some("proxy")),
];

const STOP: &[&str] = &[
    "how", "does", "do", "work", "works", "the", "a", "an", "what", "where", "is", "are", "and", "or", "of", "in", "on", "for", "to",
    "with", "flow", "feature", "module", "code", "app", "our", "my", "this", "that", "which", "files", "file", "handled", "used",
];

/// Lowercase words of an identifier or path: `api-token-matcher.ts` → api, token, matcher, ts;
/// `getAuthenticatedUser` → get, authenticated, user; `EmailOAuthState` → email, o, auth, state.
fn tokens(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() { out.push(std::mem::take(&mut cur).to_lowercase()); }
            continue;
        }
        let boundary = c.is_uppercase() && !cur.is_empty() && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if boundary {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        cur.push(c);
    }
    if !cur.is_empty() { out.push(cur.to_lowercase()); }
    out
}

/// The query's words, with related words and the service roles they point to.
fn expand(query: &str) -> (BTreeSet<String>, BTreeSet<&'static str>) {
    let mut terms = BTreeSet::new();
    let mut roles = BTreeSet::new();
    for w in tokens(query) {
        if STOP.contains(&w.as_str()) || w.len() < 2 {
            continue;
        }
        for (words, role) in CONCEPTS {
            if words.contains(&w.as_str()) {
                terms.extend(words.iter().map(|s| s.to_string()));
                roles.extend(*role);
            }
        }
        terms.insert(w);
    }
    (terms, roles)
}

/// Whether text is about the query at all (any word of it, or a related one, appears).
pub fn mentions(query: &str, text: &str) -> bool {
    let (terms, _) = expand(query);
    tokens(text).iter().any(|t| hit(t, &terms))
}

fn hit(token: &str, terms: &BTreeSet<String>) -> bool {
    terms.iter().any(|t| token == t || (t.len() >= 4 && token.starts_with(t.as_str())))
}

fn is_test(path: &str) -> bool {
    path.contains("/tests/") || path.starts_with("tests/") || path.contains(".spec.") || path.contains(".test.") || path.rsplit('/').next().is_some_and(|f| f.starts_with("test_"))
}

fn is_code(path: &str) -> bool {
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    ["py", "ts", "tsx", "js", "jsx", "mjs", "rs", "go", "java", "kt", "rb", "php", "swift", "cs", "vue", "svelte"].contains(&ext)
}

#[derive(Debug, Default, Serialize)]
pub struct FileHit {
    pub file: String,
    pub area: Option<String>,
    /// Why it belongs: "uses Auth0", "serves GET /access/me", "defines verify_token".
    pub why: Vec<String>,
    /// Other matched files it imports.
    pub imports: Vec<String>,
    /// How many files outside the result import it: what a change would ripple into.
    pub used_by: usize,
    pub used_by_sample: Vec<String>,
    score: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct EndpointHit {
    pub name: String,
    pub handlers: Vec<String>,
    pub callers: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Feature {
    pub query: String,
    /// What matched: "service Auth0 (auth)", "endpoints /access/*", "names like auth, token".
    pub matched: Vec<String>,
    /// Code files, best first, tests left out.
    pub files: Vec<FileHit>,
    pub tests: Vec<String>,
    pub endpoints: Vec<EndpointHit>,
    /// (service, "auth for backend, frontend")
    pub services: Vec<(String, String)>,
    /// Config that sets the services up: "AUTH0_DOMAIN: backend/.env.example:7, deploy/.env.example:11".
    pub config: Vec<String>,
}

type Rows<T> = engram_core::rusqlite::Result<Vec<T>>;

fn rows2(conn: &Connection, sql: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Rows<_>>()?;
    Ok(rows)
}

fn file_of(qualified: &str) -> &str {
    qualified.split("::").next().unwrap_or(qualified)
}

/// Gather everything that belongs to `query`. Empty when nothing in the code matches.
pub fn feature(conn: &Connection, query: &str) -> Result<Feature> {
    let (terms, roles) = expand(query);
    let mut f = Feature { query: query.to_string(), ..Default::default() };
    if terms.is_empty() {
        return Ok(f);
    }
    let areas = rows2(conn, "SELECT path, name FROM areas WHERE path <> '' ORDER BY length(path) DESC")?;
    let area_of = |file: &str| areas.iter().find(|(p, _)| file.starts_with(&format!("{p}/"))).map(|(_, n)| n.clone());

    let mut hits: BTreeMap<String, FileHit> = BTreeMap::new();
    let mut tests = BTreeSet::new();
    let mut add = |file: &str, why: String, score: u32| {
        if is_test(file) {
            // A test is listed when it is named for the concept or exercises its endpoints or
            // services, not because one helper inside it has a matching name.
            if score > 1 {
                tests.insert(file.to_string());
            }
            return;
        }
        let h = hits.entry(file.to_string()).or_insert_with(|| FileHit { file: file.to_string(), area: area_of(file), ..Default::default() });
        if !h.why.contains(&why) {
            h.why.push(why);
            h.score += score;
        }
    };

    // Files and functions named for the concept.
    let mut named_words = BTreeSet::new();
    for (_, path) in rows2(conn, "SELECT id, qualified_name FROM entities WHERE kind = 'file' AND status = 'active'")? {
        if !is_code(&path) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(&path);
        let words: Vec<String> = path.split('/').flat_map(tokens).filter(|t| hit(t, &terms)).collect();
        if !words.is_empty() {
            named_words.extend(words);
            add(&path, format!("named {name}"), 2);
        }
    }
    for (_, q) in rows2(conn, "SELECT id, qualified_name FROM entities WHERE kind IN ('function','method','class','interface','type') AND status = 'active'")? {
        let symbol = q.split("::").nth(1).unwrap_or("");
        let words: Vec<String> = tokens(symbol).into_iter().filter(|t| hit(t, &terms)).collect();
        if !words.is_empty() && is_code(file_of(&q)) {
            named_words.extend(words);
            add(file_of(&q), format!("defines {symbol}"), 1);
        }
    }
    if !named_words.is_empty() {
        f.matched.push(format!("names like {}", named_words.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
    }

    // Endpoints whose path is about the concept, with their handlers and callers on both sides.
    let endpoints = rows2(conn, "SELECT id, qualified_name FROM entities WHERE kind = 'endpoint' ORDER BY substr(qualified_name, instr(qualified_name, ' ') + 1)")?;
    let edges_to = |id: &str, kinds: &str| -> Result<Vec<String>> {
        let sql = format!("SELECT DISTINCT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type IN ({kinds}) AND e.valid_to IS NULL");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<Rows<_>>()?;
        Ok(rows)
    };
    let mut prefixes = BTreeSet::new();
    for (id, name) in &endpoints {
        let path = name.split_once(' ').map_or(name.as_str(), |(_, p)| p);
        if !path.split('/').flat_map(tokens).any(|t| hit(&t, &terms)) {
            continue;
        }
        prefixes.insert(path.split('/').find(|s| !s.is_empty()).unwrap_or("").to_string());
        let handlers = edges_to(id, "'serves'")?;
        let callers = edges_to(id, "'calls'")?;
        for h in &handlers {
            add(file_of(h), format!("serves {name}"), 3);
        }
        for c in &callers {
            add(file_of(c), format!("calls {name}"), 3);
        }
        f.endpoints.push(EndpointHit {
            name: name.clone(),
            handlers: handlers.clone(),
            callers: callers.into_iter().filter(|c| !is_test(c)).collect(),
        });
    }
    if !prefixes.is_empty() {
        f.matched.push(format!("endpoints {}", prefixes.iter().map(|p| format!("/{p}/*")).collect::<Vec<_>>().join(", ")));
    }

    // Services named, or doing what the concept is about (Auth0 for auth), with what uses them.
    let links = {
        let mut stmt = conn.prepare("SELECT l.service, l.role, COALESCE(a.name, l.area) FROM system_links l LEFT JOIN areas a ON a.path = l.area ORDER BY 1, 3")?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Rows<_>>()?
    };
    let services = rows2(conn, "SELECT id, qualified_name FROM entities WHERE kind = 'service' ORDER BY qualified_name")?;
    for (id, name) in &services {
        // By its whole name (`auth0`, `postgres` for PostgreSQL), not a word inside it: Google
        // OAuth links mail accounts, it is not how users sign in.
        let whole = name.to_lowercase().replace([' ', '.'], "");
        let by_name = terms.iter().any(|t| whole == *t || (t.len() >= 4 && whole.starts_with(t.as_str())));
        let mine: Vec<&(String, String, String)> = links.iter().filter(|(s, _, _)| s == name).collect();
        let by_role = mine.iter().any(|(_, r, _)| roles.contains(r.as_str()));
        if !by_name && !by_role {
            continue;
        }
        let mut role_parts: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (_, role, area) in &mine {
            role_parts.entry(role.as_str()).or_default().push(area.as_str());
        }
        let how = role_parts.iter().map(|(r, a)| format!("{r} for {}", a.join(", "))).collect::<Vec<_>>().join("; ");
        f.services.push((name.clone(), how));
        f.matched.push(format!("service {name}"));
        for user in edges_to(id, "'uses','calls'")? {
            if is_code(file_of(&user)) {
                add(file_of(&user), format!("uses {name}"), 3);
            }
        }
        // Where it is set up, by setting: "AUTH0_DOMAIN: backend/.env.example:7, deploy/.env.example:11".
        let mut stmt = conn.prepare("SELECT file, via FROM net_services WHERE service = ? AND via LIKE 'config:%' ORDER BY file")?;
        let rows = stmt.query_map([name], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Rows<_>>()?;
        let mut by_setting: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (file, via) in rows {
            let mut parts = via.splitn(3, ':');
            let (_, line, why) = (parts.next(), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            if is_test(&file) || (is_code(&file) && !why.contains("URL")) {
                continue;
            }
            let setting = why.strip_prefix("env ").unwrap_or(why).to_string();
            by_setting.entry(setting).or_default().push(format!("{file}:{line}"));
        }
        for (setting, places) in by_setting {
            f.config.push(format!("{name} {setting}: {}", places.join(", ")));
        }
    }

    // How the matched files connect: which of them import each other, and how far a change ripples.
    let imports = rows2(
        conn,
        "SELECT s.qualified_name, d.qualified_name FROM edges e JOIN entities s ON s.id = e.src JOIN entities d ON d.id = e.dst
          WHERE e.type = 'imports' AND s.kind = 'file' AND d.kind = 'file' AND e.valid_to IS NULL",
    )?;
    let mut importers: HashMap<&str, Vec<&str>> = HashMap::new();
    for (src, dst) in &imports {
        importers.entry(dst.as_str()).or_default().push(src.as_str());
    }
    let matched: BTreeSet<String> = hits.keys().cloned().collect();
    for h in hits.values_mut() {
        h.imports = imports.iter().filter(|(s, d)| *s == h.file && matched.contains(d)).map(|(_, d)| d.clone()).collect();
        let outside: Vec<&str> = importers.get(h.file.as_str()).into_iter().flatten().copied().filter(|s| !matched.contains(*s) && !is_test(s)).collect();
        h.used_by = outside.len();
        h.used_by_sample = outside.iter().take(3).map(|s| s.to_string()).collect();
    }
    let mut files: Vec<FileHit> = hits.into_values().collect();
    files.sort_by(|a, b| b.score.cmp(&a.score).then(a.file.cmp(&b.file)));
    f.files = files;
    f.tests = tests.into_iter().collect();
    Ok(f)
}

fn short(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Agent-facing text, grouped by part, at most `limit` files per part.
pub fn feature_text(f: &Feature, limit: usize) -> String {
    if f.files.is_empty() && f.endpoints.is_empty() && f.services.is_empty() {
        return format!("Nothing in the code matches `{}`. Try another word for it, or `search` for docs and memories.", f.query);
    }
    let mut lines = vec![format!("`{}` in the code (matched {}):", f.query, f.matched.join("; "))];
    let mut by_area: BTreeMap<String, Vec<&FileHit>> = BTreeMap::new();
    for h in &f.files {
        by_area.entry(h.area.clone().unwrap_or_else(|| "other".into())).or_default().push(h);
    }
    let mut weak = vec![];
    for (area, files) in &by_area {
        // A file that only defines one matching name (`access_token` in a mail client) is listed
        // briefly at the end; the rest is what the concept is made of.
        let (strong, faint): (Vec<&&FileHit>, Vec<&&FileHit>) = files.iter().partition(|h| h.score > 1);
        weak.extend(faint.iter().map(|h| h.file.clone()));
        if strong.is_empty() {
            continue;
        }
        lines.push(format!("{area} ({} files):", strong.len()));
        for h in strong.iter().take(limit) {
            // Endpoints are listed once below; per file, say how many. The file name says itself.
            let (eps, rest): (Vec<&String>, Vec<&String>) = h.why.iter().partition(|w| w.starts_with("serves ") || w.starts_with("calls "));
            let defines: Vec<&str> = rest.iter().filter_map(|w| w.strip_prefix("defines ")).take(3).collect();
            let mut why: Vec<String> = rest.iter().filter(|w| !w.starts_with("named ") && !w.starts_with("defines ")).map(|w| w.to_string()).collect();
            if !defines.is_empty() {
                why.push(format!("defines {}", defines.join(", ")));
            }
            let served = eps.iter().filter(|w| w.starts_with("serves ")).count();
            let called = eps.len() - served;
            if served > 0 { why.push(format!("serves {served} endpoint(s)")); }
            if called > 0 { why.push(format!("calls {called} endpoint(s)")); }
            if !h.imports.is_empty() {
                why.push(format!("imports {}", h.imports.iter().map(|i| short(i)).collect::<Vec<_>>().join(", ")));
            }
            if h.used_by > 0 {
                why.push(format!("used by {} other file(s): {}{}", h.used_by, h.used_by_sample.iter().map(|s| short(s)).collect::<Vec<_>>().join(", "), if h.used_by > 3 { ", …" } else { "" }));
            }
            lines.push(if why.is_empty() { format!("  • {}", h.file) } else { format!("  • {} — {}", h.file, why.join("; ")) });
        }
        if strong.len() > limit {
            lines.push(format!("  … {} more", strong.len() - limit));
        }
    }
    if !weak.is_empty() {
        lines.push(format!("Also defines a matching name: {}", weak.join(", ")));
    }
    if !f.endpoints.is_empty() {
        lines.push("Endpoints (handler ← callers):".into());
        for e in &f.endpoints {
            let side = |xs: &[String]| if xs.is_empty() { "none here".to_string() } else { xs.iter().map(|x| short(x).to_string()).collect::<Vec<_>>().join(", ") };
            lines.push(format!("  • {} — {} ← {}", e.name, side(&e.handlers), side(&e.callers)));
        }
    }
    if !f.services.is_empty() {
        lines.push(format!("Services: {}", f.services.iter().map(|(s, how)| if how.is_empty() { s.clone() } else { format!("{s} ({how})") }).collect::<Vec<_>>().join(", ")));
    }
    if !f.config.is_empty() {
        lines.push("Config:".into());
        lines.extend(f.config.iter().take(8).map(|c| format!("  • {c}")));
    }
    if !f.tests.is_empty() {
        lines.push(format!("Tests: {}", f.tests.iter().take(8).map(String::as_str).collect::<Vec<_>>().join(", ")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_split_into_words() {
        assert_eq!(tokens("api-token-matcher.ts"), ["api", "token", "matcher", "ts"]);
        assert_eq!(tokens("getAuthenticatedUser"), ["get", "authenticated", "user"]);
        assert_eq!(tokens("EmailOAuthState"), ["email", "o", "auth", "state"]);
    }

    #[test]
    fn a_concept_brings_its_related_words_and_role() {
        let (terms, roles) = expand("how does auth work?");
        assert!(terms.contains("login") && terms.contains("token") && !terms.contains("how"));
        assert_eq!(roles.into_iter().collect::<Vec<_>>(), ["auth"]);
    }
}
