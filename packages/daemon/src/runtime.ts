import { mkdirSync, writeFileSync } from "node:fs";
import { archiveStale, openDb, type Db } from "@engram/core";
import { ensureRepo, git, startCapture, type CaptureHandle } from "@engram/capture";
import { clientFromEnv, indexPython, runPipeline, type LlmClient } from "@engram/understand";
import { startServer, type RunningServer } from "@engram/serve";
import { planConnect, AGENTS, type AgentId } from "./connectors.ts";
import { projectPaths, type ProjectPaths } from "./paths.ts";
import { ensureToken } from "./secrets.ts";

export interface RunningEngram {
  paths: ProjectPaths;
  db: Db;
  url: string;
  uiToken: string;
  stop(): Promise<void>;
}

export interface StartOptions {
  port?: number;
  llm?: LlmClient | null;
  pipelineEveryMs?: number;
  decayEveryMs?: number;
}

const listPython = async (root: string) =>
  (await git(root, ["ls-files", "--cached", "--others", "--exclude-standard", "*.py"]).catch(() => "")).split("\n").filter(Boolean);

async function reindex(db: Db, root: string, changed?: string[]): Promise<void> {
  const all = await listPython(root);
  const paths = changed ? changed.filter((p) => p.endsWith(".py") && all.includes(p)) : all;
  if (paths.length) await indexPython(db, root, paths, new Set(all));
}

/** One project, fully wired: capture → understand → serve, plus background upkeep. */
export async function startEngram(root: string, opts: StartOptions = {}): Promise<RunningEngram> {
  await ensureRepo(root);
  const paths = projectPaths(root);
  mkdirSync(paths.dir, { recursive: true });
  const db = openDb(paths.db);
  const uiToken = ensureToken(db, paths.tokens, "user", "engram-ui");
  const llm = opts.llm === undefined ? clientFromEnv() : opts.llm;

  await reindex(db, paths.root);
  const capture: CaptureHandle = await startCapture({
    db,
    root: paths.root,
    onChange: (changes) => void reindex(db, paths.root, changes.map((c) => c.path)),
  });

  const timers: NodeJS.Timeout[] = [];
  let url = "";
  const connectRoutes = {
    "POST /api/connect": async (_ctx: unknown, _u: URL, body: { agent: AgentId; apply?: boolean }) => {
      if (!AGENTS.includes(body.agent)) throw new Error(`unknown agent: ${body.agent}`);
      const token = ensureToken(db, paths.tokens, "agent", agentName(body.agent));
      const plan = planConnect(body.agent, { url, token });
      return { agent: body.agent, preview: plan.preview, result: body.apply ? await plan.apply() : null };
    },
  };
  const server: RunningServer = await startServer({ db, port: opts.port ?? 0, routes: connectRoutes });
  url = server.url;
  writeFileSync(paths.runtime, JSON.stringify({ url, uiToken, pid: process.pid, root: paths.root }, null, 2), { mode: 0o600 });

  if (llm) {
    const tick = () => void runPipeline(db, { llm, getDiff: (sha) => git(paths.root, ["show", "--stat", "--patch", "--format=", sha]) });
    timers.push(setInterval(tick, opts.pipelineEveryMs ?? 60_000));
  }
  timers.push(setInterval(() => archiveStale(db), opts.decayEveryMs ?? 6 * 3_600_000));

  return {
    paths,
    db,
    url,
    uiToken,
    async stop() {
      timers.forEach(clearInterval);
      await capture.snapshotNow();
      await capture.stop();
      await server.close();
      db.close();
    },
  };
}

export const agentName = (id: AgentId) => ({ claude: "claude-code", codex: "codex", muse: "muse-code" })[id];
