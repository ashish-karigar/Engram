import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { appendEvent, archiveStale, createMemory, ensureActor, expand, getMemory, listEvents, openDb, searchMemories, strength } from "@engram/core";
import {
  groupTasks, indexPython, parseCandidates, parsePython, recordNote, resolveImport, runPipeline,
  scriptedClient, similarity, verifyCandidate,
} from "../src/index.ts";

test("tree-sitter extracts symbols and imports from Python", async () => {
  const idx = await parsePython(`
import os
import numpy as np
from perception.lidar import voxel
from . import utils
from ..common.geometry import Pose

class Fusion:
    def update(self): pass

def run(): pass
`);
  assert.deepEqual(idx.symbols.map((s) => `${s.kind}:${s.name}`), ["class:Fusion", "method:update", "function:run"]);
  assert.deepEqual(idx.imports.map((i) => `${".".repeat(i.level)}${i.module}`), ["os", "numpy", "perception.lidar", ".", "..common.geometry"]);
});

test("imports resolve to project files only", () => {
  const files = new Set(["perception/lidar.py", "perception/fusion.py", "common/geometry.py", "perception/__init__.py"]);
  assert.equal(resolveImport({ module: "perception.lidar", level: 0 }, "main.py", files), "perception/lidar.py");
  assert.equal(resolveImport({ module: "lidar", level: 1 }, "perception/fusion.py", files), "perception/lidar.py");
  assert.equal(resolveImport({ module: "common.geometry", level: 3 }, "perception/sub/x.py", files), "common/geometry.py");
  assert.equal(resolveImport({ module: "numpy", level: 0 }, "main.py", files), null);
});

test("indexPython builds module/file/symbol entities and import edges", async () => {
  const root = await mkdtemp(join(tmpdir(), "engram-py-"));
  await mkdir(join(root, "perception"));
  await writeFile(join(root, "perception", "lidar.py"), "def voxel(): pass\n");
  await writeFile(join(root, "perception", "fusion.py"), "from perception.lidar import voxel\nclass Fusion: pass\n");
  const db = openDb(":memory:");
  const all = new Set(["perception/lidar.py", "perception/fusion.py"]);
  const res = await indexPython(db, root, [...all], all);
  assert.deepEqual(res, { files: 2, symbols: 2, importEdges: 1 });
  const fusion = db.prepare("SELECT id FROM entities WHERE qualified_name = 'perception/fusion.py'").get() as { id: string };
  const near = expand(db, [fusion.id], { edgeTypes: ["imports"], maxDepth: 1 });
  assert.equal(near.length, 2);
  await rm(root, { recursive: true });
});

test("groupTasks splits on idle gaps", () => {
  const db = openDb(":memory:");
  const at = (ts: string) => appendEvent(db, { kind: "file.change", payload: {}, ts });
  const events = [at("2026-10-30T10:00:00Z"), at("2026-10-30T10:05:00Z"), at("2026-10-30T11:00:00Z")];
  assert.deepEqual(groupTasks(events).map((t) => t.events.length), [2, 1]);
});

test("similarity is a Jaccard word overlap", () => {
  assert.equal(similarity("Use JWT tokens", "use jwt tokens"), 1);
  assert.ok(similarity("Auth strategy", "Database schema") < 0.2);
});

test("parseCandidates tolerates fences and rejects ungrounded or malformed items", () => {
  const raw = '```json\n{"memories":[{"type":"decision","title":"T","claim":"C","quote":"q","evidence":[1]},{"type":"bogus","title":"x","claim":"y","quote":"q","evidence":[1]},{"type":"fact","title":"no quote","claim":"y","evidence":[1]}]}\n```';
  assert.equal(parseCandidates(raw).length, 1);
  assert.deepEqual(parseCandidates("not json"), []);
});

test("verifyCandidate requires the quote in the cited evidence", () => {
  const seen = new Map([[1, "commit abc: Switch to token auth because mobile app cannot hold cookies"]]);
  const base = { type: "decision" as const, title: "t", claim: "c", files: [], confidence: 0.9 };
  assert.ok(verifyCandidate({ ...base, quote: "cannot hold   COOKIES", evidence: [1] }, seen).ok);
  assert.equal(verifyCandidate({ ...base, quote: "made up", evidence: [1] }, seen).ok, false);
  assert.equal(verifyCandidate({ ...base, quote: "token", evidence: [9] }, seen).ok, false);
});

test("recordNote logs an event, creates a memory, and supersedes on a changed claim", () => {
  const db = openDb(":memory:");
  const claude = ensureActor(db, "agent", "claude-code");
  const first = recordNote(db, { type: "decision", title: "Auth strategy", claim: "Use session cookies", rationale: "simple", files: ["auth/login.py"] }, { actorId: claude.id });
  assert.equal(first.action, "created");
  const dup = recordNote(db, { type: "decision", title: "Auth strategy", claim: "Use session cookies", files: [] }, { actorId: claude.id });
  assert.equal(dup.action, "duplicate");
  const next = recordNote(db, { type: "decision", title: "Auth strategy", claim: "Use JWT bearer tokens", rationale: "mobile" }, { actorId: claude.id });
  assert.equal(next.action, "superseded");
  assert.equal(getMemory(db, first.memory.id)!.status, "superseded");
  assert.equal(searchMemories(db, "auth").length, 1);
  assert.equal(listEvents(db, { kind: "agent.note" }).length, 3);
});

test("pipeline extracts grounded memories and drops hallucinated ones", async () => {
  const db = openDb(":memory:");
  appendEvent(db, { kind: "git.commit", commitSha: "abc12345", ts: "2026-10-30T10:00:00Z", payload: { sha: "abc12345", subject: "Switch lidar filter to voxel grid", files: ["perception/lidar.py"], author: "a" } });
  const llm = scriptedClient(JSON.stringify({
    memories: [
      { type: "decision", title: "Lidar downsampling", claim: "Use a voxel grid filter", rationale: "faster than radius filter", quote: "voxel grid", evidence: [1], files: ["perception/lidar.py"], confidence: 0.9 },
      { type: "decision", title: "Invented", claim: "x", quote: "never said this", evidence: [1], files: [], confidence: 0.9 },
    ],
  }));
  const report = await runPipeline(db, { llm });
  assert.equal(report.counts.created, 1);
  assert.deepEqual(report.rejected.map((r) => r.title), ["Invented"]);
  const again = await runPipeline(db, { llm });
  assert.equal(again.tasks, 0, "cursor advanced; nothing reprocessed");
});

test("a failing model leaves the cursor so the task is retried", async () => {
  const db = openDb(":memory:");
  appendEvent(db, { kind: "git.commit", ts: "2026-10-30T10:00:00Z", payload: { sha: "abc12345", subject: "x", files: [], author: "a" } });
  const failing = { name: "down", complete: async () => { throw new Error("503"); } };
  assert.match((await runPipeline(db, { llm: failing })).error ?? "", /503/);
  const ok = await runPipeline(db, { llm: scriptedClient('{"memories":[]}') });
  assert.equal(ok.tasks, 1);
});

test("decay archives old episodes but never decisions", () => {
  const db = openDb(":memory:");
  const long = "2026-01-01T00:00:00Z";
  const ep = createMemory(db, { type: "episode", title: "tried filter", claim: "x", observedAt: long, salience: 0.5 }, long);
  const dec = createMemory(db, { type: "decision", title: "use voxel", claim: "y", observedAt: long, salience: 0.5 }, long);
  const now = Date.parse("2026-10-30T00:00:00Z");
  assert.ok(strength(getMemory(db, ep.id)!, now) < 0.1);
  assert.deepEqual(archiveStale(db, now), [ep.id]);
  assert.equal(getMemory(db, dec.id)!.status, "current");
});
