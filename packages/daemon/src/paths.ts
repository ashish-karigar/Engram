import { createHash } from "node:crypto";
import { realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";

export interface ProjectPaths {
  id: string;
  name: string;
  root: string;
  dir: string;
  db: string;
  tokens: string;
  runtime: string;
}

/** Engram's data lives outside the project, so there is nothing to gitignore or leak. */
export function appDataDir(env: NodeJS.ProcessEnv = process.env): string {
  if (env.ENGRAM_HOME) return env.ENGRAM_HOME;
  if (process.platform === "darwin") return join(homedir(), "Library", "Application Support", "Engram");
  return join(env.XDG_DATA_HOME ?? join(homedir(), ".local", "share"), "engram");
}

export function projectPaths(root: string, base = appDataDir()): ProjectPaths {
  const real = realpathSync(root);
  const id = createHash("sha1").update(real).digest("hex").slice(0, 12);
  const dir = join(base, "projects", id);
  return { id, name: basename(real), root: real, dir, db: join(dir, "engram.sqlite"), tokens: join(dir, "tokens.json"), runtime: join(dir, "runtime.json") };
}
