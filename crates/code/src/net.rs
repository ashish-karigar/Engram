//! HTTP routes a file serves and HTTP calls it makes, read from source text. This is what connects
//! a frontend to a backend, a phone app to its receiver, a service to the APIs it uses: the edges an
//! import graph cannot see, and the ones that decide the impact of changing an API.
//!
//! Text patterns, not full type analysis: they cover the common frameworks (FastAPI, Flask,
//! http.server, Express, Next.js API routes, axum, actix, slice-pattern routers) and clients
//! (fetch, Angular HttpClient, axios, wrapper functions, requests/httpx/http.client, reqwest,
//! Swift URLs). Paths are normalized so `${this.base}/jobs/${id}`, `/jobs/{job_id}` and
//! `/jobs/:id` all read `/jobs/{}`.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// GET, POST, ... or ANY.
    pub method: String,
    /// Normalized, e.g. `/jobs/{}`.
    pub path: String,
    /// The route also serves everything below `path` (`rest @ ..`, `startswith`).
    pub prefix: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// GET, POST, ... or ANY when the method is decided elsewhere.
    pub method: String,
    pub path: String,
    /// Host for absolute URLs: `api.stripe.com`, `localhost:11434`, or `{auth0_domain}` when it
    /// comes from configuration. None for relative URLs and URLs built on a base variable.
    pub host: Option<String>,
    pub line: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Net {
    pub routes: Vec<Route>,
    pub calls: Vec<Call>,
    /// Settings a file's HTTP client is pointed at (`base_url=settings.ollama_base_url`,
    /// `axios.create({ baseURL: env.STRIPE_URL })`). Calls from that file that no local route serves
    /// go to the service named after the setting.
    pub client_bases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetLang {
    Python,
    Js,
    Rust,
    Swift,
}

impl NetLang {
    pub fn from_path(path: &str) -> Option<NetLang> {
        let ext = path.rsplit('.').next()?;
        match ext {
            "py" => Some(NetLang::Python),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(NetLang::Js),
            "rs" => Some(NetLang::Rust),
            "swift" => Some(NetLang::Swift),
            _ => None,
        }
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

fn line_of(source: &str, byte: usize) -> usize {
    source[..byte].bytes().filter(|b| *b == b'\n').count() + 1
}

/// Services a configured host is commonly for; used to name `https://{domain}/...` calls.
const KNOWN_SERVICES: &[&str] = &["Auth0", "Stripe", "Supabase", "Firebase", "Ollama", "OpenAI", "Anthropic", "Twilio", "SendGrid", "Okta", "Clerk"];

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];

/// Names that hold a server address rather than a path: left out of the path when unresolved.
fn is_base_name(name: &str) -> bool {
    let n = name.rsplit(['.', ':']).next().unwrap_or(name).to_lowercase();
    ["base", "url", "uri", "host", "origin", "prefix", "endpoint", "server", "address", "domain", "root", "api"]
        .iter()
        .any(|k| n == *k || n.ends_with(k) || n.starts_with(k))
}

/// Normalize a path: placeholders become `{}`, query and fragment dropped, slashes tidied.
pub fn normalize(path: &str) -> String {
    static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{[^}]*\}|\{[^}]*\}|<[^>]*>|:[A-Za-z_][A-Za-z0-9_]*|\\\([^)]*\)"));
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = PLACEHOLDER.replace_all(path, "{}");
    let mut out = String::from("/");
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if out.len() > 1 {
            out.push('/');
        }
        out.push_str(seg);
    }
    out.to_lowercase()
}

/// String-valued assignments in a file (`base = `${APP_CONFIG.apiUrl}/access``, `PREFIX = "/v1"`),
/// so calls built from them can be resolved. Last assignment wins; good enough within one file.
fn string_bindings(source: &str) -> HashMap<String, String> {
    static ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
        re(r#"(?m)(?:^|[\s(,;{])(?:const|let|var|readonly|private|public|protected|static|self\.)*\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*[A-Za-z<>\[\]| ]+)?=\s*(f?`[^`]*`|f?"[^"\n]*"|f?'[^'\n]*')"#)
    });
    let mut out = HashMap::new();
    for c in ASSIGN.captures_iter(source) {
        let value = c[2].trim_start_matches('f');
        out.insert(c[1].to_string(), value[1..value.len() - 1].to_string());
    }
    out
}

/// Resolve a string or template literal's interpolations against the file's bindings.
/// `${this.base}/jobs/${id}` -> `{access-base}/jobs/{}` -> `/access/jobs/{}` once `base` resolves.
fn resolve(text: &str, bindings: &HashMap<String, String>, depth: usize) -> String {
    resolve_with(text, bindings, depth, false)
}

/// `keep_bases`: an unresolved base name stays as `{name}` instead of vanishing, so a host that
/// comes from configuration (`https://{settings.auth0_domain}`) can still be named.
fn resolve_with(text: &str, bindings: &HashMap<String, String>, depth: usize, keep_bases: bool) -> String {
    static INTERP: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{\s*([^}]+?)\s*\}|\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}|\\\(\s*([^)]+?)\s*\)"));
    INTERP
        .replace_all(text, |c: &regex::Captures| {
            let expr = c.get(1).or(c.get(2)).or(c.get(3)).map_or("", |m| m.as_str());
            let name = expr.rsplit('.').next().unwrap_or(expr);
            match bindings.get(name).filter(|_| depth < 4) {
                Some(value) => resolve_with(value, bindings, depth + 1, keep_bases),
                None if is_base_name(expr) && keep_bases => format!("{{{name}}}"),
                None if is_base_name(expr) => String::new(),
                None => "{}".into(),
            }
        })
        .into_owned()
}

/// Split an absolute URL into host and path; relative URLs have no host.
fn split_url(url: &str) -> (Option<String>, String) {
    match url.split_once("://") {
        Some((_, rest)) => {
            let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
            let host = if host.is_empty() || host == "{}" { None } else { Some(host.to_lowercase()) };
            (host, format!("/{path}"))
        }
        None => (None, url.to_string()),
    }
}

/// A literal that reads like an API path once resolved: starts with `/`, or is an absolute URL.
fn api_like(url: &str) -> bool {
    let looks = url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://");
    let asset = [".css", ".js", ".png", ".jpg", ".svg", ".ico", ".html", ".woff", ".map"].iter().any(|e| url.split('?').next().unwrap_or("").ends_with(e));
    looks && !asset && url.len() > 1
}

fn push_call(net: &mut Net, method: &str, raw: &str, bindings: &HashMap<String, String>, line: usize) {
    let resolved = resolve(raw, bindings, 0);
    if !api_like(&resolved) && !raw.contains("://") {
        return;
    }
    let (host, path) = split_url(&resolved);
    let host = host.or_else(|| {
        // `https://{settings.auth0_domain}/...`, possibly through variables: the host is
        // configuration; name it after the setting.
        let kept = resolve_with(raw, bindings, 0, true);
        let after = kept.split_once("://")?.1;
        let h = after.split('/').next()?;
        h.contains('{').then(|| h.trim_matches(|c| c == '{' || c == '}' || c == '$').rsplit('.').next().unwrap_or(h).to_lowercase())
    });
    net.calls.push(Call { method: method.to_uppercase(), path: normalize(&path), host, line });
}

/// Comment lines blanked (same length, so byte offsets and line numbers stay right): examples in
/// comments are not routes. Only whole-line comments, so `"http://..."` inside code is untouched.
fn without_comments(lang: NetLang, source: &str) -> String {
    let markers: &[&str] = match lang {
        NetLang::Python => &["#"],
        _ => &["//", "/*", "*"],
    };
    source
        .split_inclusive('\n')
        .map(|line| {
            let t = line.trim_start();
            if markers.iter().any(|m| t.starts_with(m)) {
                line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect()
            } else {
                line.to_string()
            }
        })
        .collect()
}

/// Tests call endpoints (worth knowing for impact) but never serve them.
fn is_test_file(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("/tests/") || p.starts_with("tests/") || p.contains("/test/") || p.contains(".spec.") || p.contains(".test.")
        || p.rsplit('/').next().is_some_and(|f| f.starts_with("test_") || f.ends_with("_test.py") || f.ends_with("_test.go"))
}

pub fn extract(lang: NetLang, path: &str, source: &str) -> Net {
    let mut net = Net::default();
    let cleaned = without_comments(lang, source);
    let source = cleaned.as_str();
    let bindings = string_bindings(source);
    // `base_url=settings.x_url`, `baseURL: env.STRIPE_URL`, or `base_url=f"https://{settings.auth0_domain}"`.
    // A bare name (`base_url: String`) is a type or parameter, not a setting.
    static BASE: LazyLock<Regex> = LazyLock::new(|| re(r#"\b(?:base_url|baseURL|baseUrl)\s*[=:]\s*(?:f?["'`][a-z]+://\$?\{([A-Za-z_][\w.]*)\}|([A-Za-z_]\w*\.[A-Za-z_][\w.]*))"#));
    net.client_bases = BASE
        .captures_iter(source)
        .filter_map(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_string()))
        .filter(|b| b != "None" && b != "undefined")
        .collect();
    match lang {
        NetLang::Python => python(source, &bindings, &mut net),
        NetLang::Js => js(path, source, &bindings, &mut net),
        NetLang::Rust => rust(source, &bindings, &mut net),
        NetLang::Swift => swift(source, &bindings, &mut net),
    }
    if is_test_file(path) {
        net.routes.clear();
    }
    let route_lines: Vec<usize> = net.routes.iter().map(|r| r.line).collect();
    net.calls.retain(|c| !route_lines.contains(&c.line));
    // A configured host with a generic name (`https://{domain}/...`) is named after the one known
    // service the file talks about, when there is exactly one.
    let mentioned: Vec<&str> = KNOWN_SERVICES.iter().filter(|s| source.to_lowercase().contains(&s.to_lowercase())).copied().collect();
    for c in &mut net.calls {
        if let (Some(h), [only]) = (&c.host, mentioned.as_slice()) {
            if is_base_name(h) {
                c.host = Some(only.to_string());
            }
        }
    }
    net
}

fn python(source: &str, bindings: &HashMap<String, String>, net: &mut Net) {
    // FastAPI / Flask: `router = APIRouter(prefix="/jobs")`, `bp = Blueprint(..., url_prefix="/x")`.
    static ROUTER: LazyLock<Regex> = LazyLock::new(|| re(r#"([A-Za-z_]\w*)\s*=\s*(?:APIRouter|Blueprint)\(([^)]*)\)"#));
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r#"(?:url_)?prefix\s*=\s*["']([^"']*)["']"#));
    let prefixes: HashMap<String, String> = ROUTER
        .captures_iter(source)
        .map(|c| (c[1].to_string(), PREFIX.captures(&c[2]).map_or(String::new(), |p| p[1].to_string())))
        .collect();
    static DECORATOR: LazyLock<Regex> = LazyLock::new(|| re(r#"@([A-Za-z_]\w*)\.(get|post|put|patch|delete|head|options|route|websocket|api_route)\(\s*["']([^"']*)["']([^)\n]*)"#));
    static FLASK_METHODS: LazyLock<Regex> = LazyLock::new(|| re(r#"methods\s*=\s*\[([^\]]*)\]"#));
    for c in DECORATOR.captures_iter(source) {
        let prefix = prefixes.get(&c[1]).cloned().unwrap_or_default();
        let methods: Vec<String> = match &c[2] {
            "route" | "api_route" => FLASK_METHODS
                .captures(&c[4])
                .map(|m| m[1].split(',').map(|s| s.trim().trim_matches(['"', '\'']).to_uppercase()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_else(|| vec!["GET".into()]),
            "websocket" => vec!["WS".into()],
            m => vec![m.to_uppercase()],
        };
        let line = line_of(source, c.get(0).unwrap().start());
        for m in methods {
            net.routes.push(Route { method: m, path: normalize(&format!("{prefix}{}", &c[3])), prefix: false, line });
        }
    }
    // http.server: `def do_POST(self):` ... `if self.path == "/frame":` / `url.path != "/x"` / startswith.
    static HANDLER: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\s*def do_([A-Z]+)\(self"));
    static COMPARE: LazyLock<Regex> = LazyLock::new(|| re(r#"path[\w.\[\]()'"?, ]*?(==|!=|\.startswith\()\s*["'](/[^"']*)["']"#));
    let handlers: Vec<(usize, String)> = HANDLER.captures_iter(source).map(|c| (c.get(0).unwrap().start(), c[1].to_string())).collect();
    for (i, (start, method)) in handlers.iter().enumerate() {
        let end = handlers.get(i + 1).map_or(source.len(), |h| h.0);
        for c in COMPARE.captures_iter(&source[*start..end]) {
            let line = line_of(source, start + c.get(0).unwrap().start());
            net.routes.push(Route { method: method.clone(), path: normalize(&c[2]), prefix: c[1].starts_with('.'), line });
        }
    }
    // Clients: requests/httpx/session/client.<method>(url), connection.request("GET", url).
    // String literals may hold `{expr}` parts with their own quotes (`f"/jobs/{job["id"]}"`).
    const PY_STR: &str = r#"(f?"(?:[^"\n{]|\{[^}\n]*\})*"|f?'(?:[^'\n{]|\{[^}\n]*\})*')"#;
    static CLIENT: LazyLock<Regex> = LazyLock::new(|| re(&format!(r#"\.(get|post|put|patch|delete|head)\(\s*(?:{PY_STR}|(self\.[A-Za-z_]\w*|[A-Za-z_]\w*))"#)));
    static REQUEST: LazyLock<Regex> = LazyLock::new(|| re(&format!(r#"\.request\(\s*["']([A-Z]+)["']\s*,\s*(?:{PY_STR}|([A-Za-z_][\w.]*))"#)));
    for c in CLIENT.captures_iter(source) {
        let at = c.get(0).unwrap().start();
        // `@router.get("/me")` is a route, not a call.
        if source[..at].rsplit('\n').next().is_some_and(|l| l.trim_start().starts_with('@')) {
            continue;
        }
        let arg = match c.get(2) {
            Some(lit) => format!("{}{}\"", &lit.as_str()[..lit.as_str().len() - 1], concat_tail(&source[lit.end()..])),
            None => c.get(3).map_or(String::new(), |m| m.as_str().to_string()),
        };
        python_call(net, &c[1], &arg, bindings, line_of(source, at));
    }
    for c in REQUEST.captures_iter(source) {
        let arg = c.get(2).or(c.get(3)).map_or("", |m| m.as_str());
        python_call(net, &c[1], arg, bindings, line_of(source, c.get(0).unwrap().start()));
    }
}

/// `'/jobs/' + job['id'] + '/resolve'`: the literal parts, with `{}` for each expression.
/// `rest` is the source right after the first literal.
fn concat_tail(rest: &str) -> String {
    static PART: LazyLock<Regex> = LazyLock::new(|| re(r#"^\s*\+\s*(?:"([^"\n]*)"|'([^'\n]*)'|`([^`]*)`|([A-Za-z_][\w.]*(?:\[[^\]\n]*\]|\([^)\n]*\))*))"#));
    let (mut out, mut at) = (String::new(), 0);
    while let Some(c) = PART.captures(&rest[at..]) {
        match c.get(1).or(c.get(2)).or(c.get(3)) {
            Some(lit) => out.push_str(lit.as_str()),
            None => out.push_str("{}"),
        }
        at += c.get(0).unwrap().end();
    }
    out
}

fn python_call(net: &mut Net, method: &str, arg: &str, bindings: &HashMap<String, String>, line: usize) {
    let raw = if arg.starts_with(['"', '\'']) || arg.starts_with("f\"") || arg.starts_with("f'") {
        let s = arg.trim_start_matches('f');
        s[1..s.len() - 1].to_string()
    } else {
        // A variable: only follow it when it is a known string (dict.get(key) is not a request).
        match bindings.get(arg.rsplit('.').next().unwrap_or(arg)) {
            Some(v) => v.clone(),
            None => return,
        }
    };
    push_call(net, method, &raw, bindings, line);
}

fn js(path: &str, source: &str, bindings: &HashMap<String, String>, net: &mut Net) {
    // Express / Hono / Fastify: `app.get("/x", ...)`, `router.post('/y', ...)`.
    static EXPRESS: LazyLock<Regex> = LazyLock::new(|| re(r#"\b(?:app|router|server|api)\.(get|post|put|patch|delete|all|use)\(\s*["'`](/[^"'`]*)["'`]\s*,"#));
    for c in EXPRESS.captures_iter(source) {
        let method = if &c[1] == "all" || &c[1] == "use" { "ANY".to_string() } else { c[1].to_uppercase() };
        net.routes.push(Route { method, path: normalize(&c[2]), prefix: &c[1] == "use", line: line_of(source, c.get(0).unwrap().start()) });
    }
    // Next.js API routes: `pages/api/x.ts`, `app/api/x/route.ts` export handlers per method.
    if let Some(route) = next_api_path(path) {
        static EXPORTED: LazyLock<Regex> = LazyLock::new(|| re(r"export\s+(?:async\s+)?function\s+(GET|POST|PUT|PATCH|DELETE)\b"));
        let methods: Vec<String> = EXPORTED.captures_iter(source).map(|c| c[1].to_string()).collect();
        for m in if methods.is_empty() { vec!["ANY".to_string()] } else { methods } {
            net.routes.push(Route { method: m, path: normalize(&route), prefix: false, line: 1 });
        }
    }
    // Clients: fetch(url, {method}), http.get(url), axios.post(url), and wrappers like call(c, url, post(..)).
    static CALL: LazyLock<Regex> = LazyLock::new(|| {
        re(r#"\b(fetch|request|call|api|get|post|put|patch|delete|head)(?:<[^>()]*>)?\(\s*(?:[A-Za-z_][\w.]*\s*,\s*)?(`[^`]*`|"[^"\n]*"|'[^'\n]*')([^;\n]*)"#)
    });
    static METHOD_OPT: LazyLock<Regex> = LazyLock::new(|| re(r#"method\s*:\s*["'`]([A-Za-z]+)["'`]|\b(post|put|patch|del|delete)\("#));
    for c in CALL.captures_iter(source) {
        let callee = &c[1];
        let mut method = if METHODS.contains(&callee) { callee.to_uppercase() } else { "GET".to_string() };
        if let Some(m) = METHOD_OPT.captures(&c[3]) {
            method = m.get(1).or(m.get(2)).map_or(method, |x| match x.as_str().to_lowercase().as_str() {
                "del" => "DELETE".into(),
                other => other.to_uppercase(),
            });
        }
        let lit = &c[2];
        push_call(net, &method, &lit[1..lit.len() - 1], bindings, line_of(source, c.get(0).unwrap().start()));
    }
}

/// `pages/api/users/[id].ts` -> `/api/users/{}`; `app/api/users/[id]/route.ts` -> same.
fn next_api_path(path: &str) -> Option<String> {
    let at = path.find("pages/api/").map(|i| i + "pages/".len()).or_else(|| path.find("app/api/").map(|i| i + "app/".len()))?;
    let rest = &path[at..];
    let rest = rest.strip_suffix("/route.ts").or_else(|| rest.strip_suffix("/route.js")).unwrap_or(rest);
    let rest = rest.rsplit_once('.').map_or(rest, |(p, _)| p);
    let rest = rest.strip_suffix("/index").unwrap_or(rest);
    Some(format!("/{}", rest.replace('[', "{").replace(']', "}")))
}

fn rust(source: &str, bindings: &HashMap<String, String>, net: &mut Net) {
    // axum: `.route("/x", get(a).post(b))`.
    static AXUM: LazyLock<Regex> = LazyLock::new(|| re(r#"\.route\(\s*"([^"]*)"\s*,\s*([^;]*?)\)\s*(?:\.|;|$)"#));
    static AXUM_METHOD: LazyLock<Regex> = LazyLock::new(|| re(r"\b(get|post|put|patch|delete|head|options|any)\("));
    for c in AXUM.captures_iter(source) {
        let line = line_of(source, c.get(0).unwrap().start());
        for m in AXUM_METHOD.captures_iter(&c[2]) {
            let method = if &m[1] == "any" { "ANY".to_string() } else { m[1].to_uppercase() };
            net.routes.push(Route { method, path: normalize(&c[1]), prefix: false, line });
        }
    }
    // actix / rocket: `#[get("/x")]`.
    static ATTR: LazyLock<Regex> = LazyLock::new(|| re(r#"#\[(get|post|put|patch|delete|head)\(\s*"([^"]*)""#));
    for c in ATTR.captures_iter(source) {
        net.routes.push(Route { method: c[1].to_uppercase(), path: normalize(&c[2]), prefix: false, line: line_of(source, c.get(0).unwrap().start()) });
    }
    // Hand-written routers matching on (method, path segments): `(&Method::GET, ["api", "graph"]) =>`.
    static SLICE: LazyLock<Regex> = LazyLock::new(|| re(r#"\(\s*(&?Method::([A-Z]+)|_|[a-z_]\w*)\s*,\s*\[([^\]]*)\]\s*\)\s*(?:if [^=]*)?=>"#));
    for c in SLICE.captures_iter(source) {
        let method = c.get(2).map_or("ANY".to_string(), |m| m.as_str().to_string());
        let mut prefix = false;
        let segments: Vec<String> = c[3]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                if s.ends_with("..") {
                    prefix = true;
                    None
                } else if let Some(lit) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    Some(lit.to_string())
                } else {
                    Some("{}".to_string())
                }
            })
            .collect();
        if segments.is_empty() {
            continue;
        }
        net.routes.push(Route { method, path: normalize(&format!("/{}", segments.join("/"))), prefix, line: line_of(source, c.get(0).unwrap().start()) });
    }
    // reqwest and friends: `.get(url)`, `.post(format!("{}/api/chat", host))`.
    static CLIENT: LazyLock<Regex> = LazyLock::new(|| re(r#"\.(get|post|put|patch|delete)\(\s*(?:&?format!\(\s*)?"([^"]*)""#));
    for c in CLIENT.captures_iter(source) {
        let raw = c[2].replacen("{}", "{base}", 1);
        push_call(net, &c[1], &raw, bindings, line_of(source, c.get(0).unwrap().start()));
    }
}

fn swift(source: &str, bindings: &HashMap<String, String>, net: &mut Net) {
    // `URL(string: "http://host/path")`, `url.appendingPathComponent("frame")`. The method is set on
    // the request elsewhere, so it is recorded as ANY.
    static URL_LIT: LazyLock<Regex> = LazyLock::new(|| re(r#"URL\(string:\s*"([^"]+)""#));
    static COMPONENT: LazyLock<Regex> = LazyLock::new(|| re(r#"appendingPathComponent\(\s*"([^"]+)""#));
    for c in URL_LIT.captures_iter(source) {
        push_call(net, "ANY", &c[1], bindings, line_of(source, c.get(0).unwrap().start()));
    }
    for c in COMPONENT.captures_iter(source) {
        push_call(net, "ANY", &format!("/{}", &c[1]), bindings, line_of(source, c.get(0).unwrap().start()));
    }
}
