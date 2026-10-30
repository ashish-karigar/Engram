import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

export async function git(cwd: string, args: string[], env: Record<string, string> = {}): Promise<string> {
  const { stdout } = await run("git", args, {
    cwd,
    env: { ...process.env, ...env },
    maxBuffer: 64 * 1024 * 1024,
  });
  return stdout.trim();
}

export async function isRepo(cwd: string): Promise<boolean> {
  return git(cwd, ["rev-parse", "--is-inside-work-tree"]).then(
    (out) => out === "true",
    () => false,
  );
}

/** Engram requires git. Initialise a repository if the folder does not have one. */
export async function ensureRepo(cwd: string): Promise<{ initialised: boolean }> {
  if (await isRepo(cwd)) return { initialised: false };
  await git(cwd, ["init"]);
  return { initialised: true };
}

export async function headSha(cwd: string): Promise<string | null> {
  return git(cwd, ["rev-parse", "--verify", "-q", "HEAD"]).catch(() => null);
}

export async function gitDir(cwd: string): Promise<string> {
  return git(cwd, ["rev-parse", "--absolute-git-dir"]);
}

export interface CommitInfo {
  sha: string;
  date: string;
  author: string;
  subject: string;
  files: string[];
}

const RECORD = "\u001e";
const FIELD = "\u001f";

/** Commits newer than `since` (exclusive), oldest first. */
export async function listCommits(cwd: string, opts: { since?: string | null; limit?: number } = {}): Promise<CommitInfo[]> {
  if (!(await headSha(cwd))) return [];
  const range = opts.since ? [`${opts.since}..HEAD`] : [];
  const out = await git(cwd, [
    "log",
    ...range,
    `--max-count=${opts.limit ?? 200}`,
    "--reverse",
    "--name-only",
    `--format=${RECORD}%H${FIELD}%aI${FIELD}%an${FIELD}%s${FIELD}`,
  ]).catch(() => "");
  return out
    .split(RECORD)
    .filter(Boolean)
    .map((chunk) => {
      const [sha = "", date = "", author = "", subject = "", files = ""] = chunk.split(FIELD);
      return { sha, date, author, subject, files: files.split("\n").filter(Boolean) };
    });
}
