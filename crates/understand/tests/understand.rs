use std::collections::{HashMap, HashSet};

use engram_core::db::open_memory;
use engram_core::events::{NewEvent, append_event, ensure_actor, list_events};
use engram_core::graph::{ExpandOpts, expand};
use engram_core::memory::{SearchOpts, get_memory, search_memories};
use engram_understand::codeindex::{IndexResult, index_python};
use engram_understand::extract::{Candidate, parse_candidates};
use engram_understand::llm::{LlmClient, LlmRequest, ScriptedClient};
use engram_understand::notes::{NoteContext, NoteInput, record_note};
use engram_understand::pipeline::{PipelineOptions, run_pipeline};
use engram_understand::pyindex::{PyImport, parse_python, resolve_import};
use engram_understand::reconcile::{SaveAction, similarity};
use engram_understand::tasks::group_tasks;
use engram_understand::verify::verify_candidate;
use serde_json::json;

#[test]
fn tree_sitter_extracts_symbols_and_imports() {
    let idx = parse_python(
        "import os\nimport numpy as np\nfrom perception.lidar import voxel\nfrom . import utils\nfrom ..common.geometry import Pose\n\nclass Fusion:\n    def update(self): pass\n\ndef run(): pass\n",
    )
    .unwrap();
    let symbols: Vec<_> = idx.symbols.iter().map(|s| format!("{}:{}", s.kind, s.name)).collect();
    assert_eq!(symbols, ["class:Fusion", "method:update", "function:run"]);
    let imports: Vec<_> = idx.imports.iter().map(|i| format!("{}{}", ".".repeat(i.level), i.module)).collect();
    assert_eq!(imports, ["os", "numpy", "perception.lidar", ".", "..common.geometry"]);
}

#[test]
fn imports_resolve_to_project_files_only() {
    let files: HashSet<String> =
        ["perception/lidar.py", "perception/fusion.py", "common/geometry.py", "perception/__init__.py"].map(String::from).into();
    let imp = |m: &str, level| PyImport { module: m.into(), level };
    assert_eq!(resolve_import(&imp("perception.lidar", 0), "main.py", &files).as_deref(), Some("perception/lidar.py"));
    assert_eq!(resolve_import(&imp("lidar", 1), "perception/fusion.py", &files).as_deref(), Some("perception/lidar.py"));
    assert_eq!(resolve_import(&imp("common.geometry", 3), "perception/sub/x.py", &files).as_deref(), Some("common/geometry.py"));
    assert_eq!(resolve_import(&imp("numpy", 0), "main.py", &files), None);
}

#[test]
fn index_python_builds_entities_and_import_edges() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("perception")).unwrap();
    std::fs::write(root.path().join("perception/lidar.py"), "def voxel(): pass\n").unwrap();
    std::fs::write(root.path().join("perception/fusion.py"), "from perception.lidar import voxel\nclass Fusion: pass\n").unwrap();
    let db = open_memory().unwrap();
    let all: HashSet<String> = ["perception/lidar.py", "perception/fusion.py"].map(String::from).into();
    let paths: Vec<String> = all.iter().cloned().collect();
    let res = index_python(&db, root.path(), &paths, &all, None).unwrap();
    assert_eq!(res, IndexResult { files: 2, symbols: 2, import_edges: 1 });
    let fusion: String = db.query_row("SELECT id FROM entities WHERE qualified_name = 'perception/fusion.py'", [], |r| r.get(0)).unwrap();
    let opts = ExpandOpts { max_depth: 1, edge_types: Some(vec!["imports".into()]), ..Default::default() };
    assert_eq!(expand(&db, &[fusion], &opts).unwrap().len(), 2);
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
}

#[test]
fn verify_requires_the_quote_in_the_cited_evidence() {
    let seen: HashMap<i64, String> = [(1, "commit abc: Switch to token auth because mobile app cannot hold cookies".to_string())].into();
    let base = Candidate { kind: "decision".into(), title: "t".into(), claim: "c".into(), rationale: None, quote: String::new(), evidence: vec![1], files: vec![], confidence: 0.9 };
    let with = |quote: &str, evidence: Vec<i64>| Candidate { quote: quote.into(), evidence, ..base.clone() };
    assert!(verify_candidate(&with("cannot hold   COOKIES", vec![1]), &seen).is_ok());
    assert!(verify_candidate(&with("made up", vec![1]), &seen).is_err());
    assert!(verify_candidate(&with("token", vec![9]), &seen).is_err());
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
