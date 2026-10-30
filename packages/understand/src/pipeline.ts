import { ensureActor, getMeta, listEvents, setMeta, type Db, type StoredEvent } from "@engram/core";
import { buildDigest, type DiffProvider } from "./digest.ts";
import { extractCandidates } from "./extract.ts";
import type { LlmClient } from "./llm.ts";
import { linkFiles } from "./link.ts";
import { saveMemory, type SaveAction } from "./reconcile.ts";
import { groupTasks, type Task } from "./tasks.ts";
import { verifyCandidate } from "./verify.ts";

const CURSOR_KEY = "understand.cursor";
const SKIP_KINDS = new Set(["snapshot.created", "agent.note"]);

export interface PipelineOptions {
  llm: LlmClient;
  getDiff?: DiffProvider;
  batchLimit?: number;
  idleGapMs?: number;
}

export interface PipelineReport {
  tasks: number;
  counts: Record<SaveAction, number>;
  rejected: { title: string; reason: string }[];
  error?: string;
}

/** A task is worth a model call only if it carries intent: a commit, or several file changes. */
function isSignificant(task: Task): boolean {
  const commits = task.events.filter((e) => e.kind === "git.commit").length;
  return commits > 0 || task.events.filter((e) => e.kind === "file.change").length >= 3;
}

const knownPaths = (events: readonly StoredEvent[]): Set<string> => {
  const paths = new Set<string>();
  for (const e of events) {
    const p = e.payload as { files?: unknown[] };
    for (const f of p.files ?? []) paths.add(typeof f === "string" ? f : (f as { path: string }).path);
  }
  return paths;
};

/** Process events after the cursor. On a model error the cursor stays put, so the task is retried. */
export async function runPipeline(db: Db, opts: PipelineOptions): Promise<PipelineReport> {
  const report: PipelineReport = { tasks: 0, counts: { created: 0, superseded: 0, duplicate: 0 }, rejected: [] };
  const cursor = Number(getMeta(db, CURSOR_KEY) ?? 0);
  const events = listEvents(db, { afterSeq: cursor, limit: opts.batchLimit ?? 500 }).filter((e) => !SKIP_KINDS.has(e.kind));
  const author = ensureActor(db, "system", "engram-extractor").id;

  for (const task of groupTasks(events, opts.idleGapMs)) {
    const last = task.events.at(-1)!.seq;
    if (isSignificant(task)) {
      try {
        await processTask(db, task, opts, author, report);
      } catch (err) {
        report.error = err instanceof Error ? err.message : String(err);
        return report;
      }
      report.tasks++;
    }
    setMeta(db, CURSOR_KEY, String(last));
  }
  return report;
}

async function processTask(db: Db, task: Task, opts: PipelineOptions, author: string, report: PipelineReport) {
  const digest = await buildDigest(task.events, opts.getDiff);
  const paths = knownPaths(task.events);
  const commitSha = task.events.findLast((e) => e.commitSha)?.commitSha ?? undefined;
  for (const c of await extractCandidates(opts.llm, digest.text)) {
    const verdict = verifyCandidate(c, digest.bySeq);
    if (!verdict.ok) {
      report.rejected.push({ title: c.title, reason: verdict.reason });
      continue;
    }
    const { action } = saveMemory(db, {
      type: c.type,
      title: c.title,
      claim: c.claim,
      rationale: c.rationale,
      confidence: c.confidence,
      salience: 0.4 + 0.4 * c.confidence,
      observedAt: task.endedAt,
      commitSha,
      authoredBy: author,
      evidence: c.evidence,
      touches: linkFiles(db, c.files, paths),
    });
    report.counts[action]++;
  }
}
