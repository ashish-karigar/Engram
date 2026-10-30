import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { git, headSha } from "./git.ts";

export const SNAPSHOT_REF = "refs/engram/snapshots";

const IDENTITY = {
  GIT_AUTHOR_NAME: "Engram",
  GIT_AUTHOR_EMAIL: "engram@localhost",
  GIT_COMMITTER_NAME: "Engram",
  GIT_COMMITTER_EMAIL: "engram@localhost",
};

export interface Snapshot {
  ref: string;
  sha: string;
  base: string | null;
}

const resolve = (cwd: string, rev: string) => git(cwd, ["rev-parse", "--verify", "-q", rev]).catch(() => null);

/**
 * Save the working tree (tracked + untracked, honouring .gitignore) as a commit on a hidden ref.
 * A temporary index is used, so the user's index, branches and working tree are never touched.
 * Returns null when nothing changed since HEAD or since the previous snapshot.
 */
export async function snapshotWorktree(cwd: string, ref = SNAPSHOT_REF): Promise<Snapshot | null> {
  const base = await headSha(cwd);
  const dir = await mkdtemp(join(tmpdir(), "engram-idx-"));
  const env = { ...IDENTITY, GIT_INDEX_FILE: join(dir, "index") };
  try {
    if (base) await git(cwd, ["read-tree", "HEAD"], env);
    await git(cwd, ["add", "-A", "--", "."], env);
    const tree = await git(cwd, ["write-tree"], env);

    if (base && tree === (await resolve(cwd, "HEAD^{tree}"))) return null;
    const previous = await resolve(cwd, ref);
    if (previous && tree === (await resolve(cwd, `${ref}^{tree}`))) return null;

    const parent = previous ?? base;
    const message = `engram snapshot\n\nbase: ${base ?? "none"}`;
    const args = ["commit-tree", tree, ...(parent ? ["-p", parent] : []), "-m", message];
    const sha = await git(cwd, args, env);
    await git(cwd, ["update-ref", ref, sha]);
    return { ref, sha, base };
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

/** Files that differ between a snapshot and HEAD: what a snapshot can recover. */
export async function snapshotChanges(cwd: string, ref = SNAPSHOT_REF): Promise<string[]> {
  const out = await git(cwd, ["diff", "--name-only", "HEAD", ref]).catch(() => "");
  return out.split("\n").filter(Boolean);
}
