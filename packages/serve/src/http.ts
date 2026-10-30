import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import type { Actor, Db } from "@engram/core";
import { graphData, nodeDetail, requestLog, stats, timeline } from "./api.ts";
import { createMcpServer } from "./mcp.ts";
import { opBrief, opExpand, opNote, opSearch, type OpContext } from "./ops.ts";
import { verifyToken } from "./tokens.ts";

export interface RunningServer {
  url: string;
  close(): Promise<void>;
}

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "authorization, content-type, mcp-session-id, mcp-protocol-version",
  "access-control-allow-methods": "GET, POST, OPTIONS",
};

function send(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json", ...CORS });
  res.end(JSON.stringify(body));
}

async function readJson(req: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  const text = Buffer.concat(chunks).toString("utf8");
  return text ? JSON.parse(text) : undefined;
}

const bearer = (req: IncomingMessage) => req.headers.authorization?.replace(/^Bearer\s+/i, "");

async function handleMcp(db: Db, actor: Actor, req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method !== "POST") return send(res, 405, { error: "use POST" });
  // Stateless: a fresh server and transport per request, so concurrent agents never share state.
  const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
  const server = createMcpServer({ db, actor });
  res.on("close", () => void transport.close().then(() => server.close()));
  await server.connect(transport);
  await transport.handleRequest(req, res, await readJson(req));
}

type Handler = (ctx: OpContext, url: URL, body: any) => unknown;

/** Plain-HTTP twins of the MCP tools, for agents without MCP, plus read-only views for the UI. */
const ROUTES: Record<string, Handler> = {
  "POST /api/brief": (ctx, _u, b) => opBrief(ctx, b ?? {}),
  "POST /api/search": (ctx, _u, b) => opSearch(ctx, b),
  "POST /api/expand": (ctx, _u, b) => opExpand(ctx, b),
  "POST /api/note": (ctx, _u, b) => opNote(ctx, b),
  "GET /api/health": () => ({ ok: true }),
  "GET /api/stats": (ctx) => stats(ctx.db),
  "GET /api/graph": (ctx, u) => graphData(ctx.db, { includeArchived: u.searchParams.get("archived") === "1" }),
  "GET /api/node": (ctx, u) => nodeDetail(ctx.db, u.searchParams.get("id") ?? ""),
  "GET /api/events": (ctx, u) => timeline(ctx.db, Number(u.searchParams.get("limit") ?? 100)),
  "GET /api/requests": (ctx, u) => requestLog(ctx.db, Number(u.searchParams.get("limit") ?? 100)),
};

export async function startServer(opts: { db: Db; port?: number; host?: string }): Promise<RunningServer> {
  const server: Server = createServer((req, res) => {
    void route(opts.db, req, res).catch((err: unknown) => {
      if (!res.headersSent) send(res, 500, { error: err instanceof Error ? err.message : String(err) });
    });
  });
  await new Promise<void>((resolve) => server.listen(opts.port ?? 0, opts.host ?? "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    url: `http://${opts.host ?? "127.0.0.1"}:${port}`,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}

async function route(db: Db, req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method === "OPTIONS") return void res.writeHead(204, CORS).end();
  const url = new URL(req.url ?? "/", "http://localhost");
  const actor = verifyToken(db, bearer(req));
  if (!actor) return send(res, 401, { error: "missing or invalid token" });

  if (url.pathname === "/mcp") return handleMcp(db, actor, req, res);
  const handler = ROUTES[`${req.method} ${url.pathname}`];
  if (!handler) return send(res, 404, { error: "not found" });
  const body = req.method === "POST" ? await readJson(req) : undefined;
  const out = handler({ db, actor }, url, body);
  const op = out as { text?: string; data?: unknown };
  send(res, 200, typeof op?.text === "string" && "data" in op ? op : out);
}
