import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { listEvents } from "@engram/core";
import { git, snapshotChanges } from "@engram/capture";
import { scriptedClient } from "@engram/understand";
import { mergeMcpServer, startEngram, upsertBlock } from "../src/index.ts";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("connector config edits are idempotent and preserve existing settings", () => {
  const once = upsertBlock('model = "x"\n', "[mcp_servers.engram]\nurl = \"a\"");
  const twice = upsertBlock(once, "[mcp_servers.engram]\nurl = \"b\"");
  assert.equal(twice.match(/>>> engram/g)?.length, 1);
  assert.match(twice, /url = "b"/);
  assert.match(twice, /model = "x"/);

  const merged = JSON.parse(mergeMcpServer('{"model":"m","mcpServers":{"other":{"url":"u"}}}', { url: "e" }));
  assert.deepEqual(Object.keys(merged.mcpServers).sort(), ["engram", "other"]);
  assert.equal(merged.model, "m");
});

test("end to end: capture, extract, recall, two agents, recovery", async () => {
  const home = await mkdtemp(join(tmpdir(), "engram-home-"));
  const repo = await mkdtemp(join(tmpdir(), "engram-demo-"));
  process.env.ENGRAM_HOME = home;
  const sh = (...a: string[]) => execFileSync("git", a, { cwd: repo });
  sh("init", "-q"); sh("config", "user.email", "t@e.com"); sh("config", "user.name", "T");
  await mkdir(join(repo, "perception"));
  await writeFile(join(repo, "perception", "lidar.py"), "def voxel_filter(points): pass\n");
  await writeFile(join(repo, "perception", "fusion.py"), "from perception.lidar import voxel_filter\nclass Fusion: pass\n");
  sh("add", "-A"); sh("commit", "-q", "-m", "Use voxel grid filter instead of radius filter for lidar downsampling");

  const llm = scriptedClient(JSON.stringify({
    memories: [{
      type: "decision", title: "Lidar downsampling", claim: "Downsample lidar with a voxel grid filter",
      rationale: "radius filter was too slow", quote: "voxel grid filter instead of radius filter",
      evidence: [1], files: ["perception/lidar.py"], confidence: 0.9,
    }],
  }));
  const app = await startEngram(repo, { llm, pipelineEveryMs: 100 });
  try {
    const call = async (token: string, path: string, body?: unknown) =>
      (await fetch(`${app.url}${path}`, { method: body ? "POST" : "GET", headers: { authorization: `Bearer ${token}`, "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined })).json() as Promise<any>;

    await sleep(600);
    const graph = await call(app.uiToken, "/api/graph");
    assert.ok(graph.nodes.some((n: any) => n.label === "perception/fusion.py"), "python indexed");
    assert.ok(graph.edges.some((e: any) => e.type === "imports"), "import edge");
    assert.ok(graph.nodes.some((n: any) => n.label === "Lidar downsampling"), "memory extracted from the commit");

    // Two agents connect through the daemon's connect route (preview only) and share memory.
    const claude = await call(app.uiToken, "/api/connect", { agent: "claude" });
    assert.match(claude.preview, /claude mcp add/);
    const { default: fs } = await import("node:fs");
    const tokens = JSON.parse(fs.readFileSync(app.paths.tokens, "utf8")) as Record<string, string>;
    await call(tokens["claude-code"]!, "/api/note", { type: "dead_end", title: "Radius filter", claim: "Radius outlier removal took 80ms per frame", files: ["perception/lidar.py"] });
    const answer = await call(app.uiToken, "/api/search", { query: "why not radius filter lidar" });
    assert.match(answer.text, /voxel grid/);
    assert.match(answer.text, /80ms/);

    // Uncommitted work survives on the hidden ref.
    await writeFile(join(repo, "perception", "planner.py"), "def plan(): pass\n");
    await sleep(1500);
    assert.deepEqual(await snapshotChanges(repo), ["perception/planner.py"]);
    assert.match(await git(repo, ["show", "refs/engram/snapshots:perception/planner.py"]), /def plan/);
    assert.ok(listEvents(app.db, { kind: "snapshot.created" }).length >= 1);

    const log = await call(app.uiToken, "/api/requests");
    assert.ok(log.some((r: any) => r.actor === "claude-code" && r.tool === "note"));
  } finally {
    await app.stop();
    await rm(home, { recursive: true, force: true });
    await rm(repo, { recursive: true, force: true });
  }
});
