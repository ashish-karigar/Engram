use chrono::{TimeZone, Utc};
use engram_core::access::{AccessRecord, log_access, recent_access};
use engram_core::db::open_memory;
use engram_core::decay::{archive_stale, strength};
use engram_core::events::*;
use engram_core::graph::*;
use engram_core::memory::*;
use serde_json::json;

#[test]
fn event_log_is_append_only_and_hash_chained() {
    let db = open_memory().unwrap();
    let a = append_event(&db, NewEvent::new("file.change", json!({"path": "a.py"}))).unwrap();
    let b = append_event(&db, NewEvent::new("git.commit", json!({"sha": "abc"}))).unwrap();
    assert_eq!(b.seq, a.seq + 1);
    assert_eq!(verify_chain(&db).unwrap(), None);
    assert!(db.execute("UPDATE events SET kind = 'x'", []).unwrap_err().to_string().contains("append-only"));
    assert!(db.execute("DELETE FROM events", []).unwrap_err().to_string().contains("append-only"));
}

#[test]
fn append_event_is_idempotent_on_op_id() {
    let db = open_memory().unwrap();
    let mk = |v| NewEvent { op_id: Some("op-1".into()), ..NewEvent::new("note", json!(v)) };
    let first = append_event(&db, mk(1)).unwrap();
    let again = append_event(&db, mk(2)).unwrap();
    assert_eq!(first.seq, again.seq);
    assert_eq!(list_events(&db, 0, None, 100).unwrap().len(), 1);
}

#[test]
fn tampering_with_the_log_is_detected() {
    let db = open_memory().unwrap();
    append_event(&db, NewEvent::new("note", json!({"v": 1}))).unwrap();
    let e2 = append_event(&db, NewEvent::new("note", json!({"v": 2}))).unwrap();
    db.execute_batch("DROP TRIGGER events_no_update").unwrap();
    db.execute("UPDATE events SET payload = '{\"v\":99}' WHERE seq = ?", [e2.seq]).unwrap();
    assert_eq!(verify_chain(&db).unwrap(), Some(e2.seq));
}

#[test]
fn supersede_keeps_history_and_moves_validity() {
    let db = open_memory().unwrap();
    let mut old = NewMemory::new("decision", "Auth", "Use session cookies for auth");
    old.rationale = Some("Simple, server-rendered".into());
    let old = create_memory_at(&db, old, "2026-10-27T10:00:00Z").unwrap();
    let next = NewMemory::new("decision", "Auth", "Use JWT bearer tokens for auth");
    let next = supersede_at(&db, &old.id, next, "2026-10-30T09:00:00Z").unwrap();
    assert_eq!(next.version, 2);
    assert_eq!(next.status, "current");
    let old_now = get_memory(&db, &old.id).unwrap().unwrap();
    assert_eq!(old_now.status, "superseded");
    assert_eq!(old_now.valid_to.as_deref(), Some("2026-10-30T09:00:00Z"));
    let claims: Vec<_> = history(&db, &next.id).unwrap().into_iter().map(|m| m.claim).collect();
    assert_eq!(claims, ["Use session cookies for auth", "Use JWT bearer tokens for auth"]);
    let err = supersede(&db, &old.id, NewMemory::new("decision", "x", "y")).unwrap_err();
    assert!(err.to_string().contains("superseded"));
}

#[test]
fn search_defaults_to_current_and_honours_as_of() {
    let db = open_memory().unwrap();
    let old = create_memory_at(&db, NewMemory::new("decision", "Auth", "Sessions store login state in cookies"), "2026-10-27T10:00:00Z").unwrap();
    supersede_at(&db, &old.id, NewMemory::new("decision", "Auth", "Tokens replace cookies for login"), "2026-10-30T09:00:00Z").unwrap();

    let now = search_memories(&db, "cookies login", &SearchOpts::default()).unwrap();
    assert_eq!(now.len(), 1);
    assert!(now[0].memory.claim.contains("Tokens"));

    let past = SearchOpts { include_history: true, as_of: Some("2026-10-28T00:00:00Z".into()), ..Default::default() };
    let hits = search_memories(&db, "cookies login", &past).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].memory.claim.contains("Sessions"));
}

#[test]
fn search_ignores_punctuation_and_question_words() {
    let db = open_memory().unwrap();
    create_memory(&db, NewMemory::new("fact", "LiDAR rate", "Point cloud arrives at 10 Hz")).unwrap();
    assert_eq!(search_memories(&db, "lidar \"rate\" AND (10Hz) -- ;", &SearchOpts::default()).unwrap().len(), 1);
    assert!(search_memories(&db, "!!!", &SearchOpts::default()).unwrap().is_empty());
    assert_eq!(search_memories(&db, "why is the lidar rate", &SearchOpts::default()).unwrap().len(), 1);
}

#[test]
fn expand_walks_to_neighbours_and_decays_with_distance() {
    let db = open_memory().unwrap();
    let lidar = upsert_entity(&db, "module", "perception.lidar", None, None).unwrap();
    let fusion = upsert_entity(&db, "module", "perception.fusion", None, None).unwrap();
    let planner = upsert_entity(&db, "module", "planning.planner", None, None).unwrap();
    add_edge(&db, &lidar.id, &fusion.id, "feeds", EdgeOpts::default()).unwrap();
    add_edge(&db, &fusion.id, &planner.id, "feeds", EdgeOpts::default()).unwrap();
    let mut m = NewMemory::new("decision", "Voxel size", "Downsample lidar with 0.2m voxels");
    m.touches = vec![lidar.id.clone()];
    let m = create_memory(&db, m).unwrap();

    let deep = ExpandOpts { max_depth: 3, ..Default::default() };
    let reached = expand(&db, &[m.id.clone()], &deep).unwrap();
    let find = |id: &str| reached.iter().find(|r| r.id == id).unwrap();
    assert_eq!(find(&lidar.id).depth, 1);
    assert_eq!(find(&fusion.id).depth, 2);
    assert_eq!(find(&planner.id).depth, 3);
    assert!(find(&lidar.id).score > find(&planner.id).score);

    let shallow = expand(&db, &[m.id.clone()], &ExpandOpts { max_depth: 1, ..Default::default() }).unwrap();
    assert_eq!(shallow.len(), 2);

    let only_feeds = ExpandOpts { max_depth: 3, edge_types: Some(vec!["feeds".into()]), ..Default::default() };
    assert!(!expand(&db, &[lidar.id.clone()], &only_feeds).unwrap().iter().any(|r| r.id == m.id));
}

#[test]
fn memories_follow_an_entity_through_a_rename() {
    let db = open_memory().unwrap();
    let before = upsert_entity(&db, "file", "auth/session.py", Some("auth/session.py"), None).unwrap();
    let mut m = NewMemory::new("decision", "Cookies", "Sessions use signed cookies");
    m.touches = vec![before.id.clone()];
    let m = create_memory(&db, m).unwrap();
    let after = rename_entity(&db, &before.id, "auth/cookies.py", Some("auth/cookies.py")).unwrap();
    let reached = expand(&db, &[m.id], &ExpandOpts { max_depth: 1, ..Default::default() }).unwrap();
    assert!(reached.iter().any(|r| r.id == after.id));
    assert!(!reached.iter().any(|r| r.id == before.id));
}

#[test]
fn archived_memories_drop_out_of_search_but_stay_retrievable() {
    let db = open_memory().unwrap();
    let m = create_memory(&db, NewMemory::new("episode", "Debug", "Tried gaussian filter on depth map")).unwrap();
    set_status(&db, &m.id, "archived").unwrap();
    assert!(search_memories(&db, "gaussian", &SearchOpts::default()).unwrap().is_empty());
    let all = SearchOpts { include_history: true, ..Default::default() };
    assert_eq!(search_memories(&db, "gaussian", &all).unwrap().len(), 1);
    set_status(&db, &m.id, "current").unwrap();
    assert_eq!(search_memories(&db, "gaussian", &SearchOpts::default()).unwrap().len(), 1);
}

#[test]
fn decay_archives_old_episodes_but_never_decisions() {
    let db = open_memory().unwrap();
    let long_ago = "2026-01-01T00:00:00Z";
    let mk = |kind, title| NewMemory { salience: Some(0.5), observed_at: Some(long_ago.into()), ..NewMemory::new(kind, title, "x") };
    let episode = create_memory_at(&db, mk("episode", "tried filter"), long_ago).unwrap();
    let decision = create_memory_at(&db, mk("decision", "use voxel"), long_ago).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 30, 0, 0, 0).unwrap();
    assert!(strength(&get_memory(&db, &episode.id).unwrap().unwrap(), at) < 0.1);
    assert_eq!(archive_stale(&db, at).unwrap(), vec![episode.id]);
    assert_eq!(get_memory(&db, &decision.id).unwrap().unwrap().status, "current");
}

#[test]
fn access_log_attributes_requests_to_agents() {
    let db = open_memory().unwrap();
    let claude = ensure_actor(&db, "agent", "claude-code").unwrap();
    let codex = ensure_actor(&db, "agent", "codex").unwrap();
    assert_eq!(ensure_actor(&db, "agent", "claude-code").unwrap().id, claude.id);
    let m = create_memory(&db, NewMemory::new("fact", "t", "c")).unwrap();
    let rec = |actor: &str, tool: &str| AccessRecord {
        actor_id: Some(actor.into()),
        tool: tool.into(),
        request: json!({"q": "auth"}),
        returned: json!([{"id": m.id, "score": 3.2}]),
        tokens_out: Some(120),
        ..Default::default()
    };
    log_access(&db, rec(&claude.id, "search")).unwrap();
    log_access(&db, rec(&codex.id, "brief")).unwrap();
    record_access(&db, &[m.id.clone()]).unwrap();
    let all = recent_access(&db, None, 50).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].actor.as_deref(), Some("codex"));
    assert_eq!(recent_access(&db, Some(&claude.id), 50).unwrap().len(), 1);
    assert_eq!(get_memory(&db, &m.id).unwrap().unwrap().access_count, 1);
}
