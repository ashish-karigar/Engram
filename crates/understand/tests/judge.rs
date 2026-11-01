//! Model-judged links: the model only labels, code decides what a label may change.

use engram_core::db::open_memory;
use engram_core::memory::{Memory, NewMemory, create_memory, get_memory};
use engram_core::{Connection, Error, Result};
use engram_understand::judge::{conflicts, decide, parse_verdicts, pending, relink};
use engram_understand::llm::{LlmClient, LlmRequest, ScriptedClient};

fn memory(conn: &Connection, title: &str, claim: &str, at: &str) -> Memory {
    create_memory(conn, NewMemory { valid_from: Some(at.into()), observed_at: Some(at.into()), ..NewMemory::new("fact", title, claim) }).unwrap()
}

fn code_fact(conn: &Connection, title: &str, claim: &str) -> Memory {
    let m = NewMemory { scope: Some("system:backend:host".into()), valid_from: Some("2026-10-01T00:00:00Z".into()), ..NewMemory::new("fact", title, claim) };
    create_memory(conn, m).unwrap()
}

/// Shows every other memory as a neighbour, in creation order.
fn everyone(conn: &Connection) -> impl Fn(&Memory) -> Vec<String> + '_ {
    move |_| {
        let mut stmt = conn.prepare("SELECT id FROM memories ORDER BY rowid").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }
}

fn reply(links: &str) -> ScriptedClient {
    ScriptedClient::new(vec![format!("{{\"links\": [{links}]}}")])
}

fn status(conn: &Connection, m: &Memory) -> String {
    get_memory(conn, &m.id).unwrap().unwrap().status
}

fn edges(conn: &Connection, kind: &str) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM edges WHERE type = ? AND valid_to IS NULL", [kind], |r| r.get(0)).unwrap()
}

#[test]
fn parser_keeps_only_listed_candidates_and_known_relations() {
    let ids = vec!["mem_a".to_string(), "mem_b".to_string()];
    let raw = r#"Sure: {"links":[{"id":"C1","relation":"extends","reason":"same cache","confidence":0.9},
        {"id":"C7","relation":"extends","reason":"not shown","confidence":0.9},
        {"id":"C2","relation":"merges","reason":"unknown","confidence":0.9},
        {"id":"C1","relation":"same","reason":"second label for C1","confidence":0.9},
        {"id":"C2","relation":"depends_on","reason":"","confidence":0.9}]}"#;
    let v = parse_verdicts(raw, &ids);
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].dst.as_str(), v[0].relation.as_str()), ("mem_a", "extends"));
    assert!(parse_verdicts("no json here", &ids).is_empty());
}

#[test]
fn a_confident_replacement_retires_the_older_memory_as_history() {
    let conn = open_memory().unwrap();
    let old = memory(&conn, "Backend hosting", "The backend runs on Oracle Cloud.", "2026-09-01T00:00:00Z");
    let new = memory(&conn, "Infra move", "We moved the API servers to AWS.", "2026-10-20T00:00:00Z");
    // `new` is judged first (newest first) and sees `old` as C1.
    let llm = reply(r#"{"id":"C1","relation":"replaces","reason":"hosting moved from Oracle to AWS","confidence":0.9}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.applied, report.pending), (1, 0));
    assert_eq!(status(&conn, &old), "superseded");
    assert_eq!(status(&conn, &new), "current");
    assert_eq!(edges(&conn, "supersedes"), 1);
    // Nothing was deleted, and the pair is never judged again.
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(relink(&conn, &llm, &everyone(&conn), 10).unwrap().calls, 0);
}

#[test]
fn an_unsure_link_waits_for_review_and_changes_nothing_until_confirmed() {
    let conn = open_memory().unwrap();
    let old = memory(&conn, "Cache", "Search results are cached for 60 seconds.", "2026-09-01T00:00:00Z");
    let new = memory(&conn, "Cache size", "The search cache holds 500 entries.", "2026-10-01T00:00:00Z");
    let llm = reply(r#"{"id":"C1","relation":"extends","reason":"both describe the search cache","confidence":0.65}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.applied, report.pending), (0, 1));
    assert_eq!(edges(&conn, "extends"), 0);

    let waiting = pending(&conn, 10).unwrap();
    assert_eq!(waiting.len(), 1);
    decide(&conn, &waiting[0].id, "approve", None).unwrap();
    assert_eq!(edges(&conn, "extends"), 1);
    assert!(pending(&conn, 10).unwrap().is_empty());
    assert_eq!((status(&conn, &old), status(&conn, &new)), ("current".into(), "current".into()));
}

#[test]
fn weak_links_are_dropped_and_lonely_memories_stay_alone() {
    let conn = open_memory().unwrap();
    memory(&conn, "Logging", "Logs go to stdout as JSON.", "2026-09-01T00:00:00Z");
    memory(&conn, "Fonts", "The UI uses the system font.", "2026-10-01T00:00:00Z");
    let llm = reply(r#"{"id":"C1","relation":"extends","reason":"both are settings","confidence":0.3}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.applied, report.pending), (0, 0));
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn a_memory_that_disagrees_with_the_code_is_flagged_never_applied() {
    let conn = open_memory().unwrap();
    let fact = code_fact(&conn, "backend hosting", "backend is hosted on Oracle.");
    let said = memory(&conn, "Hosting", "The backend now runs on AWS.", "2026-10-20T00:00:00Z");
    // Even a confident "replaces" against the code becomes a conflict to review.
    let llm = reply(r#"{"id":"C1","relation":"replaces","reason":"Oracle versus AWS hosting","confidence":0.95}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.applied, report.pending), (0, 1));
    assert_eq!((status(&conn, &fact), status(&conn, &said)), ("current".into(), "current".into()));
    assert_eq!(edges(&conn, "contradicts"), 1);
    assert!(conflicts(&conn).unwrap()[0].2.contains("conflicts with the code"));

    let link = &pending(&conn, 10).unwrap()[0];
    assert!(link.dst_from_code);
    assert!(decide(&conn, &link.id, "approve", None).is_err(), "a conflict needs a side chosen");
    assert!(decide(&conn, &link.id, "keep", Some(&said.id)).is_err(), "the code's fact can't be dropped");
    decide(&conn, &link.id, "keep", Some(&fact.id)).unwrap();
    assert_eq!(status(&conn, &said), "superseded");
    assert_eq!(status(&conn, &fact), "current");
    assert_eq!(edges(&conn, "contradicts"), 0);
    assert!(conflicts(&conn).unwrap().is_empty());
}

#[test]
fn a_duplicate_reinforces_the_first_statement() {
    let conn = open_memory().unwrap();
    let first = memory(&conn, "Auth provider", "Both apps sign users in with Auth0.", "2026-09-01T00:00:00Z");
    let repeat = memory(&conn, "Login", "Auth0 handles login for frontend and backend.", "2026-10-01T00:00:00Z");
    let llm = reply(r#"{"id":"C1","relation":"same","reason":"both say Auth0 handles sign-in","confidence":0.9}"#);
    relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!(status(&conn, &repeat), "archived");
    let kept = get_memory(&conn, &first.id).unwrap().unwrap();
    assert_eq!((kept.status.as_str(), kept.reinforcement), ("current", 1));
}

struct Down;
impl LlmClient for Down {
    fn name(&self) -> String {
        "down".into()
    }
    fn complete(&self, _req: &LlmRequest) -> Result<String> {
        Err(Error::msg("503 service unavailable"))
    }
}

#[test]
fn a_failed_call_changes_nothing_and_is_retried_later() {
    let conn = open_memory().unwrap();
    memory(&conn, "A", "First fact about the queue.", "2026-09-01T00:00:00Z");
    memory(&conn, "B", "Second fact about the queue.", "2026-10-01T00:00:00Z");
    let report = relink(&conn, &Down, &everyone(&conn), 10).unwrap();
    assert!(report.error.is_some());
    assert_eq!(report.judged, 0);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM memories_judged", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    // Later, with the model back, both get judged.
    assert_eq!(relink(&conn, &reply(""), &everyone(&conn), 10).unwrap().judged, 2);
}

#[test]
fn a_changed_code_fact_flags_the_notes_it_makes_outdated() {
    let conn = open_memory().unwrap();
    let note = memory(&conn, "Hosting", "The backend runs on Oracle Cloud.", "2026-09-01T00:00:00Z");
    // Judge the note first, alone, as would have happened before the code changed.
    relink(&conn, &reply(""), &|_: &Memory| vec![], 10).unwrap();
    // The map later derives a new fact from the code; it is judged against the note.
    let fact = code_fact(&conn, "backend hosting", "backend is hosted on AWS.");
    let llm = reply(r#"{"id":"C1","relation":"replaces","reason":"hosting is AWS in the code, Oracle in the note","confidence":0.9}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.calls, report.applied, report.pending), (1, 0, 1));
    assert_eq!(status(&conn, &note), "current", "a person decides, not the model");
    let flagged = conflicts(&conn).unwrap();
    assert_eq!(flagged[0].0, note.id);
    assert!(flagged[0].2.contains("conflicts with the code"));
    let link = &pending(&conn, 10).unwrap()[0];
    assert!(link.src_from_code);
    decide(&conn, &link.id, "keep", Some(&fact.id)).unwrap();
    assert_eq!(status(&conn, &note), "superseded");
}

fn with_commit(conn: &Connection, claim: &str, at: &str) -> Memory {
    let m = NewMemory { valid_from: Some(at.into()), commit_sha: Some("6fdc441".into()), ..NewMemory::new("decision", "Python environment", claim) };
    create_memory(conn, m).unwrap()
}

#[test]
fn chat_alone_cannot_retire_what_a_commit_backs() {
    let conn = open_memory().unwrap();
    // The LiTheR case: the commit moved to Conda; a later chat memory claims venv.
    let conda = with_commit(&conn, "Migrate the desktop setup to Conda.", "2026-10-27T00:00:00Z");
    let venv = memory(&conn, "Python environment", "The project uses the built-in venv, not Conda.", "2026-10-29T00:00:00Z");
    let llm = reply(r#"{"id":"C1","relation":"replaces","reason":"venv versus Conda","confidence":0.95}"#);
    let report = relink(&conn, &llm, &everyone(&conn), 10).unwrap();
    assert_eq!((report.applied, report.pending), (0, 1));
    assert_eq!((status(&conn, &conda), status(&conn, &venv)), ("current".into(), "current".into()));
}

#[test]
fn a_memory_in_conflict_with_the_code_retires_nothing() {
    let conn = open_memory().unwrap();
    code_fact(&conn, "backend data", "backend stores data in PostgreSQL.");
    let overview = memory(&conn, "Overview", "Angular frontend, FastAPI backend, data in PostgreSQL.", "2026-10-01T00:00:00Z");
    let sqlite = memory(&conn, "Database", "The database URL is sqlite:///./app.db.", "2026-10-20T00:00:00Z");
    // `sqlite` sees `overview` (C1) and the code fact (C2); the conflict is recorded first.
    let llm = reply(
        r#"{"id":"C1","relation":"replaces","reason":"SQLite versus PostgreSQL","confidence":0.95},
           {"id":"C2","relation":"contradicts","reason":"code says PostgreSQL","confidence":0.95}"#,
    );
    let report = relink(&conn, &llm, &everyone(&conn), 1).unwrap();
    assert_eq!((report.applied, report.pending), (0, 2));
    assert_eq!(status(&conn, &overview), "current");
    assert_eq!(status(&conn, &sqlite), "current");
}

#[test]
fn applied_links_can_be_undone_and_old_mistakes_are_reopened() {
    let conn = open_memory().unwrap();
    let old = memory(&conn, "Queue", "Jobs run on Redis.", "2026-09-01T00:00:00Z");
    let new = memory(&conn, "Queue", "Jobs moved to Postgres.", "2026-10-01T00:00:00Z");
    relink(&conn, &reply(r#"{"id":"C1","relation":"replaces","reason":"queue moved","confidence":0.95}"#), &everyone(&conn), 10).unwrap();
    assert_eq!(status(&conn, &old), "superseded");
    let id: String = conn.query_row("SELECT id FROM link_judgments", [], |r| r.get(0)).unwrap();
    decide(&conn, &id, "undo", None).unwrap();
    assert_eq!((status(&conn, &old), status(&conn, &new)), ("current".into(), "current".into()));
    assert_eq!(edges(&conn, "supersedes"), 0);
    assert!(decide(&conn, &id, "undo", None).is_err(), "already undone");

    // A replacement applied before the evidence rule existed is reopened on the next pass.
    let conn = open_memory().unwrap();
    let conda = with_commit(&conn, "Migrate the desktop setup to Conda.", "2026-10-27T00:00:00Z");
    let venv = memory(&conn, "Python environment", "The project uses venv.", "2026-10-29T00:00:00Z");
    conn.execute("UPDATE memories SET status = 'superseded' WHERE id = ?", [&conda.id]).unwrap();
    conn.execute(
        "INSERT INTO link_judgments (id, src, dst, relation, reason, confidence, judged_by, status, created_at)
         VALUES ('lnk_old', ?, ?, 'replaces', 'venv versus Conda', 0.95, 'test', 'applied', '2026-10-31')",
        [&venv.id, &conda.id],
    )
    .unwrap();
    let report = relink(&conn, &reply(""), &|_: &Memory| vec![], 10).unwrap();
    assert_eq!(report.reopened, 1);
    assert_eq!(status(&conn, &conda), "current");
    assert_eq!(pending(&conn, 10).unwrap()[0].id, "lnk_old");
}

#[test]
fn undoing_one_replacement_keeps_a_memory_another_still_replaces() {
    let conn = open_memory().unwrap();
    let pg = with_commit(&conn, "Data moved to PostgreSQL.", "2026-10-12T00:00:00Z");
    let pg_again = with_commit(&conn, "PostgreSQL stores all backend data.", "2026-10-20T00:00:00Z");
    // Judged last-created first: the SQLite memory sees both newer ones (C1, C2) in one call.
    let sqlite = with_commit(&conn, "The database is SQLite.", "2026-10-03T00:00:00Z");
    let llm = reply(
        r#"{"id":"C1","relation":"replaces","reason":"database changed","confidence":0.95},
           {"id":"C2","relation":"replaces","reason":"database changed","confidence":0.95}"#,
    );
    let ids = vec![pg.id.clone(), pg_again.id.clone()];
    relink(&conn, &llm, &move |_: &Memory| ids.clone(), 1).unwrap();
    assert_eq!(status(&conn, &sqlite), "superseded");
    assert_eq!(edges(&conn, "supersedes"), 2);

    let first: String = conn.query_row("SELECT id FROM link_judgments WHERE dst = ?", [&pg.id], |r| r.get(0)).unwrap();
    decide(&conn, &first, "undo", None).unwrap();
    assert_eq!(status(&conn, &sqlite), "superseded", "still replaced by the other link");
    let second: String = conn.query_row("SELECT id FROM link_judgments WHERE dst = ?", [&pg_again.id], |r| r.get(0)).unwrap();
    decide(&conn, &second, "undo", None).unwrap();
    assert_eq!(status(&conn, &sqlite), "current");
}

#[test]
fn reasons_never_show_the_prompt_labels() {
    let ids = vec!["mem_a".to_string()];
    let v = parse_verdicts(r#"{"links":[{"id":"C1","relation":"extends","reason":"M picks SQLite while C4 says Postgres","confidence":0.9}]}"#, &ids);
    assert_eq!(v[0].reason, "one picks SQLite while the other says Postgres");
}
