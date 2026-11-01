//! Services a project runs on that its code never imports: the database in `docker-compose.yml`,
//! the secrets manager a deploy script logs into, the host its docs name. Read from config,
//! scripts, CI workflows, example env files and docs, never from real `.env` files (secrets).
//!
//! Each finding is either `config` (the file sets the service up: an image, a connection URL, an
//! env key, a CLI call) or `docs` (prose or a comment only names it), with the line it came from.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub service: &'static str,
    pub line: usize,
    /// `config` or `docs`.
    pub how: &'static str,
    /// What gave it away: `image postgres:17-alpine`, `env AUTH0_DOMAIN`, `mentions "Cloudflare"`.
    pub why: String,
    /// The sentence (docs) or line (config) it was found in, to tell which part it is about.
    pub context: String,
}

/// Container images, by their last path segment (`ollama/ollama` → `ollama`).
const IMAGES: &[(&str, &str)] = &[
    ("postgres", "PostgreSQL"), ("postgis", "PostgreSQL"), ("mysql", "MySQL"), ("mariadb", "MySQL"), ("redis", "Redis"),
    ("valkey", "Redis"), ("mongo", "MongoDB"), ("ollama", "Ollama"), ("rabbitmq", "RabbitMQ"), ("elasticsearch", "Elasticsearch"),
    ("minio", "MinIO"), ("caddy", "Caddy"), ("nginx", "Nginx"), ("traefik", "Traefik"), ("kafka", "Kafka"), ("memcached", "Memcached"),
];
/// Connection URL schemes.
const SCHEMES: &[(&str, &str)] = &[
    ("postgres", "PostgreSQL"), ("postgresql", "PostgreSQL"), ("mysql", "MySQL"), ("mariadb", "MySQL"), ("redis", "Redis"),
    ("rediss", "Redis"), ("mongodb", "MongoDB"), ("mongodb+srv", "MongoDB"), ("amqp", "RabbitMQ"), ("amqps", "RabbitMQ"),
];
/// Environment variable prefixes (`AUTH0_DOMAIN`, `INFISICAL_CLIENT_ID`).
const ENV: &[(&str, &str)] = &[
    ("AUTH0_", "Auth0"), ("INFISICAL_", "Infisical"), ("CLOUDFLARE_", "Cloudflare"), ("CF_API_", "Cloudflare"), ("CF_ACCOUNT_", "Cloudflare"),
    ("STRIPE_", "Stripe"), ("SUPABASE_", "Supabase"), ("FIREBASE_", "Firebase"), ("OPENAI_", "OpenAI"), ("ANTHROPIC_", "Anthropic"),
    ("OLLAMA_", "Ollama"), ("SENTRY_", "Sentry"), ("POSTHOG_", "PostHog"), ("POSTGRES_", "PostgreSQL"), ("PGHOST", "PostgreSQL"),
    ("PGPASSWORD", "PostgreSQL"), ("REDIS_", "Redis"), ("MONGO_", "MongoDB"), ("MONGODB_", "MongoDB"), ("AWS_", "AWS"), ("OCI_", "Oracle"),
    ("GOOGLE_", "Google APIs"), ("GMAIL_", "Gmail"), ("TWILIO_", "Twilio"), ("SENDGRID_", "SendGrid"), ("RESEND_", "Resend"),
    ("CLERK_", "Clerk"), ("OKTA_", "Okta"), ("VERCEL_", "Vercel"), ("NETLIFY_", "Netlify"), ("FLY_API_", "Fly.io"),
];
/// Command-line tools a script or workflow runs.
const CLIS: &[(&str, &str)] = &[
    ("infisical", "Infisical"), ("wrangler", "Cloudflare"), ("oci", "Oracle"), ("aws", "AWS"), ("gcloud", "Google Cloud"),
    ("flyctl", "Fly.io"), ("vercel", "Vercel"), ("netlify", "Netlify"), ("supabase", "Supabase"), ("heroku", "Heroku"),
    ("psql", "PostgreSQL"), ("pg_dump", "PostgreSQL"), ("pg_restore", "PostgreSQL"), ("pg_isready", "PostgreSQL"), ("redis-cli", "Redis"),
    ("ollama", "Ollama"),
];
/// GitHub Actions publishers (`uses: cloudflare/pages-action@v1`).
const ACTIONS: &[(&str, &str)] = &[
    ("cloudflare", "Cloudflare"), ("aws-actions", "AWS"), ("google-github-actions", "Google Cloud"), ("infisical", "Infisical"),
    ("supabase", "Supabase"), ("amondnet", "Vercel"), ("superfly", "Fly.io"), ("oracle-actions", "Oracle"),
];
/// Files whose presence alone says where the project is deployed.
const PLATFORM_FILES: &[(&str, &str)] = &[
    ("wrangler.toml", "Cloudflare"), ("wrangler.json", "Cloudflare"), ("wrangler.jsonc", "Cloudflare"), ("netlify.toml", "Netlify"),
    ("vercel.json", "Vercel"), ("fly.toml", "Fly.io"), ("render.yaml", "Render"), ("railway.json", "Railway"), ("firebase.json", "Firebase"),
];
/// Proper names prose uses for a service, matched case-sensitively as whole words.
const NAMES: &[(&str, &str)] = &[
    ("PostgreSQL", "PostgreSQL"), ("Postgres", "PostgreSQL"), ("Auth0", "Auth0"), ("Infisical", "Infisical"), ("Cloudflare", "Cloudflare"),
    ("Oracle", "Oracle"), ("OCI", "Oracle"), ("Ollama", "Ollama"), ("Redis", "Redis"), ("MongoDB", "MongoDB"), ("MySQL", "MySQL"),
    ("Stripe", "Stripe"), ("Supabase", "Supabase"), ("Firebase", "Firebase"), ("OpenAI", "OpenAI"), ("Anthropic", "Anthropic"),
    ("Sentry", "Sentry"), ("PostHog", "PostHog"), ("AWS", "AWS"), ("Vercel", "Vercel"), ("Netlify", "Netlify"), ("Fly.io", "Fly.io"),
    ("Heroku", "Heroku"), ("Twilio", "Twilio"), ("SendGrid", "SendGrid"), ("Resend", "Resend"), ("Gmail", "Gmail"), ("Clerk", "Clerk"),
    ("Okta", "Okta"), ("Caddy", "Caddy"), ("RabbitMQ", "RabbitMQ"), ("Elasticsearch", "Elasticsearch"),
];

static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*-?\s*image:\s*["']?([\w./-]+)(?::([\w.-]+))?"#).unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([a-z][a-z0-9+]*)(?:\+[a-z0-9]+)?://").unwrap());
static ENV_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Z][A-Z0-9_]{2,}\b").unwrap());
static ACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-?\s*uses:\s*([\w.-]+)/").unwrap());
static NAME: LazyLock<Vec<(Regex, &'static str, &'static str)>> = LazyLock::new(|| {
    NAMES.iter().map(|(n, s)| (Regex::new(&format!(r"(?:^|[^\w.]){}(?:$|[^\w])", regex::escape(n))).unwrap(), *n, *s)).collect()
});

static NAME_ANY_CASE: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    NAMES.iter().map(|(n, s)| (Regex::new(&format!(r"(?i)(?:^|[^\w.]){}(?:$|[^\w])", regex::escape(n))).unwrap(), *s)).collect()
});

/// The services a piece of text names, in any case ("hosted on oracle"): for reading what people wrote.
pub fn named_services(text: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    for (re, service) in NAME_ANY_CASE.iter() {
        if re.is_match(text) && !out.contains(service) {
            out.push(service);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// docker-compose, Kubernetes and other YAML/TOML config.
    Config,
    /// An example env file: keys only, no secrets.
    Env,
    /// Shell scripts, Dockerfiles, Makefiles and CI workflows: they also run commands.
    Script,
    Docs,
    /// Source code: only connection URLs (`"postgresql://"`); imports are read elsewhere.
    Code,
}

fn kind_of(path: &str) -> Option<Kind> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    // Real env files hold secrets; only their committed templates are read.
    if lower.starts_with(".env") || lower.ends_with(".env.example") {
        return ["example", "sample", "template", "dist"].iter().any(|s| lower.ends_with(s)).then_some(Kind::Env);
    }
    if path.starts_with(".github/workflows/") || ["sh", "bash", "zsh"].contains(&ext) || lower.starts_with("dockerfile") || lower == "makefile" || lower == "procfile" {
        return Some(Kind::Script);
    }
    if lower.contains("lock") {
        return None;
    }
    if ["yml", "yaml", "toml", "ini", "conf"].contains(&ext) {
        return Some(Kind::Config);
    }
    if ["md", "mdx", "rst", "txt"].contains(&ext) {
        return Some(Kind::Docs);
    }
    if ["py", "ts", "tsx", "js", "jsx", "mjs", "rs", "go", "java", "kt", "rb", "php", "swift", "cs"].contains(&ext) {
        return Some(Kind::Code);
    }
    None
}

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// Split a config or script line into its active part and its comment (`# ...`).
fn split_comment(line: &str) -> (&str, &str) {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('#', None) if i == 0 || line[..i].ends_with(char::is_whitespace) => return (&line[..i], &line[i + 1..]),
            _ => {}
        }
    }
    (line, "")
}

/// Every service the given files set up or name. `read` returns a file's text.
pub fn scan(files: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<Finding> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    let mut push = |file: &str, service: &'static str, line: usize, how: &'static str, why: String, context: &str| {
        // Config: the first line per file and service is enough. Docs: every sentence, since each
        // may be about a different part ("Cloudflare Pages builds `frontend/`").
        // An image line is kept too: it says the service runs next to what the file deploys.
        let detail = if how == "docs" { context.to_string() } else if why.starts_with("image ") { "image".into() } else { String::new() };
        let key = (file.to_string(), service, how, detail);
        if seen.insert(key) {
            out.push(Finding { file: file.to_string(), service, line, how, why, context: context.trim().to_string() });
        }
    };
    for file in files {
        let name = file.rsplit('/').next().unwrap_or(file);
        if let Some(service) = lookup(PLATFORM_FILES, name) {
            push(file, service, 1, "config", format!("file {name}"), file);
        }
        let Some(kind) = kind_of(file) else { continue };
        let Some(text) = read(file) else { continue };
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let (active, comment) = match kind {
                Kind::Docs => ("", raw),
                Kind::Code => (raw, ""),
                _ => split_comment(raw),
            };
            if kind == Kind::Code {
                for c in SCHEME.captures_iter(active) {
                    if let Some(s) = lookup(SCHEMES, &c[1]) {
                        push(file, s, line, "config", format!("{}:// URL", &c[1]), raw);
                    }
                }
            } else if kind != Kind::Docs {
                scan_config(active, kind, |s, why| push(file, s, line, "config", why, raw));
            }
            for sentence in sentences(comment) {
                for (re, word, service) in NAME.iter() {
                    if re.is_match(sentence) {
                        push(file, service, line, "docs", format!("mentions \"{word}\""), sentence);
                    }
                }
            }
        }
    }
    // A file that sets a service up also names it; keep only the stronger finding.
    let configured: HashSet<(String, &str)> = out.iter().filter(|f| f.how == "config").map(|f| (f.file.clone(), f.service)).collect();
    out.retain(|f| f.how == "config" || !configured.contains(&(f.file.clone(), f.service)));
    out
}

/// A line's sentences: split after `.`, `!` or `?` followed by a space.
fn sentences(line: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut cuts = vec![];
    for i in 0..bytes.len().saturating_sub(1) {
        if matches!(bytes[i], b'.' | b'!' | b'?') && bytes[i + 1] == b' ' {
            cuts.push(i + 1);
        }
    }
    cuts.push(line.len());
    cuts.into_iter().map(move |end| {
        let s = &line[start..end];
        start = end;
        s
    }).filter(|s| !s.trim().is_empty())
}

fn scan_config(active: &str, kind: Kind, mut found: impl FnMut(&'static str, String)) {
    if let Some(c) = IMAGE.captures(active) {
        let image = &c[1];
        let base = image.rsplit('/').next().unwrap_or(image);
        if let Some(s) = lookup(IMAGES, base) {
            found(s, format!("image {image}{}", c.get(2).map_or(String::new(), |t| format!(":{}", t.as_str()))));
        }
    }
    for c in SCHEME.captures_iter(active) {
        if let Some(s) = lookup(SCHEMES, &c[1]) {
            found(s, format!("{}:// URL", &c[1]));
        }
    }
    for m in ENV_KEY.find_iter(active) {
        if let Some((_, s)) = ENV.iter().find(|(p, _)| m.as_str().starts_with(p)) {
            found(s, format!("env {}", m.as_str()));
        }
    }
    if kind == Kind::Script {
        if let Some(c) = ACTION.captures(active) {
            if let Some(s) = lookup(ACTIONS, &c[1].to_lowercase()) {
                found(s, format!("action {}", &c[1]));
            }
        }
        // A tool in command position: at the start, or after a pipe, `&&`, `;`, `$(` or `run:`.
        for part in active.split(['|', ';', '&', '(', '`']) {
            let part = part.trim_start().trim_start_matches("run:").trim_start().trim_start_matches("- ").trim_start();
            let part = part.strip_prefix("sudo ").unwrap_or(part);
            let word = part.split_whitespace().next().unwrap_or("");
            if let Some(s) = lookup(CLIS, word) {
                found(s, format!("runs {word}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(files: &[(&str, &str)]) -> Vec<(String, &'static str, &'static str)> {
        let list: Vec<String> = files.iter().map(|(p, _)| p.to_string()).collect();
        let mut got: Vec<_> = scan(&list, |p| files.iter().find(|(f, _)| *f == p).map(|(_, t)| t.to_string()))
            .into_iter()
            .map(|f| (f.file, f.service, f.how))
            .collect();
        got.sort();
        got.dedup();
        got
    }

    #[test]
    fn compose_images_urls_and_env() {
        let compose = "# API + Postgres + Ollama, behind Caddy\nservices:\n  api:\n    environment:\n      DATABASE_URL: postgresql://u:${POSTGRES_PASSWORD}@db:5432/x\n      OLLAMA_BASE_URL: http://ollama:11434\n  db:\n    image: postgres:17-alpine\n";
        let got = run(&[("deploy/docker-compose.yml", compose)]);
        assert_eq!(got, vec![
            ("deploy/docker-compose.yml".into(), "Caddy", "docs"),
            ("deploy/docker-compose.yml".into(), "Ollama", "config"),
            ("deploy/docker-compose.yml".into(), "PostgreSQL", "config"),
        ]);
    }

    #[test]
    fn scripts_workflows_and_docs() {
        let script = "#!/bin/bash\nsource \"$HOME/.config/x/infisical.conf\"\nTOKEN=$(infisical login --method=universal-auth)\n";
        let workflow = "# The frontend deploys itself through Cloudflare Pages.\njobs:\n  deploy:\n    steps:\n      - uses: actions/checkout@v4\n      - run: ssh host 'cd /opt && ./deploy.sh'\n";
        let readme = "Everything runs in Docker on the Oracle server.\nAuth0 tokens are checked. An OCIrrelevant word.\n";
        let got = run(&[("deploy/refresh-secrets.sh", script), (".github/workflows/deploy.yml", workflow), ("deploy/README.md", readme)]);
        assert_eq!(got, vec![
            (".github/workflows/deploy.yml".into(), "Cloudflare", "docs"),
            ("deploy/README.md".into(), "Auth0", "docs"),
            ("deploy/README.md".into(), "Oracle", "docs"),
            ("deploy/refresh-secrets.sh".into(), "Infisical", "config"),
        ]);
    }

    #[test]
    fn env_templates_only() {
        let env = "AUTH0_DOMAIN=\nDATABASE_URL=postgresql://localhost/x\n";
        let code = "AUTH0_X = 1  # Postgres\nfor prefix in (\"postgres://\", \"postgresql://\"):\n";
        let got = run(&[("backend/.env.example", env), ("backend/.env", env), ("wrangler.toml", "name = \"x\"\n"), ("app/config.py", code)]);
        assert_eq!(got, vec![
            ("app/config.py".into(), "PostgreSQL", "config"),
            ("backend/.env.example".into(), "Auth0", "config"),
            ("backend/.env.example".into(), "PostgreSQL", "config"),
            ("wrangler.toml".into(), "Cloudflare", "config"),
        ]);
    }
}
