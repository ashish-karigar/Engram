import { execFile } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";

const run = promisify(execFile);

export type AgentId = "claude" | "codex" | "muse";

export interface Endpoint {
  url: string;
  token: string;
}

export interface ConnectPlan {
  agent: AgentId;
  /** What will change, shown to the user before anything is applied. */
  preview: string;
  apply(): Promise<string>;
}

const BEGIN = "# >>> engram >>>";
const END = "# <<< engram <<<";

function backup(file: string): void {
  if (existsSync(file)) copyFileSync(file, `${file}.engram.bak`);
  else mkdirSync(dirname(file), { recursive: true });
}

/** Insert or replace a marked block so re-connecting never duplicates the entry. */
export function upsertBlock(text: string, block: string): string {
  const wrapped = `${BEGIN}\n${block}\n${END}`;
  const start = text.indexOf(BEGIN);
  const end = text.indexOf(END);
  if (start >= 0 && end > start) return text.slice(0, start) + wrapped + text.slice(end + END.length);
  return `${text.replace(/\s*$/, "")}\n\n${wrapped}\n`;
}

const codexBlock = (e: Endpoint) =>
  `[mcp_servers.engram]\nurl = "${e.url}/mcp"\nhttp_headers = { Authorization = "Bearer ${e.token}" }`;

function codexPlan(e: Endpoint, file = join(homedir(), ".codex", "config.toml")): ConnectPlan {
  return {
    agent: "codex",
    preview: `Append to ${file}:\n${codexBlock({ ...e, token: "<token>" })}`,
    async apply() {
      backup(file);
      const current = existsSync(file) ? readFileSync(file, "utf8") : "";
      writeFileSync(file, upsertBlock(current, codexBlock(e)));
      return `Updated ${file} (backup: ${file}.engram.bak). Restart Codex.`;
    },
  };
}

/** Merge an `engram` entry into a JSON settings file's `mcpServers`, leaving everything else intact. */
export function mergeMcpServer(json: string, entry: unknown): string {
  const data = json.trim() ? (JSON.parse(json) as Record<string, any>) : {};
  data.mcpServers = { ...(data.mcpServers ?? {}), engram: entry };
  return JSON.stringify(data, null, 2) + "\n";
}

function musePlan(e: Endpoint, file = join(homedir(), ".config", "muse", "settings.json")): ConnectPlan {
  const entry = (token: string) => ({ type: "http", url: `${e.url}/mcp`, headers: { Authorization: `Bearer ${token}` } });
  return {
    agent: "muse",
    preview: `Add mcpServers.engram to ${file}:\n${JSON.stringify(entry("<token>"), null, 2)}`,
    async apply() {
      backup(file);
      const current = existsSync(file) ? readFileSync(file, "utf8") : "";
      writeFileSync(file, mergeMcpServer(current, entry(e.token)));
      return `Updated ${file} (backup: ${file}.engram.bak). Restart Muse. Entry format follows Meta's docs for streamable-HTTP servers; verify with /mcp inside Muse.`;
    },
  };
}

function claudePlan(e: Endpoint): ConnectPlan {
  const args = ["mcp", "add", "--transport", "http", "--scope", "user", "engram", `${e.url}/mcp`, "--header", `Authorization: Bearer ${e.token}`];
  return {
    agent: "claude",
    preview: `Run: claude mcp add --transport http --scope user engram ${e.url}/mcp --header "Authorization: Bearer <token>"`,
    async apply() {
      await run("claude", ["mcp", "remove", "--scope", "user", "engram"]).catch(() => undefined);
      await run("claude", args);
      return "Registered with Claude Code (user scope). Start a new session.";
    },
  };
}

export function planConnect(agent: AgentId, endpoint: Endpoint): ConnectPlan {
  switch (agent) {
    case "codex": return codexPlan(endpoint);
    case "muse": return musePlan(endpoint);
    case "claude": return claudePlan(endpoint);
  }
}

export const AGENTS: readonly AgentId[] = ["claude", "codex", "muse"];
