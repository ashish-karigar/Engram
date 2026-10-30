import type { StoredEvent } from "@engram/core";
import { scrub } from "@engram/capture";

export interface Digest {
  text: string;
  /** What the model was shown for each event; grounding checks quote against this. */
  bySeq: Map<number, string>;
}

export type DiffProvider = (sha: string) => Promise<string>;

const MAX_DIFF_CHARS = 4000;
const MAX_DIGEST_CHARS = 24_000;

function render(e: StoredEvent, diff: string): string {
  const p = e.payload as Record<string, unknown>;
  switch (e.kind) {
    case "git.commit":
      return `commit ${String(p.sha).slice(0, 8)}: ${p.subject}\nfiles: ${(p.files as string[]).join(", ")}${diff ? `\n${diff}` : ""}`;
    case "file.change":
      return `files changed: ${(p.files as { path: string; change: string }[]).map((f) => `${f.path} (${f.change})`).join(", ")}`;
    case "agent.note":
      return `agent note: ${JSON.stringify(p)}`;
    default:
      return `${e.kind}: ${JSON.stringify(p).slice(0, 500)}`;
  }
}

export async function buildDigest(events: readonly StoredEvent[], getDiff?: DiffProvider): Promise<Digest> {
  const bySeq = new Map<number, string>();
  const lines: string[] = [];
  let used = 0;
  for (const e of events) {
    if (e.kind === "snapshot.created") continue;
    const sha = (e.payload as { sha?: string }).sha;
    const diff = e.kind === "git.commit" && sha && getDiff ? (await getDiff(sha).catch(() => "")).slice(0, MAX_DIFF_CHARS) : "";
    const body = scrub(render(e, diff));
    bySeq.set(e.seq, body);
    const line = `[#${e.seq} ${e.ts}] ${body}`;
    if (used + line.length > MAX_DIGEST_CHARS) break;
    lines.push(line);
    used += line.length;
  }
  return { text: lines.join("\n\n"), bySeq };
}
