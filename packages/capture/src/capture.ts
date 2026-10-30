import { ensureActor, type Db } from "@engram/core";
import { ensureRepo, headSha } from "./git.ts";
import { recordFileChanges, recordSnapshot, syncCommits, type CaptureContext } from "./ingest.ts";
import { snapshotWorktree } from "./snapshot.ts";
import { watchProject, type FileChange } from "./watcher.ts";

export interface CaptureOptions {
  db: Db;
  root: string;
  debounceMs?: number;
  /** Minimum gap between snapshots so busy edits do not create a commit per keystroke. */
  snapshotIntervalMs?: number;
  /** Called after file changes are recorded, e.g. to re-index code. */
  onChange?(changes: FileChange[]): void;
}

export interface CaptureHandle {
  stop(): Promise<void>;
  /** Force a snapshot now, e.g. before shutdown. */
  snapshotNow(): Promise<void>;
}

export async function startCapture(opts: CaptureOptions): Promise<CaptureHandle> {
  await ensureRepo(opts.root);
  const watcherActor = ensureActor(opts.db, "system", "engram-watcher");
  const ctx: CaptureContext = { db: opts.db, root: opts.root, actorId: watcherActor.id };

  await syncCommits(ctx);

  let lastSnapshotAt = 0;
  let snapshotTimer: NodeJS.Timeout | undefined;
  const interval = opts.snapshotIntervalMs ?? 15_000;

  const snapshotNow = async () => {
    clearTimeout(snapshotTimer);
    snapshotTimer = undefined;
    lastSnapshotAt = Date.now();
    const snap = await snapshotWorktree(opts.root).catch(() => null);
    if (snap) recordSnapshot(ctx, snap);
  };

  const scheduleSnapshot = () => {
    if (snapshotTimer) return;
    const wait = Math.max(0, lastSnapshotAt + interval - Date.now());
    snapshotTimer = setTimeout(() => void snapshotNow(), wait);
  };

  const watcher = await watchProject({
    root: opts.root,
    debounceMs: opts.debounceMs,
    onFiles: (changes) => {
      void headSha(opts.root).then((sha) => recordFileChanges(ctx, changes, sha));
      scheduleSnapshot();
      opts.onChange?.(changes);
    },
    onGitActivity: () => void syncCommits(ctx),
  });

  return {
    async stop() {
      clearTimeout(snapshotTimer);
      await watcher.close();
    },
    snapshotNow,
  };
}
