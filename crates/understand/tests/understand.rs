use std::collections::{HashMap, HashSet};

use engram_core::db::open_memory;
use engram_core::events::{NewEvent, append_event, ensure_actor, list_events};
use engram_core::graph::{ExpandOpts, expand};
use engram_core::memory::{SearchOpts, get_memory, search_memories};
use engram_code::{Import, Lang, Resolver, parse_file};
use engram_understand::codeindex::{IndexResult, index_code};
use engram_understand::extract::{Candidate, parse_candidates};
use engram_understand::llm::{LlmClient, LlmRequest, ScriptedClient};
use engram_understand::notes::{NoteContext, NoteInput, record_note};
use engram_understand::pipeline::{PipelineOptions, run_pipeline};
use engram_understand::reconcile::{SaveAction, similarity};
use engram_understand::tasks::group_tasks;
use engram_understand::verify::verify_candidate;
use serde_json::json;

#[test]
fn index_code_builds_entities_and_import_edges_across_languages() {
    let root = tempfile::tempdir().unwrap();
    let w = |p: &str, body: &str| {
        let full = root.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    };
    w("perception/lidar.py", "def voxel(): pass\n");
    w("perception/fusion.py", "from perception.lidar import voxel\nclass Fusion: pass\n");
    w("core/Cargo.toml", "[package]\nname = \"drive-core\"\n");
    w("core/src/lib.rs", "pub mod plan;\npub struct Pose;\n");
    w("core/src/plan.rs", "use crate::Pose;\npub fn plan(p: Pose) {}\n");
    w("ui/app.ts", "import { plan } from './plan';\nexport function App() {}\n");
    w("ui/plan.ts", "export const plan = () => 1;\n");
    let paths: Vec<String> = ["perception/lidar.py", "perception/fusion.py", "core/src/lib.rs", "core/src/plan.rs", "ui/app.ts", "ui/plan.ts"].map(String::from).into();
    let files: HashSet<String> = paths.iter().cloned().collect();
    let manifests = vec![("core/Cargo.toml".to_string(), std::fs::read_to_string(root.path().join("core/Cargo.toml")).unwrap())];
    let db = open_memory().unwrap();
    let res = index_code(&db, root.path(), &paths, &Resolver::new(&files, &manifests), None).unwrap();
    assert_eq!(res, IndexResult { files: 6, symbols: 6, import_edges: 4 });
    let fusion: String = db.query_row("SELECT id FROM entities WHERE qualified_name = 'perception/fusion.py'", [], |r| r.get(0)).unwrap();
    let opts = ExpandOpts { max_depth: 1, edge_types: Some(vec!["imports".into()]), ..Default::default() };
    assert_eq!(expand(&db, &[fusion], &opts).unwrap().len(), 2);
    let folders: i64 = db.query_row("SELECT COUNT(*) FROM edges e JOIN entities a ON a.id = e.src JOIN entities b ON b.id = e.dst
        WHERE a.qualified_name = 'core' AND b.qualified_name = 'core/src' AND e.type = 'contains'", [], |r| r.get(0)).unwrap();
    assert_eq!(folders, 1, "folders link to their parent folder");
    let plan_rs: i64 = db.query_row("SELECT COUNT(*) FROM entities WHERE qualified_name = 'core/src/plan.rs::plan' AND kind = 'function'", [], |r| r.get(0)).unwrap();
    assert_eq!(plan_rs, 1);
    let _ = (Import::Js { specifier: String::new() }, Lang::Rust, parse_file(Lang::Python, "x = 1"));
}

#[test]
fn group_tasks_splits_on_idle_gaps() {
    let db = open_memory().unwrap();
    let at = |ts: &str| append_event(&db, NewEvent { ts: Some(ts.into()), ..NewEvent::new("file.change", json!({})) }).unwrap();
    let events = [at("2026-10-30T10:00:00Z"), at("2026-10-30T10:05:00Z"), at("2026-10-30T11:00:00Z")];
    let sizes: Vec<_> = group_tasks(&events, 10 * 60_000).iter().map(|t| t.events.len()).collect();
    assert_eq!(sizes, [2, 1]);
}

#[test]
fn similarity_is_jaccard_word_overlap() {
    assert_eq!(similarity("Use JWT tokens", "use jwt tokens"), 1.0);
    assert!(similarity("Auth strategy", "Database schema") < 0.2);
}

#[test]
fn parse_candidates_tolerates_fences_and_drops_malformed() {
    let raw = "```json\n{\"memories\":[{\"type\":\"decision\",\"title\":\"T\",\"claim\":\"C\",\"quote\":\"q\",\"evidence\":[1]},{\"type\":\"bogus\",\"title\":\"x\",\"claim\":\"y\",\"quote\":\"q\",\"evidence\":[1]},{\"type\":\"fact\",\"title\":\"no quote\",\"claim\":\"y\",\"evidence\":[1]}]}\n```";
    assert_eq!(parse_candidates(raw).len(), 1);
    assert!(parse_candidates("not json").is_empty());
    let bare = "```json\n[{\"type\":\"fact\",\"title\":\"T\",\"claim\":\"C\",\"quote\":\"q\",\"evidence\":[1]}]\n```";
    assert_eq!(parse_candidates(bare).len(), 1, "a bare list is accepted");
}

#[test]
fn grounding_accepts_real_quotes_fixes_citations_and_rejects_inventions() {
    let seen: HashMap<i64, String> = [
        (1, "commit abc: Switch to token auth because the **mobile app** cannot hold cookies".to_string()),
        (2, "user: we also decided the session store moves to Redis for horizontal scaling".to_string()),
    ].into();
    let base = Candidate { kind: "decision".into(), title: "t".into(), claim: "c".into(), rationale: None, quote: String::new(), evidence: vec![1], files: vec![], confidence: 0.9 };
    let with = |quote: &str, evidence: Vec<i64>| Candidate { quote: quote.into(), evidence, ..base.clone() };

    let exact = verify_candidate(&with("the mobile app cannot hold   COOKIES", vec![1]), &seen).unwrap();
    assert_eq!((exact.evidence, exact.exact), (vec![1], true), "markdown and case are ignored");

    let moved = verify_candidate(&with("session store moves to Redis", vec![1]), &seen).unwrap();
    assert_eq!(moved.evidence, vec![2], "a wrong citation is corrected to where the quote really is");

    let near = verify_candidate(&with("the session store moves to Redis for scaling", vec![2]), &seen).unwrap();
    assert!(!near.exact, "near-verbatim is accepted but marked");

    assert!(verify_candidate(&with("we chose a single agent for everything", vec![1]), &seen).is_err());
    assert!(verify_candidate(&with("token", vec![9]), &seen).is_ok(), "a short exact quote still counts");
    assert!(verify_candidate(&with("", vec![1]), &seen).is_err());
}

#[test]
fn record_note_logs_an_event_and_supersedes_on_a_changed_claim() {
    let db = open_memory().unwrap();
    let claude = ensure_actor(&db, "agent", "claude-code").unwrap();
    let ctx = NoteContext { actor_id: claude.id.clone(), session_id: None, commit_sha: None };
    let note = |claim: &str, files: Vec<String>| NoteInput {
        kind: Some("decision".into()),
        title: "Auth strategy".into(),
        claim: claim.into(),
        rationale: Some("why".into()),
        files,
        confidence: None,
        expires_at: None,
    };
    let (first, _) = record_note(&db, &note("Use session cookies", vec!["auth/login.py".into()]), &ctx).unwrap();
    assert_eq!(first.action.as_str(), "created");
    let (dup, _) = record_note(&db, &note("Use session cookies", vec![]), &ctx).unwrap();
    assert_eq!(dup.action, SaveAction::Reinforced);
    assert_eq!(dup.memory.reinforcement, 1);
    let (next, _) = record_note(&db, &note("Use JWT bearer tokens", vec![]), &ctx).unwrap();
    assert_eq!(next.action, SaveAction::Superseded);
    assert_eq!(get_memory(&db, &first.memory.id).unwrap().unwrap().status, "superseded");
    assert_eq!(search_memories(&db, "auth", &SearchOpts::default()).unwrap().len(), 1);
    assert_eq!(list_events(&db, 0, Some("agent.note"), 10).unwrap().len(), 3);
}

fn commit_event(db: &engram_core::Connection) {
    let payload = json!({"sha": "abc12345", "subject": "Switch lidar filter to voxel grid", "files": ["perception/lidar.py"], "author": "a"});
    append_event(db, NewEvent { ts: Some("2026-10-30T10:00:00Z".into()), commit_sha: Some("abc12345".into()), ..NewEvent::new("git.commit", payload) }).unwrap();
}

#[test]
fn pipeline_extracts_grounded_memories_and_drops_hallucinated_ones() {
    let db = open_memory().unwrap();
    commit_event(&db);
    let reply = json!({"memories": [
        {"type": "decision", "title": "Lidar downsampling", "claim": "Use a voxel grid filter", "rationale": "faster than radius filter",
         "quote": "voxel grid", "evidence": [1], "files": ["perception/lidar.py"], "confidence": 0.9},
        {"type": "decision", "title": "Invented", "claim": "x", "quote": "never said this", "evidence": [1], "files": [], "confidence": 0.9}
    ]});
    let llm = ScriptedClient::new(vec![reply.to_string()]);
    let report = run_pipeline(&db, &PipelineOptions::new(&llm)).unwrap();
    assert_eq!(report.created, 1);
    assert_eq!(report.rejected.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["Invented"]);
    assert_eq!(run_pipeline(&db, &PipelineOptions::new(&llm)).unwrap().tasks, 0, "cursor advanced");
}

struct Down;
impl LlmClient for Down {
    fn name(&self) -> String {
        "down".into()
    }
    fn complete(&self, _: &LlmRequest) -> engram_core::Result<String> {
        Err(engram_core::Error::msg("503"))
    }
}

#[test]
fn a_failing_model_leaves_the_cursor_so_the_task_is_retried() {
    let db = open_memory().unwrap();
    commit_event(&db);
    let failed = run_pipeline(&db, &PipelineOptions::new(&Down)).unwrap();
    assert!(failed.error.unwrap().contains("503"));
    let ok = run_pipeline(&db, &PipelineOptions::new(&ScriptedClient::new(vec!["{\"memories\":[]}".into()]))).unwrap();
    assert_eq!(ok.tasks, 1);
}

#[test]
fn related_notes_extend_each_other() {
    let db = open_memory().unwrap();
    let ctx = NoteContext { actor_id: ensure_actor(&db, "agent", "codex").unwrap().id, session_id: None, commit_sha: None };
    let note = |title: &str, claim: &str| NoteInput { kind: Some("decision".into()), title: title.into(), claim: claim.into(), ..Default::default() };
    let (base, _) = record_note(&db, &note("Lidar downsampling", "Downsample lidar with a voxel grid"), &ctx).unwrap();
    let (detail, _) = record_note(&db, &note("Lidar voxel size", "Voxel grid size for lidar is 0.2m"), &ctx).unwrap();
    assert_eq!(detail.action.as_str(), "created");
    assert_eq!(detail.extends.as_deref(), Some(base.memory.id.as_str()));
    let (unrelated, _) = record_note(&db, &note("Billing", "Stripe webhooks verified"), &ctx).unwrap();
    assert!(unrelated.extends.is_none());
}

#[test]
fn conversations_are_extracted_like_commits() {
    let db = open_memory().unwrap();
    let claude = ensure_actor(&db, "agent", "claude-code").unwrap();
    let turn = |role: &str, text: &str, ts: &str| {
        let payload = json!({"agent": "claude-code", "role": role, "text": text, "tools": []});
        append_event(&db, NewEvent { actor_id: Some(claude.id.clone()), ts: Some(ts.into()), ..NewEvent::new("agent.turn", payload) }).unwrap();
    };
    turn("user", "Should we keep session cookies for the mobile app?", "2026-10-30T10:00:00Z");
    turn("assistant", "No: the mobile app cannot hold cookies, so we will issue JWT bearer tokens.", "2026-10-30T10:00:05Z");
    let llm = ScriptedClient::new(vec![json!({"memories": [{"type": "decision", "title": "Mobile auth", "claim": "Mobile app uses JWT bearer tokens",
        "rationale": "it cannot hold cookies", "quote": "mobile app cannot hold cookies", "evidence": [2], "files": [], "confidence": 0.9}]}).to_string()]);
    let report = run_pipeline(&db, &PipelineOptions::new(&llm)).unwrap();
    assert_eq!((report.tasks, report.created), (1, 1));
}

#[test]
fn max_tasks_stops_early_and_leaves_the_rest_for_the_next_run() {
    let db = open_memory().unwrap();
    // Three commits an hour apart are three separate tasks.
    for hour in 10..13 {
        let payload = json!({"sha": format!("c{hour}"), "subject": "change", "files": ["a.py"], "author": "a"});
        append_event(&db, NewEvent { ts: Some(format!("2026-10-30T{hour}:00:00Z")), ..NewEvent::new("git.commit", payload) }).unwrap();
    }
    let llm = ScriptedClient::new(vec!["{\"memories\":[]}".into()]);
    let mut opts = PipelineOptions::new(&llm);
    opts.max_tasks = Some(2);
    assert_eq!(run_pipeline(&db, &opts).unwrap().tasks, 2);
    assert_eq!(run_pipeline(&db, &opts).unwrap().tasks, 1);
}

#[test]
fn weak_memories_are_suggested_with_reasons_and_archiving_is_reversible() {
    use engram_core::memory::{NewMemory, create_memory};
    use engram_understand::quality::{set_archived, weak_memories};
    let db = open_memory().unwrap();
    let make = |kind: &str, title: &str, claim: &str, rationale: Option<&str>, confidence: f64| {
        create_memory(&db, NewMemory { rationale: rationale.map(String::from), confidence: Some(confidence), ..NewMemory::new(kind, title, claim) }).unwrap()
    };
    let good = make("decision", "Session auth", "Use signed session cookies", Some("both sites share one domain"), 0.9);
    let changelog = make("decision", "Job deletion", "A delete endpoint for jobs was added", None, 1.0);
    let identifier = make("fact", "Auth0 client", "The Auth0 client ID is 'TeYnrICvXmwNhRjiGWZcCoUX12'", None, 1.0);
    let unsure = make("fact", "Maybe Redis", "The cache might be Redis", None, 0.5);
    let keep = make("fact", "Viewer controls", "The viewer has freeze, fit to view and range filters", Some("users asked for them"), 1.0);
    let dupe = make("fact", "Viewer functionality", "The viewer has freeze, fit to view and range filters", None, 1.0);

    let weak = weak_memories(&db).unwrap();
    let reason = |id: &str| weak.iter().find(|w| w.memory.id == id).map(|w| w.reason.as_str());
    assert_eq!(reason(&good.id), None);
    assert_eq!(reason(&changelog.id), Some("changelog"));
    assert_eq!(reason(&identifier.id), Some("identifier"));
    assert_eq!(reason(&unsure.id), Some("low_confidence"));
    assert_eq!((reason(&keep.id), reason(&dupe.id)), (None, Some("duplicate")), "the one with a reason is kept");

    assert_eq!(set_archived(&db, &[changelog.id.clone(), good.id.clone()], true).unwrap(), 2);
    assert_eq!(get_memory(&db, &changelog.id).unwrap().unwrap().status, "archived");
    assert_eq!(set_archived(&db, &[good.id.clone()], false).unwrap(), 1);
    assert_eq!(get_memory(&db, &good.id).unwrap().unwrap().status, "current");
}
