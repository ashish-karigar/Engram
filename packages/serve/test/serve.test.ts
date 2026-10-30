import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { addEdge, appendEvent, createMemory, openDb, recentAccess, upsertEntity } from "@engram/core";
import { recordNote } from "@engram/understand";
import { estimateTokens, issueToken, packByDensity, recall, startServer, verifyToken, type RunningServer } from "../src/index.ts";

const db = openDb(":memory:");
let server: RunningServer;
let claude: { token: string };
let codex: { token: string };

before(async () => {
  claude = issueToken(db, "agent", "claude-code");
  codex = issueToken(db, "agent", "codex");
  const auth = upsertEntity(db, { kind: "file", qualifiedName: "backend/auth.py", path: "backend/auth.py" });
  const web = upsertEntity(db, { kind: "file", qualifiedName: "web/login.py", path: "web/login.py" });
  addEdge(db, web.id, auth.id, "imports");
  const sess = createMemory(db, { type: "decision", title: "Auth strategy", claim: "Use signed session cookies", rationale: "Both sites are server rendered", touches: [auth.id] });
  createMemory(db, { type: "dead_end", title: "Login redirect loop", claim: "Cookie SameSite=Strict broke the redirect from site B", touches: [web.id] });
  createMemory(db, { type: "fact", title: "Billing", claim: "Stripe webhooks are verified in billing.py" });
  assert.ok(sess.id);
  server = await startServer({ db });
});
after(() => server.close());

test("tokens are per agent, hashed, and verifiable", () => {
  assert.equal(verifyToken(db, claude.token)?.name, "claude-code");
  assert.equal(verifyToken(db, "egm_wrong"), null);
  assert.equal(verifyToken(db, undefined), null);
});

test("packByDensity prefers valuable-per-token items and respects the budget", () => {
  const { packed, used } = packByDensity(
    [{ item: "big", value: 5, cost: 100 }, { item: "dense", value: 4, cost: 10 }, { item: "ok", value: 2, cost: 10 }],
    25,
  );
  assert.deepEqual(packed, ["dense", "ok"]);
  assert.equal(used, 20);
  assert.equal(estimateTokens("abcdefgh"), 2);
});

test("recall finds memories through the graph, not just keywords", () => {
  const items = recall(db, "backend auth");
  const titles = items.map((i) => i.memory.title);
  assert.equal(titles[0], "Auth strategy");
  assert.ok(titles.includes("Login redirect loop"), "reached via web/login.py -imports-> backend/auth.py");
  assert.ok(!titles.includes("Billing"));
});

test("rejects requests without a valid token", async () => {
  assert.equal((await fetch(`${server.url}/api/stats`)).status, 401);
  assert.equal((await fetch(`${server.url}/api/stats`, { headers: { authorization: "Bearer nope" } })).status, 401);
});

test("HTTP ops work and every call is attributed in the request log", async () => {
  const headers = { authorization: `Bearer ${codex.token}`, "content-type": "application/json" };
  const res = await fetch(`${server.url}/api/search`, { method: "POST", headers, body: JSON.stringify({ query: "why session cookies" }) });
  const body = (await res.json()) as { text: string };
  assert.match(body.text, /Auth strategy/);
  const entry = recentAccess(db, { limit: 1 })[0]!;
  assert.equal(entry.actor, "codex");
  assert.equal(entry.tool, "search");
  assert.ok((entry.returned as unknown[]).length > 0);
});

test("two agents share one memory over MCP", async () => {
  const connect = async (token: string) => {
    const client = new Client({ name: "test", version: "0" });
    await client.connect(new StreamableHTTPClientTransport(new URL(`${server.url}/mcp`), { requestInit: { headers: { authorization: `Bearer ${token}` } } }));
    return client;
  };
  const a = await connect(claude.token);
  const b = await connect(codex.token);
  const tools = (await a.listTools()).tools.map((t) => t.name).sort();
  assert.deepEqual(tools, ["brief", "expand", "note", "search"]);

  await a.callTool({ name: "note", arguments: { type: "decision", title: "Token expiry", claim: "Access tokens last 15 minutes", rationale: "limit blast radius", files: ["backend/auth.py"] } });
  const found = (await b.callTool({ name: "search", arguments: { query: "token expiry" } })) as { content: { text: string }[] };
  assert.match(found.content[0]!.text, /15 minutes/);

  const brief = (await b.callTool({ name: "brief", arguments: { task: "change auth from session to token" } })) as { content: { text: string }[] };
  assert.match(brief.content[0]!.text, /Project map|What to know/);
  assert.match(brief.content[0]!.text, /Auth strategy/);

  const id = /mem_[a-z0-9]+/.exec(found.content[0]!.text)![0];
  const open = (await b.callTool({ name: "expand", arguments: { id } })) as { content: { text: string }[] };
  assert.match(open.content[0]!.text, /backend\/auth\.py/);
  await Promise.all([a.close(), b.close()]);

  const who = recentAccess(db, { limit: 20 }).map((r) => `${r.actor}:${r.tool}`);
  assert.ok(who.includes("claude-code:note") && who.includes("codex:search"));
});

test("UI endpoints return graph, node detail, stats and timeline", async () => {
  appendEvent(db, { kind: "file.change", payload: { files: [] } });
  recordNote(db, { title: "Seed", claim: "x" }, { actorId: verifyToken(db, claude.token)!.id });
  const get = async (path: string) => (await fetch(`${server.url}${path}`, { headers: { authorization: `Bearer ${claude.token}` } })).json() as Promise<any>;
  const graph = await get("/api/graph");
  assert.ok(graph.nodes.length >= 4 && graph.edges.length >= 1);
  const memoryNode = graph.nodes.find((n: any) => n.label === "Auth strategy");
  const detail = await get(`/api/node?id=${memoryNode.id}`);
  assert.equal(detail.memory.title, "Auth strategy");
  assert.ok((await get("/api/stats")).memories >= 3);
  assert.ok((await get("/api/events")).length >= 1);
  assert.ok((await get("/api/requests")).length >= 1);
});
