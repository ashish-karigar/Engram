use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;

static RULES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    let rules = [
        ("private-key", r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----"),
        ("aws-key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b"),
        ("github-token", r"\bgh[pousr]_[A-Za-z0-9]{30,}\b"),
        ("api-key", r"\bsk-[A-Za-z0-9_-]{20,}\b"),
        ("bearer", r"\bBearer\s+[A-Za-z0-9._~+/=-]{16,}"),
    ];
    rules.iter().map(|(name, re)| (*name, Regex::new(re).expect("valid regex"))).collect()
});

static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(password|passwd|secret|token|api[_-]?key)\b(\s*[:=]\s*)(["']?)[^\s"']{6,}"#).expect("valid regex")
});

/// Remove likely secrets before anything is written to the event log.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    for (name, re) in RULES.iter() {
        out = re.replace_all(&out, format!("[REDACTED:{name}]")).into_owned();
    }
    ASSIGNMENT.replace_all(&out, |c: &Captures| format!("{}{}[REDACTED]", &c[1], &c[2])).into_owned()
}

pub fn scrub_value(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(scrub(&s)),
        Value::Array(items) => Value::Array(items.into_iter().map(scrub_value).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, scrub_value(v))).collect()),
        other => other,
    }
}
