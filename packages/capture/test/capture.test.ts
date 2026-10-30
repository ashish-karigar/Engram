import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { listEvents, openDb } from "@engram/core";
import {
  SNAPSHOT_REF, ensureRepo, git, isIgnored, mergeChange, scrub, snapshotChanges,
  snapshotWorktree, startCapture, syncCommits, type ChangeKind,
} from "../src/index.ts";

async function tempRepo(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "engram-repo-"));
  execFileSync("git", ["init", "-q"], { cwd: dir });
  execFileSync("git", ["config", "user.email", "t@example.com"], { cwd: dir });
  execFileSync("git", ["config", "user.name", "T"], { cwd: dir });
  return dir;
}
const commit = (dir: string, msg: string) => {
  execFileSync("git", ["add", "-A"], { cwd: dir });
  execFileSync("git", ["commit", "-q", "-m", msg], { cwd: dir });
};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("scrub removes common secrets and keeps normal text", () => {
  const text = "key=AKIAABCDEFGHIJKLMNOP token: ghp_" + "a".repeat(36) + " password = hunter2hunter2 ok";
  const out = scrub(text);
  assert.doesNotMatch(out, /AKIA|ghp_|hunter2/);
  assert.match(out, /ok$/);
});

test("ignore rules skip noise directories and bytecode", () => {
  assert.ok(isIgnored("node_modules/x/index.js"));
  assert.ok(isIgnored("pkg/__pycache__/a.pyc"));
  assert.ok(!isIgnored("src/lidar/filter.py"));
});

test("mergeChange collapses bursts", () => {
  const m = new Map<string, ChangeKind>();
  mergeChange(m, "a.py", "add");
  mergeChange(m, "a.py", "change");
  assert.equal(m.get("a.py"), "add");
  mergeChange(m, "a.py", "unlink");
  assert.equal(m.size, 0);
});

test("ensureRepo initialises a plain folder", async () => {
  const dir = await mkdtemp(join(tmpdir(), "engram-plain-"));
  assert.deepEqual(await ensureRepo(dir), { initialised: true });
  assert.deepEqual(await ensureRepo(dir), { initialised: false });
  await rm(dir, { recursive: true });
});

test("snapshot saves uncommitted work without touching index or branch", async () => {
  const dir = await tempRepo();
  await writeFile(join(dir, "a.py"), "print(1)\n");
  commit(dir, "init");
  assert.equal(await snapshotWorktree(dir), null);

  await writeFile(join(dir, "a.py"), "print(2)\n");
  await writeFile(join(dir, "new.py"), "x = 1\n");
  const snap = await snapshotWorktree(dir);
  assert.ok(snap);
  assert.deepEqual((await snapshotChanges(dir)).sort(), ["a.py", "new.py"]);
  assert.equal(await git(dir, ["show", `${SNAPSHOT_REF}:new.py`]), "x = 1");

  assert.equal(await git(dir, ["status", "--porcelain", "--untracked-files=no"]), "M a.py");
  assert.equal(await git(dir, ["diff", "--cached", "--name-only"]), "");
  assert.equal(await snapshotWorktree(dir), null);
  await rm(dir, { recursive: true });
});

test("syncCommits is incremental and idempotent", async () => {
  const dir = await tempRepo();
  const db = openDb(":memory:");
  await writeFile(join(dir, "a.py"), "1\n");
  commit(dir, "first");
  const ctx = { db, root: dir };
  assert.equal((await syncCommits(ctx)).length, 1);
  await writeFile(join(dir, "b.py"), "2\n");
  commit(dir, "second");
  assert.equal((await syncCommits(ctx)).length, 1);
  assert.equal((await syncCommits(ctx)).length, 0);
  const subjects = listEvents(db, { kind: "git.commit" }).map((e) => (e.payload as { subject: string }).subject);
  assert.deepEqual(subjects, ["first", "second"]);
  await rm(dir, { recursive: true });
});

test("capture records file changes, commits and a snapshot end to end", async () => {
  const dir = await tempRepo();
  await writeFile(join(dir, "a.py"), "1\n");
  commit(dir, "init");
  const db = openDb(":memory:");
  const cap = await startCapture({ db, root: dir, debounceMs: 50, snapshotIntervalMs: 50 });
  await mkdir(join(dir, "pkg"));
  await writeFile(join(dir, "pkg", "lidar.py"), "def f(): pass\n");
  await sleep(1200);
  await cap.stop();

  const kinds = listEvents(db).map((e) => e.kind);
  assert.ok(kinds.includes("git.commit"));
  assert.ok(kinds.includes("file.change"));
  assert.ok(kinds.includes("snapshot.created"));
  await rm(dir, { recursive: true });
});
