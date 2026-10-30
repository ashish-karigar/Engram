import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { ActorKind, Db } from "@engram/core";
import { issueToken, verifyToken } from "@engram/serve";

type TokenFile = Record<string, string>;

function read(file: string): TokenFile {
  return existsSync(file) ? (JSON.parse(readFileSync(file, "utf8")) as TokenFile) : {};
}

/**
 * Raw tokens must reach the UI and the agent config, so they are kept in a 0600 file
 * next to the database. The database itself only stores hashes.
 */
export function ensureToken(db: Db, file: string, kind: ActorKind, name: string): string {
  const tokens = read(file);
  const existing = tokens[name];
  if (existing && verifyToken(db, existing)) return existing;
  const { token } = issueToken(db, kind, name);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify({ ...tokens, [name]: token }, null, 2), { mode: 0o600 });
  chmodSync(file, 0o600);
  return token;
}
