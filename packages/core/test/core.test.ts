import { test } from "node:test";
import assert from "node:assert/strict";
import {
  addEdge,
  appendEvent,
  createMemory,
  ensureActor,
  expand,
  getMemory,
  history,
  listEvents,
  logAccess,
  openDb,
  recentAccess,
  recordAccess,
  renameEntity,
  searchMemories,
  setStatus,
  startSession,
  supersede,
  upsertEntity,
  verifyChain,
} from "../src/index.ts";

test("event log is append-only and hash-chained", () => {
  const db = openDb(":memory:");
  const a = appendEvent(db, { kind: "file.change", payload: { path: "a.py" } });
  const b = appendEvent(db, { kind: "git.commit", payload: { sha: "abc" } });
  assert.equal(b.seq, a.seq + 1);
  assert.equal(verifyChain(db), null);
  assert.throws(() => db.exec("UPDATE events SET kind = 'x'"), /append-only/);
  assert.throws(() => db.exec("DELETE FROM events"), /append-only/);
});

test("appendEvent is idempotent on opId", () => {
  const db = openDb(":memory:");
  const first = appendEvent(db, { kind: "note", payload: 1, opId: "op-1" });
  const again = appendEvent(db, { kind: "note", payload: 2, opId: "op-1" });
  assert.equal(again.seq, first.seq);
  assert.equal(listEvents(db).length, 1);
});

test("tampering with the log is detected", () => {
  const db = openDb(":memory:");
  appendEvent(db, { kind: "note", payload: { v: 1 } });
  const e2 = appendEvent(db, { kind: "note", payload: { v: 2 } });
  db.exec("DROP TRIGGER events_no_update");
  db.exec("UPDATE events SET payload = '{\"v\":99}' WHERE seq = " + e2.seq);
  assert.equal(verifyChain(db), e2.seq);
});

test("supersede keeps history and moves validity", () => {
  const db = openDb(":memory:");
  const old = createMemory(
    db,
    { type: "decision", title: "Auth", claim: "Use session cookies for auth", rationale: "Simple, server-rendered" },
    "2026-10-27T10:00:00Z",
  );
  const next = supersede(
    db,
    old.id,
    { type: "decision", title: "Auth", claim: "Use JWT bearer tokens for auth", rationale: "Mobile client needs it" },
    "2026-10-30T09:00:00Z",
  );
  assert.equal(next.version, 2);
  assert.equal(next.status, "current");
  const oldNow = getMemory(db, old.id)!;
  assert.equal(oldNow.status, "superseded");
  assert.equal(oldNow.validTo, "2026-10-30T09:00:00Z");
  assert.deepEqual(
    history(db, next.id).map((m) => m.claim),
    ["Use session cookies for auth", "Use JWT bearer tokens for auth"],
  );
  assert.throws(() => supersede(db, old.id, { type: "decision", title: "x", claim: "y" }), /superseded/);
});

test("search returns current memories by default and honours asOf", () => {
  const db = openDb(":memory:");
  const old = createMemory(
    db,
    { type: "decision", title: "Auth", claim: "Sessions store login state in cookies" },
    "2026-10-27T10:00:00Z",
  );
  supersede(db, old.id, { type: "decision", title: "Auth", claim: "Tokens replace cookies for login" }, "2026-10-30T09:00:00Z");

  const now = searchMemories(db, "cookies login");
  assert.equal(now.length, 1);
  assert.match(now[0]!.memory.claim, /Tokens/);

  const past = searchMemories(db, "cookies login", { includeHistory: true, asOf: "2026-10-28T00:00:00Z" });
  assert.equal(past.length, 1);
  assert.match(past[0]!.memory.claim, /Sessions/);
});

test("search ignores punctuation instead of throwing", () => {
  const db = openDb(":memory:");
  createMemory(db, { type: "fact", title: "LiDAR rate", claim: "Point cloud arrives at 10 Hz" });
  assert.equal(searchMemories(db, 'lidar "rate" AND (10Hz) -- ;').length, 1);
  assert.deepEqual(searchMemories(db, "!!!"), []);
});

test("expand walks from major nodes to neighbours and decays with distance", () => {
  const db = openDb(":memory:");
  const lidar = upsertEntity(db, { kind: "module", qualifiedName: "perception.lidar" });
  const fusion = upsertEntity(db, { kind: "module", qualifiedName: "perception.fusion" });
  const planner = upsertEntity(db, { kind: "module", qualifiedName: "planning.planner" });
  addEdge(db, lidar.id, fusion.id, "feeds");
  addEdge(db, fusion.id, planner.id, "feeds");

  const m = createMemory(db, {
    type: "decision",
    title: "Voxel size",
    claim: "Downsample lidar with 0.2m voxels",
    rationale: "Fusion latency budget",
    touches: [lidar.id],
  });

  const reached = expand(db, [m.id], { maxDepth: 3 });
  const byId = new Map(reached.map((r) => [r.id, r]));
  assert.equal(byId.get(lidar.id)?.depth, 1);
  assert.equal(byId.get(fusion.id)?.depth, 2);
  assert.equal(byId.get(planner.id)?.depth, 3);
  assert.ok(byId.get(lidar.id)!.score > byId.get(planner.id)!.score);

  const shallow = expand(db, [m.id], { maxDepth: 1 });
  assert.deepEqual(new Set(shallow.map((r) => r.id)), new Set([m.id, lidar.id]));

  const onlyFeeds = expand(db, [lidar.id], { edgeTypes: ["feeds"], maxDepth: 3 });
  assert.ok(!onlyFeeds.some((r) => r.id === m.id));
});

test("memories follow an entity through a rename", () => {
  const db = openDb(":memory:");
  const before = upsertEntity(db, { kind: "file", qualifiedName: "auth/session.py", path: "auth/session.py" });
  const m = createMemory(db, { type: "decision", title: "Cookies", claim: "Sessions use signed cookies", touches: [before.id] });
  const after = renameEntity(db, before.id, { qualifiedName: "auth/cookies.py", path: "auth/cookies.py" });
  const reached = expand(db, [m.id], { maxDepth: 1 });
  assert.ok(reached.some((r) => r.id === after.id));
  assert.ok(!reached.some((r) => r.id === before.id));
});

test("archived memories drop out of search but stay retrievable", () => {
  const db = openDb(":memory:");
  const m = createMemory(db, { type: "episode", title: "Debug", claim: "Tried gaussian filter on depth map" });
  setStatus(db, m.id, "archived");
  assert.equal(searchMemories(db, "gaussian").length, 0);
  assert.equal(searchMemories(db, "gaussian", { includeHistory: true }).length, 1);
  setStatus(db, m.id, "current");
  assert.equal(searchMemories(db, "gaussian").length, 1);
});

test("access log records who asked for what and what came back", () => {
  const db = openDb(":memory:");
  const claude = ensureActor(db, "agent", "claude-code");
  const codex = ensureActor(db, "agent", "codex");
  assert.equal(ensureActor(db, "agent", "claude-code").id, claude.id);
  const ses = startSession(db, claude.id);
  const m = createMemory(db, { type: "fact", title: "t", claim: "c" });

  logAccess(db, {
    actorId: claude.id,
    sessionId: ses,
    tool: "search",
    request: { q: "auth" },
    returned: [{ id: m.id, score: 3.2, reason: "bm25" }],
    tokensOut: 120,
    latencyMs: 4,
  });
  logAccess(db, { actorId: codex.id, tool: "brief", request: {}, returned: [] });
  recordAccess(db, [m.id]);

  const all = recentAccess(db);
  assert.equal(all.length, 2);
  assert.equal(all[0]!.actor, "codex");
  const mine = recentAccess(db, { actorId: claude.id });
  assert.equal(mine.length, 1);
  assert.equal((mine[0]!.returned as { id: string }[])[0]!.id, m.id);
  assert.equal(getMemory(db, m.id)!.accessCount, 1);
});
