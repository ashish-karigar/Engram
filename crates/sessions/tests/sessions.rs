use std::io::Write;
use std::path::Path;

use engram_core::db::open_memory;
use engram_core::events::list_events;
use engram_embed::HashEmbedder;
use engram_rag::document::list_documents;
use engram_rag::search_chunks;
use engram_sessions::source::claude_slug;
use engram_sessions::{Role, Sources, claude, codex, muse, sync_sessions};
use serde_json::json;

fn claude_lines(cwd: &str) -> Vec<String> {
    [
        json!({"type": "user", "sessionId": "s1", "timestamp": "2026-10-30T10:00:00Z", "cwd": cwd, "message": {"role": "user", "content": "Why do we use a voxel grid for lidar?"}}),
        json!({"type": "assistant", "sessionId": "s1", "timestamp": "2026-10-30T10:00:05Z", "cwd": cwd, "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "hidden"},
            {"type": "text", "text": "Radius outlier removal cost 80ms per frame, so we switched to a 0.2m voxel grid."},
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "perception/lidar.py"}}
        ]}}),
        json!({"type": "user", "sessionId": "s1", "timestamp": "2026-10-30T10:00:06Z", "cwd": cwd, "message": {"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}}),
        json!({"type": "user", "sessionId": "s1", "timestamp": "2026-10-30T10:00:07Z", "cwd": cwd, "isMeta": true, "message": {"role": "user", "content": "meta"}}),
        json!({"type": "user", "sessionId": "s1", "timestamp": "2026-10-30T10:00:08Z", "cwd": cwd, "message": {"role": "user", "content": "<system-reminder>injected</system-reminder>"}}),
        json!({"type": "user", "sessionId": "s1", "timestamp": "2026-10-30T11:00:00Z", "cwd": cwd, "isCompactSummary": true, "message": {"role": "user", "content": "summary"}}),
    ].iter().map(|v| v.to_string()).collect()
}

#[test]
fn claude_lines_become_turns_with_tools_and_compaction() {
    let turns: Vec<_> = claude_lines("/p").iter().filter_map(|l| claude::parse_line(l)).collect();
    assert_eq!(turns.len(), 3, "tool results, meta and injected text are skipped");
    assert_eq!(turns[0].role, Role::User);
    assert!(turns[1].text.contains("80ms") && !turns[1].text.contains("hidden"), "thinking is not stored");
    assert_eq!(turns[1].tools[0].files, ["perception/lidar.py"]);
    assert!(turns[2].compaction);
}

#[test]
fn codex_lines_become_turns() {
    let lines = [
        json!({"timestamp": "2026-10-30T10:00:00Z", "type": "session_meta", "payload": {"id": "cx1", "cwd": "/p"}}),
        json!({"timestamp": "2026-10-30T10:00:01Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>cwd</environment_context>"}]}}),
        json!({"timestamp": "2026-10-30T10:00:02Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Add billing page"}]}}),
        json!({"timestamp": "2026-10-30T10:00:03Z", "type": "response_item", "payload": {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "rules"}]}}),
        json!({"timestamp": "2026-10-30T10:00:04Z", "type": "response_item", "payload": {"type": "custom_tool_call", "name": "apply_patch", "input": "*** Begin Patch\n*** Add File: web/billing.py\n+x\n*** End Patch"}}),
        json!({"timestamp": "2026-10-30T10:00:05Z", "type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Billing page added; Stripe webhooks verified first."}]}}),
        json!({"timestamp": "2026-10-30T10:00:06Z", "type": "compacted", "payload": {"message": "x"}}),
    ];
    let first = lines[0].to_string();
    assert_eq!(codex::session_id_of(&first).as_deref(), Some("cx1"));
    assert_eq!(codex::cwd_of(&first).as_deref(), Some("/p"));
    let turns: Vec<_> = lines.iter().filter_map(|l| codex::parse_line(&l.to_string(), "cx1")).collect();
    assert_eq!(turns.len(), 4);
    assert_eq!(turns[0].text, "Add billing page");
    assert_eq!(turns[1].tools[0].files, ["web/billing.py"]);
    assert!(turns[3].compaction);
}

#[test]
fn muse_frames_and_run_events_become_turns() {
    let rec = |kind: &str, event: serde_json::Value| json!({"stream": {"kind": "session", "id": "mu1"}, "recorded_at": 1793403000000000i64, "payload": {"kind": kind, "event": event}});
    let facts = json!({"stream": {"id": "mu1"}, "payload": {"kind": "route_facts", "record": {"cwd": "/p"}}});
    let frame = json!({"children": [
        {"child_index": 0, "record_json": rec("run", json!({"kind": "started", "prompt": "Plan the camera pipeline"})).to_string()},
        {"child_index": 1, "record_json": rec("run", json!({"kind": "assistant_message_committed", "text": "Fuse camera detections onto lidar clusters."})).to_string()},
    ]});
    let tools = rec("run", json!({"kind": "assistant_tool_calls_committed", "tool_calls": [{"name": "edit", "args": "{\"path\": \"perception/camera.py\"}"}]}));
    assert_eq!(muse::cwd_of(&facts).as_deref(), Some("/p"));
    let turns: Vec<_> = [frame, tools].iter().flat_map(|l| muse::records(&l.to_string())).filter_map(|r| muse::parse_record(&r)).collect();
    assert_eq!(turns.len(), 3);
    assert_eq!((turns[0].role, turns[0].text.as_str()), (Role::User, "Plan the camera pipeline"));
    assert!(turns[0].ts.starts_with("2026-"));
    assert_eq!(turns[2].tools[0].files, ["perception/camera.py"]);
}

#[test]
fn a_session_across_projects_gives_each_only_its_own_turns() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (a.path().canonicalize().unwrap(), b.path().canonicalize().unwrap());
    let parent = a.parent().unwrap().to_path_buf();
    assert_eq!(b.parent(), Some(parent.as_path()), "both projects sit side by side");
    let sources = Sources { claude: home.path().join("claude"), codex: home.path().join("codex"), muse: home.path().join("muse") };

    // One long session started in the parent folder: planning there, work in A, then work in B.
    let s = |cwd: &Path, i: usize| claude_lines(&cwd.to_string_lossy())[i].replace("\"s1\"", "\"s3\"");
    write(&sources.claude.join(claude_slug(&parent)).join("s3.jsonl"), &[s(&parent, 0), s(&a, 1), s(&b, 0)]);

    let e = HashEmbedder::default();
    let (in_a, in_b) = (open_memory().unwrap(), open_memory().unwrap());
    assert_eq!(sync_sessions(&in_a, &e, &a, &sources).unwrap().turns, 1, "only the turn run inside A");
    assert_eq!(sync_sessions(&in_b, &e, &b, &sources).unwrap().turns, 1, "only the turn run inside B");
}

fn write(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

#[test]
fn sync_is_incremental_attributed_scrubbed_and_searchable() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cwd = root_path.to_string_lossy().to_string();
    let sources = Sources { claude: home.path().join("claude"), codex: home.path().join("codex"), muse: home.path().join("muse") };

    let log = sources.claude.join(claude_slug(&root_path)).join("s1.jsonl");
    write(&log, &claude_lines(&cwd)[..2]);
    let other = sources.claude.join(claude_slug(Path::new("/elsewhere"))).join("s9.jsonl");
    write(&other, &claude_lines("/elsewhere"));
    // A session started in the parent folder that later worked only inside this project counts whole.
    let parent = root_path.parent().unwrap();
    let parent_log = sources.claude.join(claude_slug(parent)).join("s2.jsonl");
    let mut outside = claude_lines(&parent.to_string_lossy())[..1].to_vec();
    outside[0] = outside[0].replace("\"s1\"", "\"s2\"");
    let mut inside = claude_lines(&cwd)[1..2].to_vec();
    inside[0] = inside[0].replace("\"s1\"", "\"s2\"");
    write(&parent_log, &[outside, inside].concat());
    write(&sources.codex.join("2026/10/30/rollout-a.jsonl"), &[
        json!({"timestamp": "2026-10-30T12:00:00Z", "type": "session_meta", "payload": {"id": "cx1", "cwd": cwd}}).to_string(),
        json!({"timestamp": "2026-10-30T12:00:01Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "token: ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa is set"}]}}).to_string(),
    ]);

    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    let first = sync_sessions(&db, &e, &root_path, &sources).unwrap();
    assert_eq!(first.turns, 5, "two Claude turns, two from a parent-folder session, one Codex turn; other projects ignored");
    assert_eq!(sync_sessions(&db, &e, &root_path, &sources).unwrap().turns, 0, "nothing new, nothing re-read");

    // A half-written line is left for the next sync.
    let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    write!(f, "{}", claude_lines(&cwd)[5]).unwrap();
    assert_eq!(sync_sessions(&db, &e, &root_path, &sources).unwrap().compactions, 0);
    writeln!(f).unwrap();
    assert_eq!(sync_sessions(&db, &e, &root_path, &sources).unwrap().compactions, 1);

    let turns = list_events(&db, 0, Some("agent.turn"), 100).unwrap();
    let actors: std::collections::HashSet<String> = turns.iter().map(|t| t.payload["agent"].as_str().unwrap().to_string()).collect();
    assert_eq!(actors, ["claude-code".to_string(), "codex".to_string()].into());
    assert!(!turns.iter().any(|t| t.payload["text"].as_str().unwrap().contains("ghp_")), "secrets are scrubbed");

    let docs = list_documents(&db, Some("session"), 10).unwrap();
    assert_eq!(docs.len(), 3);
    assert!(docs.iter().any(|d| d.title.starts_with("Claude Code session · 2026-10-30") && d.metadata["topic"].as_str().unwrap().starts_with("Why do we use a voxel grid")));
    let hits = search_chunks(&db, Some(&e), "why voxel grid instead of radius outlier", 3).unwrap();
    assert!(hits[0].content.contains("80ms per frame"), "{:?}", hits.iter().map(|h| &h.content).collect::<Vec<_>>());
}

#[test]
fn tool_only_turns_fold_into_the_spoken_turn() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cwd = root_path.to_string_lossy().to_string();
    let sources = Sources { claude: home.path().join("claude"), codex: home.path().join("c"), muse: home.path().join("m") };
    let line = |content: serde_json::Value, ts: &str| json!({"type": "assistant", "sessionId": "s1", "timestamp": ts, "cwd": cwd, "message": {"role": "assistant", "content": content}}).to_string();
    write(&sources.claude.join(claude_slug(&root_path)).join("s1.jsonl"), &[
        line(json!([{"type": "text", "text": "I will update the lidar filter."}]), "2026-10-30T10:00:00Z"),
        line(json!([{"type": "tool_use", "name": "Edit", "input": {"file_path": "lidar.py"}}]), "2026-10-30T10:00:01Z"),
        line(json!([{"type": "tool_use", "name": "Bash", "input": {"command": "pytest"}}]), "2026-10-30T10:00:02Z"),
    ]);
    let db = open_memory().unwrap();
    sync_sessions(&db, &HashEmbedder::default(), &root_path, &sources).unwrap();
    let docs = list_documents(&db, Some("session"), 10).unwrap();
    assert_eq!(docs[0].chunk_count, 1, "tool-only turns do not become chunks");
    let content: String = db.query_row("SELECT content FROM chunks", [], |r| r.get(0)).unwrap();
    assert!(content.contains("I will update the lidar filter.") && content.contains("Edit lidar.py; Bash"));
}

#[test]
fn codex_reviewer_sessions_are_skipped_and_earlier_ones_hidden() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cwd = root_path.to_string_lossy().to_string();
    let sources = Sources { claude: home.path().join("c"), codex: home.path().join("codex"), muse: home.path().join("m") };
    let review = sources.codex.join("2026/10/30/rollout-r.jsonl");
    let user = |text: &str| json!({"timestamp": "2026-10-30T12:00:01Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}}).to_string();
    let review_meta = json!({"timestamp": "2026-10-30T12:00:00Z", "type": "session_meta",
        "payload": {"id": "rv1", "cwd": cwd, "thread_source": "guardian_review", "source": {"subagent": {"other": "guardian"}}}}).to_string();
    write(&review, &[review_meta.clone(), user("The following is the Codex agent history")]);
    assert!(codex::is_review(&review_meta));

    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    assert_eq!(sync_sessions(&db, &e, &root_path, &sources).unwrap().turns, 0, "reviewer logs are not read");

    // A reviewer session synced by an older version: hidden once, its transcript removed.
    engram_core::meta::set_meta(&db, &format!("sessions.offset:{}", review.to_string_lossy()), "10").unwrap();
    engram_core::meta::set_meta(&db, &format!("sessions.sid:{}", review.to_string_lossy()), "rv1").unwrap();
    sync_sessions(&db, &e, &root_path, &sources).unwrap();
    sync_sessions(&db, &e, &root_path, &sources).unwrap();
    let hidden = list_events(&db, 0, Some("agent.session_hidden"), 10).unwrap();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].payload["session"], "rv1");
}
