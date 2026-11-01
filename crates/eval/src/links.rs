//! Suite H: how well the model links memories. Each labelled case is one memory and one
//! candidate with the relations a person would accept (none = unrelated). Cases run through the
//! same `judge` call and the same apply/review/drop rule as the app, so the numbers describe what
//! would actually happen to a project's memory.

use std::path::Path;

use engram_core::db::open_memory;
use engram_core::{Error, Result};
use engram_core::memory::{NewMemory, create_memory};
use engram_understand::judge::{judge_raw, outcome, parse_verdicts};
use engram_understand::llm::{LlmClient, MetaClient, OllamaClient, client_from_env};
use serde::Deserialize;
use serde_json::Value;

const CASES: &str = include_str!("../data/links.json");

#[derive(Deserialize)]
struct Case {
    /// [claim, date]
    m: Vec<String>,
    /// [claim, date] or [claim, date, "code"]
    c: Vec<String>,
    accept: Vec<String>,
}

/// The model from the environment, else the one configured in the app. The key is used, never shown.
fn client() -> Option<Box<dyn LlmClient>> {
    if let Some(c) = client_from_env() {
        return Some(c);
    }
    let home = std::env::var_os("HOME")?;
    let text = std::fs::read_to_string(Path::new(&home).join("Library/Application Support/Engram/settings.json")).ok()?;
    let llm = serde_json::from_str::<Value>(&text).ok()?["llm"].clone();
    let get = |k: &str| llm[k].as_str().unwrap_or("").to_string();
    match get("provider").as_str() {
        "meta" if !get("metaApiKey").is_empty() => {
            Some(Box::new(MetaClient { base_url: get("metaBaseUrl"), api_key: get("metaApiKey"), model: get("metaModel") }))
        }
        "ollama" => Some(Box::new(OllamaClient { host: get("ollamaHost"), model: get("ollamaModel") })),
        _ => None,
    }
}

fn title(claim: &str) -> String {
    claim.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
}

pub fn run() -> Result<()> {
    let llm = client().ok_or_else(|| Error::msg("no model configured: set META_API_KEY, or choose a model in the app's Settings"))?;
    let cases: Vec<Case> = serde_json::from_str(CASES)?;
    let conn = open_memory()?;
    let (mut right, mut harmful, mut false_links, mut missed, mut errors) = (0, 0, 0, 0, 0);
    let (mut applied, mut pending) = (0, 0);
    println!("Linking memories with {} on {} labelled pairs\n", llm.name(), cases.len());
    for (i, case) in cases.iter().enumerate() {
        let at = |date: &str| format!("{date}T00:00:00Z");
        let m = create_memory(&conn, NewMemory { valid_from: Some(at(&case.m[1])), ..NewMemory::new("fact", &title(&case.m[0]), &case.m[0]) })?;
        let from_code = case.c.get(2).is_some_and(|s| s == "code");
        let scope = from_code.then(|| "system:eval:uses".to_string());
        let c = create_memory(&conn, NewMemory { valid_from: Some(at(&case.c[1])), scope, ..NewMemory::new("fact", &title(&case.c[0]), &case.c[0]) })?;
        let raw = match judge_raw(llm.as_ref(), &m, std::slice::from_ref(&c)) {
            Ok(raw) => raw,
            Err(err) => {
                errors += 1;
                println!("{:>2}  error: {err}", i + 1);
                continue;
            }
        };
        let verdicts = parse_verdicts(&raw, std::slice::from_ref(&c.id));
        let v = verdicts.first();
        let (got, status) = v.and_then(|v| outcome(v, from_code)).unwrap_or(("unrelated", "-"));
        let expected = if case.accept.is_empty() { "unrelated".to_string() } else { case.accept.join("|") };
        let ok = if case.accept.is_empty() { got == "unrelated" } else { case.accept.iter().any(|a| a == got) };
        right += ok as usize;
        applied += (status == "applied") as usize;
        pending += (status == "pending") as usize;
        if !ok {
            if case.accept.is_empty() {
                false_links += 1;
            } else if got == "unrelated" {
                missed += 1;
            }
            // A wrong link that retires a memory without anyone reviewing it.
            if status == "applied" && matches!(got, "same" | "replaces") {
                harmful += 1;
            }
        }
        let conf = v.map(|v| format!("{:.2}", v.confidence)).unwrap_or_default();
        println!("{:>2} {} expected {expected:<22} got {got:<11} {conf:<4} {status}", i + 1, if ok { "✓" } else { "✗" });
        // `LINKS_RAW=1` shows what the model actually replied for a miss.
        if !ok && std::env::var_os("LINKS_RAW").is_some() {
            println!("      raw: {}", raw.replace('\n', " ").chars().take(400).collect::<String>());
        }
        if !ok {
            println!("      M: {}\n      C: {}{}", case.m[0], case.c[0], v.map(|v| format!("\n      model: {}", v.reason)).unwrap_or_default());
        }
    }
    let judged = cases.len() - errors;
    let pct = |n: usize| if judged == 0 { 0.0 } else { 100.0 * n as f64 / judged as f64 };
    println!("\nCorrect relation       {right}/{judged} ({:.0}%)", pct(right));
    println!("Linked unrelated pairs {false_links}");
    println!("Missed real links      {missed}");
    println!("Wrongly retired a memory without review: {harmful}");
    println!("Applied {applied}, sent to review {pending}, failed calls {errors}");
    Ok(())
}
