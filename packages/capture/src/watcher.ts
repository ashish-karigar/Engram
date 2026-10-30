import { watch } from "chokidar";
import { relative, sep } from "node:path";
import { gitDir } from "./git.ts";

export type ChangeKind = "add" | "change" | "unlink";
export interface FileChange {
  path: string;
  change: ChangeKind;
}

const IGNORED_DIRS = new Set([
  ".git", "node_modules", "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache", ".idea", "dist", "build",
]);
const IGNORED_FILES = /(\.pyc|\.pyo|\.DS_Store|\.swp|~)$/;

export function isIgnored(relPath: string): boolean {
  return relPath.split(sep).some((part) => IGNORED_DIRS.has(part)) || IGNORED_FILES.test(relPath);
}

/** Collapse a burst of events into one change per path; the latest kind wins, add+unlink cancels. */
export function mergeChange(pending: Map<string, ChangeKind>, path: string, change: ChangeKind): void {
  const prev = pending.get(path);
  if (prev === "add" && change === "unlink") pending.delete(path);
  else if (prev === "add" && change === "change") pending.set(path, "add");
  else pending.set(path, change);
}

export interface WatchHandle {
  close(): Promise<void>;
}

export interface WatchOptions {
  root: string;
  debounceMs?: number;
  onFiles(changes: FileChange[]): void;
  /** Fired when HEAD moves (commit, checkout, rebase). */
  onGitActivity(): void;
}

export async function watchProject(opts: WatchOptions): Promise<WatchHandle> {
  const pending = new Map<string, ChangeKind>();
  let timer: NodeJS.Timeout | undefined;
  const debounce = opts.debounceMs ?? 500;

  const flush = () => {
    timer = undefined;
    if (pending.size === 0) return;
    const changes = [...pending].map(([path, change]) => ({ path, change }));
    pending.clear();
    opts.onFiles(changes);
  };

  const files = watch(opts.root, {
    ignoreInitial: true,
    ignored: (path) => isIgnored(relative(opts.root, path)),
    awaitWriteFinish: { stabilityThreshold: 150, pollInterval: 50 },
  });
  for (const kind of ["add", "change", "unlink"] as const) {
    files.on(kind, (path) => {
      mergeChange(pending, relative(opts.root, path), kind);
      clearTimeout(timer);
      timer = setTimeout(flush, debounce);
    });
  }

  const headLog = (await gitDir(opts.root)) + "/logs/HEAD";
  const head = watch(headLog, { ignoreInitial: true });
  head.on("add", opts.onGitActivity).on("change", opts.onGitActivity);

  return {
    async close() {
      clearTimeout(timer);
      await Promise.all([files.close(), head.close()]);
    },
  };
}
