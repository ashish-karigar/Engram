//! Suite A, answer accuracy: LongMemEval's own protocol with MemoryOS as the retriever. For each
//! question: replay the history, take the top chunks from the same ranking `search` uses, let the
//! model answer with the official reader prompt, then let the model judge the answer with the
//! official per-type judge prompt (https://github.com/xiaowu0162/LongMemEval, MIT).
//!
//! Every answer and verdict is appended to a JSONL file as it finishes, so a run can be stopped and
//! resumed, and the saved answers can be judged again by another model without answering again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use engram_core::db::open_memory;
use engram_core::events::ensure_actor;
use engram_core::{Error, Result};
use engram_embed::Embedder;
use engram_serve::ops::{Ctx, rank_as_of};
use engram_understand::llm::{LlmClient, LlmRequest};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Split;
use crate::longmemeval::{Question, ingest, sessions_by_document, timestamp};
use crate::report::table;
use crate::split::includes;

/// Ranked chunks considered per question; the context is built from the best `context` of them.
const CANDIDATES: usize = 80;

#[derive(Serialize, Deserialize, Clone)]
pub struct Graded {
    pub id: String,
    pub kind: String,
    pub question: String,
    pub gold: String,
    pub hypothesis: String,
    pub correct: bool,
    pub reader: String,
    pub judge: String,
    /// Chunks and sessions given to the reader, and its rough size.
    pub chunks: usize,
    pub sessions: usize,
    pub context_tokens: usize,
}

fn gold(q: &Question) -> String {
    match &q.answer {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The reader prompt from LongMemEval's `run_generation.py` (retrieval, chain of thought).
fn reader_prompt(history: &str, date: &str, question: &str) -> String {
    format!(
        "I will give you several history chats between you and a user. Please answer the question based on the relevant chat history. Answer the question step by step: first extract all the relevant information, and then reason over the information to get the answer.\n\n\nHistory Chats:\n\n{history}\n\nCurrent Date: {date}\nQuestion: {question}\nAnswer (step by step):"
    )
}

/// The judge prompts from LongMemEval's `evaluate_qa.py`, unchanged.
fn judge_prompt(kind: &str, abstention: bool, question: &str, answer: &str, response: &str) -> String {
    if abstention {
        return format!("I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.\n\nQuestion: {question}\n\nExplanation: {answer}\n\nModel Response: {response}\n\nDoes the model correctly identify the question as unanswerable? Answer yes or no only.");
    }
    match kind {
        "temporal-reasoning" => format!("I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no. In addition, do not penalize off-by-one errors for the number of days. If the question asks for the number of days/weeks/months, etc., and the model makes off-by-one errors (e.g., predicting 19 days when the answer is 18), the model's response is still correct. \n\nQuestion: {question}\n\nCorrect Answer: {answer}\n\nModel Response: {response}\n\nIs the model response correct? Answer yes or no only."),
        "knowledge-update" => format!("I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response contains some previous information along with an updated answer, the response should be considered as correct as long as the updated answer is the required answer.\n\nQuestion: {question}\n\nCorrect Answer: {answer}\n\nModel Response: {response}\n\nIs the model response correct? Answer yes or no only."),
        "single-session-preference" => format!("I will give you a question, a rubric for desired personalized response, and a response from a model. Please answer yes if the response satisfies the desired response. Otherwise, answer no. The model does not need to reflect all the points in the rubric. The response is correct as long as it recalls and utilizes the user's personal information correctly.\n\nQuestion: {question}\n\nRubric: {answer}\n\nModel Response: {response}\n\nIs the model response correct? Answer yes or no only."),
        _ => format!("I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no. \n\nQuestion: {question}\n\nCorrect Answer: {answer}\n\nModel Response: {response}\n\nIs the model response correct? Answer yes or no only."),
    }
}

/// The retrieved history in LongMemEval's layout: whole chunks grouped by session, sessions by date.
fn history(q: &Question, embedder: &dyn Embedder, context: usize) -> Result<(String, usize, usize)> {
    let conn = open_memory()?;
    ingest(&conn, embedder, q)?;
    let sessions = sessions_by_document(&conn)?;
    let actor = ensure_actor(&conn, "user", "engram-eval")?;
    let ctx = Ctx { conn: &conn, actor: &actor, embedder, rerank: None };
    let ranked = rank_as_of(&ctx, &q.question, "hybrid", CANDIDATES, Some(timestamp(&q.question_date, 0)))?;
    let picked: Vec<_> = ranked.iter().filter(|r| r.document_id.is_some()).take(context).collect();

    let date_of: HashMap<&str, &str> = q.haystack_session_ids.iter().map(String::as_str).zip(q.haystack_dates.iter().map(String::as_str)).collect();
    let mut by_session: BTreeMap<(String, String), Vec<(i64, String)>> = BTreeMap::new();
    for r in &picked {
        let Some(session) = r.document_id.as_deref().and_then(|d| sessions.get(d)) else { continue };
        // The heading says who spoke ("You · 14:02").
        let (ord, content): (i64, String) = conn.query_row(
            "SELECT ordinal, COALESCE(heading || char(10), '') || content FROM chunks WHERE id = ?",
            [&r.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let date = date_of.get(session.as_str()).copied().unwrap_or("");
        // Sorting key: the session's own timestamp, so the history reads in order.
        by_session.entry((timestamp(date, 0), date.to_string())).or_default().push((ord, content));
    }
    let mut out = String::new();
    for (i, ((_, date), mut chunks)) in by_session.into_iter().enumerate() {
        chunks.sort();
        let body: Vec<String> = chunks.into_iter().map(|(_, c)| c).collect();
        out.push_str(&format!("### Session {}:\nSession Date: {date}\nSession Content:\n{}\n\n", i + 1, body.join("\n\n")));
    }
    let sessions_used = out.matches("### Session").count();
    Ok((out, picked.len(), sessions_used))
}

fn grade(q: &Question, embedder: &dyn Embedder, reader: &dyn LlmClient, judge: &dyn LlmClient, context: usize) -> Result<Graded> {
    let (hist, chunks, sessions) = history(q, embedder, context)?;
    // Reasoning models spend part of max_tokens thinking, so both budgets are well above the
    // official 800 / 10 tokens.
    let hypothesis = reader.complete(&LlmRequest { system: String::new(), prompt: reader_prompt(&hist, &q.question_date, &q.question), max_tokens: 6000 })?;
    let abstention = q.question_id.ends_with("_abs");
    let verdict = judge.complete(&LlmRequest { system: String::new(), prompt: judge_prompt(&q.question_type, abstention, &q.question, &gold(q), &hypothesis), max_tokens: 2000 })?;
    Ok(Graded {
        id: q.question_id.clone(),
        kind: if abstention { "abstention".into() } else { q.question_type.clone() },
        question: q.question.clone(),
        gold: gold(q),
        hypothesis: hypothesis.trim().to_string(),
        // The official parse: any "yes" in the reply.
        correct: verdict.to_lowercase().contains("yes"),
        reader: reader.name(),
        judge: judge.name(),
        chunks,
        sessions,
        context_tokens: hist.len() / 4,
    })
}

fn report(rows: &[Graded]) {
    let mut groups: BTreeMap<String, Vec<&Graded>> = BTreeMap::new();
    for r in rows {
        groups.entry(r.kind.clone()).or_default().push(r);
    }
    let cells = |rs: &[&Graded]| {
        let right = rs.iter().filter(|r| r.correct).count();
        let tokens = rs.iter().map(|r| r.context_tokens).sum::<usize>() / rs.len().max(1);
        vec![rs.len().to_string(), format!("{:.1}", 100.0 * right as f64 / rs.len().max(1) as f64), tokens.to_string()]
    };
    let table_rows: BTreeMap<String, Vec<String>> = groups.iter().map(|(k, rs)| (k.clone(), cells(rs))).collect();
    let all: Vec<&Graded> = rows.iter().collect();
    println!("{}", table(&["category", "n", "accuracy", "context tokens"], &table_rows, cells(&all)));
    // LongMemEval also reports the mean over question types, so large types do not dominate.
    let per_type: Vec<f64> = groups.values().map(|rs| rs.iter().filter(|r| r.correct).count() as f64 / rs.len() as f64).collect();
    println!("\nmean over question types: {:.1}", 100.0 * per_type.iter().sum::<f64>() / per_type.len().max(1) as f64);
}

pub struct QaSetup<'a> {
    pub reader: &'a dyn LlmClient,
    pub judge: &'a dyn LlmClient,
    /// Chunks of retrieved history the reader sees.
    pub context: usize,
    /// Model calls in flight at once.
    pub parallel: usize,
}

pub fn run(data: &Path, split: Split, kind: Option<&str>, embedder: Arc<dyn Embedder>, setup: QaSetup, limit: Option<usize>, out: &Path) -> Result<()> {
    let text = std::fs::read_to_string(data).map_err(|e| Error::msg(format!("{}: {e}", data.display())))?;
    let all: Vec<Question> = serde_json::from_str(&text)?;
    // Unlike retrieval, answer accuracy includes the abstention questions, as LongMemEval does.
    let mut questions: Vec<&Question> = all.iter().filter(|q| includes(split, &q.question_id) && kind.is_none_or(|k| q.question_type == k)).collect();
    if let Some(n) = limit {
        questions.truncate(n);
    }

    let mut done: Vec<Graded> = std::fs::read_to_string(out)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let seen: HashSet<String> = done.iter().map(|g| g.id.clone()).collect();
    let todo: Vec<&Question> = questions.iter().copied().filter(|q| !seen.contains(&q.question_id)).collect();
    eprintln!(
        "LongMemEval-S answers: {} questions ({} already in {}), reader {}, judge {}, {} chunks of context",
        questions.len(), questions.len() - todo.len(), out.display(), setup.reader.name(), setup.judge.name(), setup.context
    );

    let file = Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(out)?);
    let finished = Mutex::new(0usize);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(setup.parallel).build().map_err(|e| Error::msg(e.to_string()))?;
    let fresh: Vec<Graded> = pool.install(|| {
        todo.par_iter()
            .filter_map(|q| match grade(q, embedder.as_ref(), setup.reader, setup.judge, setup.context) {
                Ok(g) => {
                    let mut f = file.lock().unwrap();
                    let _ = writeln!(f, "{}", serde_json::to_string(&g).unwrap_or_default());
                    let mut n = finished.lock().unwrap();
                    *n += 1;
                    eprintln!("[{}/{}] {} {} {}", *n, todo.len(), g.id, g.kind, if g.correct { "correct" } else { "wrong" });
                    Some(g)
                }
                // Not saved, so the next run retries it.
                Err(e) => {
                    eprintln!("{}: {e}", q.question_id);
                    None
                }
            })
            .collect()
    });
    done.extend(fresh);
    let wanted: HashSet<&str> = questions.iter().map(|q| q.question_id.as_str()).collect();
    done.retain(|g| wanted.contains(g.id.as_str()));
    report(&done);
    if done.len() < questions.len() {
        println!("{} questions failed and were not saved; run the same command again to retry them.", questions.len() - done.len());
    }
    Ok(())
}
