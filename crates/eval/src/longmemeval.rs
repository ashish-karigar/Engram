//! Suite A: LongMemEval-S retrieval. Each question's chat history is replayed through Engram's own
//! session path (turn events, then transcript documents), the question goes through the same
//! ranking `search` uses, and we check whether an evidence session comes back.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use chrono::NaiveDateTime;
use engram_core::db::open_memory;
use engram_core::events::ensure_actor;
use engram_core::{Connection, Error, Result};
use engram_embed::Embedder;
use engram_serve::budget::estimate_tokens;
use engram_serve::ops::{Ctx, SearchReq, op_search, rank};
use engram_sessions::sync::record_turn;
use engram_sessions::transcript::rebuild_transcript;
use engram_sessions::{Agent, Role, Turn};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::report::{pct, percentile, table};
use crate::split::includes;
use crate::Split;

const KS: [usize; 3] = [5, 10, 20];
/// Chunks ranked per question; enough for 20 distinct sessions in practice.
const CANDIDATES: usize = 80;

#[derive(Deserialize)]
struct Question {
    question_id: String,
    question_type: String,
    question: String,
    answer: Value,
    answer_session_ids: Vec<String>,
    haystack_session_ids: Vec<String>,
    haystack_dates: Vec<String>,
    haystack_sessions: Vec<Vec<Message>>,
}

#[derive(Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct Outcome {
    id: String,
    kind: String,
    question: String,
    answer: Value,
    /// Any evidence session in the top K sessions, for K in `KS` (LongMemEval's recall_any@K).
    hits: Vec<bool>,
    /// All evidence sessions in the top K (stricter; matters for multi-session questions).
    all_hits: Vec<bool>,
    /// Chance that K sessions picked at random include any evidence session, for K in `KS`.
    random: Vec<f64>,
    /// Evidence session present in the answer an agent actually receives (cutoff + budget applied).
    in_answer: bool,
    answer_tokens: usize,
    /// Rank (1-based) of the first evidence session, if within the ranked list.
    first_rank: Option<usize>,
    rank_ms: f64,
    ingest_ms: f64,
}

/// "2023/05/20 (Sat) 02:21" -> RFC 3339. Turns in a session are a second apart so they keep order.
fn timestamp(date: &str, offset_secs: i64) -> String {
    let base = NaiveDateTime::parse_from_str(date, "%Y/%m/%d (%a) %H:%M").unwrap_or_default();
    (base + chrono::Duration::seconds(offset_secs)).and_utc().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn ingest(conn: &Connection, embedder: &dyn Embedder, q: &Question) -> Result<()> {
    let actor = ensure_actor(conn, "agent", Agent::Claude.actor_name())?.id;
    for ((sid, date), messages) in q.haystack_session_ids.iter().zip(&q.haystack_dates).zip(&q.haystack_sessions) {
        let mut external = None;
        for (i, m) in messages.iter().enumerate() {
            let role = if m.role == "user" { Role::User } else { Role::Assistant };
            let turn = Turn { session_id: sid.clone(), ts: timestamp(date, i as i64), role, text: m.content.clone(), tools: vec![], compaction: false, cwd: None };
            external = Some(record_turn(conn, Agent::Claude, &actor, &turn, format!("lme:{sid}:{i}"))?);
        }
        if let Some(external) = external {
            rebuild_transcript(conn, embedder, &external)?;
        }
    }
    Ok(())
}

/// P(at least one of `e` evidence items among `k` drawn without replacement from `n`) = 1 - C(n-e,k)/C(n,k).
fn random_any(n: usize, e: usize, k: usize) -> f64 {
    if k >= n {
        return 1.0;
    }
    let none: f64 = (0..k).map(|i| (n - e).saturating_sub(i) as f64 / (n - i) as f64).product();
    1.0 - none
}

/// Document id -> LongMemEval session id, via the transcript's metadata (`claude:<session>`).
fn sessions_by_document(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT id, json_extract(metadata, '$.session') FROM documents")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        if let (doc, Some(session)) = row? {
            out.insert(doc, session.split_once(':').map_or(session.clone(), |(_, s)| s.to_string()));
        }
    }
    Ok(out)
}

fn evaluate(q: &Question, embedder: &dyn Embedder) -> Result<Outcome> {
    let conn = open_memory()?;
    let started = Instant::now();
    ingest(&conn, embedder, q)?;
    let ingest_ms = started.elapsed().as_secs_f64() * 1000.0;
    let sessions = sessions_by_document(&conn)?;
    let actor = ensure_actor(&conn, "user", "engram-eval")?;
    let ctx = Ctx { conn: &conn, actor: &actor, embedder };

    let started = Instant::now();
    let ranked = rank(&ctx, &q.question, "hybrid", CANDIDATES)?;
    let rank_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut order: Vec<&str> = vec![];
    for r in &ranked {
        if let Some(s) = r.document_id.as_deref().and_then(|d| sessions.get(d)) {
            if !order.contains(&s.as_str()) {
                order.push(s);
            }
        }
    }
    let is_evidence = |s: &str| q.answer_session_ids.iter().any(|a| a == s);
    let first_rank = order.iter().position(|s| is_evidence(s)).map(|p| p + 1);
    let hits = KS.iter().map(|&k| first_rank.is_some_and(|r| r <= k)).collect();
    let all_hits = KS
        .iter()
        .map(|&k| q.answer_session_ids.iter().all(|a| order.iter().take(k).any(|s| *s == a.as_str())))
        .collect();
    let (n, e) = (q.haystack_session_ids.len(), q.answer_session_ids.len());
    let random = KS.iter().map(|&k| random_any(n, e, k)).collect();

    // What an agent is actually shown: the default search answer.
    let answer = op_search(&ctx, SearchReq { query: q.question.clone(), budget: None, as_of: None, mode: None })?;
    let chunk_docs: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT id, document_id FROM chunks")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<engram_core::rusqlite::Result<_>>()?
    };
    let in_answer = answer.data.as_array().into_iter().flatten().filter_map(|r| r["id"].as_str()).any(|id| {
        chunk_docs.get(id).and_then(|d| sessions.get(d)).is_some_and(|s| is_evidence(s))
    });

    Ok(Outcome {
        id: q.question_id.clone(),
        kind: q.question_type.clone(),
        question: q.question.clone(),
        answer: q.answer.clone(),
        hits,
        all_hits,
        random,
        in_answer,
        answer_tokens: estimate_tokens(&answer.text),
        first_rank,
        rank_ms,
        ingest_ms,
    })
}

pub fn run(data: &Path, split: Split, embedder: Arc<dyn Embedder>, limit: Option<usize>, out: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(data).map_err(|e| Error::msg(format!("{}: {e}", data.display())))?;
    let all: Vec<Question> = serde_json::from_str(&text)?;
    // Abstention questions have no evidence to retrieve; LongMemEval excludes them from retrieval scoring.
    let mut questions: Vec<&Question> = all.iter().filter(|q| !q.question_id.ends_with("_abs") && includes(split, &q.question_id)).collect();
    if let Some(n) = limit {
        questions.truncate(n);
    }
    eprintln!("LongMemEval-S: {} questions, embeddings {}", questions.len(), embedder.model_id());

    let started = Instant::now();
    let outcomes: Vec<Outcome> = questions
        .par_iter()
        .map(|q| evaluate(q, embedder.as_ref()).unwrap_or_else(|e| panic!("{}: {e}", q.question_id)))
        .collect();

    let mut groups: BTreeMap<String, Vec<&Outcome>> = BTreeMap::new();
    for o in &outcomes {
        groups.entry(o.kind.clone()).or_default().push(o);
    }
    let summarize = |os: &[&Outcome]| -> Vec<String> {
        let mut cells = vec![os.len().to_string()];
        for i in 0..KS.len() {
            cells.push(format!("{:.1}", pct(&os.iter().map(|o| o.hits[i]).collect::<Vec<_>>())));
        }
        cells.push(format!("{:.1}", pct(&os.iter().map(|o| o.all_hits[0]).collect::<Vec<_>>())));
        cells.push(format!("{:.1}", 100.0 * os.iter().map(|o| o.random[0]).sum::<f64>() / os.len().max(1) as f64));
        cells.push(format!("{:.1}", 100.0 * os.iter().map(|o| o.random[2]).sum::<f64>() / os.len().max(1) as f64));
        cells.push(format!("{:.1}", pct(&os.iter().map(|o| o.in_answer).collect::<Vec<_>>())));
        cells.push(format!("{:.0}", os.iter().map(|o| o.answer_tokens).sum::<usize>() as f64 / os.len().max(1) as f64));
        cells
    };
    let rows: BTreeMap<String, Vec<String>> = groups.iter().map(|(k, os)| (k.clone(), summarize(os))).collect();
    let everyone: Vec<&Outcome> = outcomes.iter().collect();
    println!("{}", table(&["category", "n", "R@5", "R@10", "R@20", "all@5", "random R@5", "random R@20", "in answer", "answer tokens"], &rows, summarize(&everyone)));
    let rank_ms: Vec<f64> = outcomes.iter().map(|o| o.rank_ms).collect();
    println!(
        "\nsearch latency p50 {:.1} ms, p95 {:.1} ms · ingest per question p50 {:.0} ms · total {:.0} s",
        percentile(&rank_ms, 0.5),
        percentile(&rank_ms, 0.95),
        percentile(&outcomes.iter().map(|o| o.ingest_ms).collect::<Vec<_>>(), 0.5),
        started.elapsed().as_secs_f64()
    );
    if let Some(path) = out {
        std::fs::write(path, serde_json::to_string_pretty(&outcomes)?)?;
    }
    Ok(())
}
