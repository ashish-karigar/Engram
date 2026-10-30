#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { agentName, AGENTS, planConnect, projectPaths, startEngram, ensureToken, type AgentId } from "./index.ts";

const [cmd = "help", ...rest] = process.argv.slice(2);
const flags = new Set(rest.filter((a) => a.startsWith("--")));
const positional = rest.filter((a) => !a.startsWith("--"));

function runtimeInfo(root: string) {
  try {
    return JSON.parse(readFileSync(projectPaths(root).runtime, "utf8")) as { url: string; uiToken: string; pid: number };
  } catch {
    return null;
  }
}

async function main(): Promise<void> {
  switch (cmd) {
    case "start": {
      const root = resolve(positional[0] ?? ".");
      const app = await startEngram(root, { port: Number(process.env.ENGRAM_PORT ?? 0) });
      console.log(`Engram watching ${app.paths.root}\n  api:  ${app.url}\n  data: ${app.paths.dir}`);
      console.log(`ENGRAM_READY ${JSON.stringify({ url: app.url, uiToken: app.uiToken, root: app.paths.root })}`);
      const shutdown = () => void app.stop().then(() => process.exit(0));
      process.on("SIGINT", shutdown);
      process.on("SIGTERM", shutdown);
      return;
    }
    case "connect": {
      const [agent, dir = "."] = positional;
      if (!AGENTS.includes(agent as AgentId)) throw new Error(`usage: engram connect <${AGENTS.join("|")}> [dir] [--apply]`);
      const root = resolve(dir);
      const info = runtimeInfo(root);
      if (!info) throw new Error("Engram is not running for this project. Run: engram start");
      const paths = projectPaths(root);
      const { openDb } = await import("@engram/core");
      const token = ensureToken(openDb(paths.db), paths.tokens, "agent", agentName(agent as AgentId));
      const plan = planConnect(agent as AgentId, { url: info.url, token });
      console.log(plan.preview);
      console.log(flags.has("--apply") ? `\n${await plan.apply()}` : "\nNothing changed. Re-run with --apply to make this change.");
      return;
    }
    case "status": {
      const info = runtimeInfo(resolve(positional[0] ?? "."));
      console.log(info ? `running at ${info.url} (pid ${info.pid})` : "not running");
      return;
    }
    default:
      console.log("engram start [dir]\nengram connect <claude|codex|muse> [dir] [--apply]\nengram status [dir]");
  }
}

main().catch((err: unknown) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
