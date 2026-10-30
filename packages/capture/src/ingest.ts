import { appendEvent, getMeta, setMeta, type Db, type StoredEvent } from "@engram/core";
import { listCommits } from "./git.ts";
import { scrubDeep } from "./scrub.ts";
import type { FileChange } from "./watcher.ts";
import type { Snapshot } from "./snapshot.ts";

const LAST_COMMIT_KEY = "capture.last_commit";

export interface CaptureContext {
  db: Db;
  root: string;
  actorId?: string;
}

export function recordFileChanges(ctx: CaptureContext, changes: FileChange[], commitSha: string | null): StoredEvent {
  return appendEvent(ctx.db, {
    kind: "file.change",
    actorId: ctx.actorId,
    commitSha: commitSha ?? undefined,
    payload: { files: changes },
  });
}

export function recordSnapshot(ctx: CaptureContext, snap: Snapshot): StoredEvent {
  return appendEvent(ctx.db, {
    kind: "snapshot.created",
    actorId: ctx.actorId,
    commitSha: snap.base ?? undefined,
    opId: `snapshot:${snap.sha}`,
    payload: snap,
  });
}

/** Ingest commits we have not seen yet. Idempotent: each commit has a stable op id. */
export async function syncCommits(ctx: CaptureContext, limit = 200): Promise<StoredEvent[]> {
  const since = getMeta(ctx.db, LAST_COMMIT_KEY) ?? null;
  const commits = await listCommits(ctx.root, { since, limit }).catch(() => listCommits(ctx.root, { limit }));
  const events = commits.map((c) =>
    appendEvent(ctx.db, {
      kind: "git.commit",
      actorId: ctx.actorId,
      commitSha: c.sha,
      ts: c.date,
      opId: `commit:${c.sha}`,
      payload: scrubDeep({ sha: c.sha, author: c.author, subject: c.subject, files: c.files }),
    }),
  );
  const last = commits.at(-1);
  if (last) setMeta(ctx.db, LAST_COMMIT_KEY, last.sha);
  return events;
}
