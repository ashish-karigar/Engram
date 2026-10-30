import { expand, getEntity, getMemories, getMemory, history, logAccess, recordAccess, type Actor, type Db } from "@engram/core";
import { recordNote, type NoteInput } from "@engram/understand";
import { estimateTokens, packByDensity } from "./budget.ts";
import { orientation } from "./brief.ts";
import { formatMemory } from "./format.ts";
import { recall, type RecallItem } from "./recall.ts";

/** The four operations every agent gets, whether it connects over MCP or plain HTTP. */
export interface OpContext {
  db: Db;
  actor: Actor;
  sessionId?: string;
}

export interface OpResult {
  text: string;
  data: unknown;
}

const DEFAULT_BUDGET = 1200;

interface Returned {
  id: string;
  score?: number;
  reason?: string;
}

/** Run an operation, time it, and write the request log entry with what was returned and why. */
function logged(ctx: OpContext, tool: string, request: unknown, run: () => { result: OpResult; returned: Returned[] }): OpResult {
  const started = performance.now();
  try {
    const { result, returned } = run();
    recordAccess(ctx.db, returned.map((r) => r.id).filter((id) => id.startsWith("mem_")));
    logAccess(ctx.db, {
      actorId: ctx.actor.id,
      sessionId: ctx.sessionId,
      tool,
      request,
      returned,
      tokensOut: estimateTokens(result.text),
      latencyMs: Math.round(performance.now() - started),
    });
    return result;
  } catch (err) {
    logAccess(ctx.db, {
      actorId: ctx.actor.id,
      sessionId: ctx.sessionId,
      tool,
      request,
      returned: [],
      error: err instanceof Error ? err.message : String(err),
      latencyMs: Math.round(performance.now() - started),
    });
    throw err;
  }
}

const toReturned = (items: RecallItem[]): Returned[] =>
  items.map((i) => ({ id: i.memory.id, score: Number(i.score.toFixed(3)), reason: i.reason }));

function packItems(items: RecallItem[], budget: number): RecallItem[] {
  const lines = items.map((item) => ({ item, value: item.score, cost: estimateTokens(formatMemory(item.memory, item.reason)) }));
  return packByDensity(lines, budget).packed.sort((a, b) => b.score - a.score);
}

export function opSearch(ctx: OpContext, req: { query: string; budget?: number; asOf?: string }): OpResult {
  return logged(ctx, "search", req, () => {
    const picked = packItems(recall(ctx.db, req.query, { asOf: req.asOf }), req.budget ?? DEFAULT_BUDGET);
    const text = picked.length
      ? picked.map((i) => formatMemory(i.memory, i.reason)).join("\n")
      : "No matching memories.";
    return { result: { text, data: picked }, returned: toReturned(picked) };
  });
}

export function opBrief(ctx: OpContext, req: { task?: string; budget?: number } = {}): OpResult {
  return logged(ctx, "brief", req, () => {
    const o = orientation(ctx.db);
    const relevant = req.task ? recall(ctx.db, req.task, { limit: 10 }) : [];
    const seen = new Set(relevant.map((r) => r.memory.id));
    const background: RecallItem[] = o.memories
      .filter((m) => !seen.has(m.id))
      .map((memory, i) => ({ memory, score: 0.5 / (1 + i * 0.2), reason: "recent" }));
    const picked = packItems([...relevant.map((r) => ({ ...r, score: r.score + 1 })), ...background], req.budget ?? DEFAULT_BUDGET);

    const lines: string[] = [];
    if (o.modules.length) lines.push("Project map:", ...o.modules.map((m) => `- ${m.name} (${m.files} files)`), "");
    if (picked.length) lines.push("What to know:", ...picked.map((i) => formatMemory(i.memory, i.reason)), "");
    if (o.activity.length) lines.push("Recent agent activity:", ...o.activity.map((a) => `- ${a.actor}: ${a.kind} at ${a.ts}`));
    const text = lines.join("\n").trim() || "Memory is empty so far.";
    return { result: { text, data: { ...o, picked } }, returned: toReturned(picked) };
  });
}

/** Open one node: a memory's full story (reasons, history, evidence) or an entity's connections. */
export function opExpand(ctx: OpContext, req: { id: string; depth?: number }): OpResult {
  return logged(ctx, "expand", req, () => {
    const reached = expand(ctx.db, [req.id], { maxDepth: req.depth ?? 1, limit: 40 }).filter((n) => n.id !== req.id);
    const memories = getMemories(ctx.db, reached.map((n) => n.id).filter((id) => id.startsWith("mem_")));
    const entities = reached.flatMap((n) => (n.id.startsWith("ent_") ? [getEntity(ctx.db, n.id)] : [])).filter((e) => e !== undefined);
    const lines: string[] = [];

    const self = getMemory(ctx.db, req.id);
    if (self) {
      lines.push(formatMemory(self));
      const versions = history(ctx.db, req.id);
      if (versions.length > 1) lines.push("History:", ...versions.map((v) => `  v${v.version} [${v.status}] ${v.claim}`));
      const evidence = ctx.db.prepare("SELECT event_seq FROM memory_evidence WHERE memory_id = ?").all(req.id) as { event_seq: number }[];
      if (evidence.length) lines.push(`Evidence events: ${evidence.map((e) => `#${e.event_seq}`).join(", ")}`);
    } else {
      const entity = getEntity(ctx.db, req.id);
      if (!entity) throw new Error(`unknown node: ${req.id}`);
      lines.push(`${entity.kind} ${entity.qualifiedName}`);
    }
    if (entities.length) lines.push("Connected code:", ...entities.map((e) => `- ${e.kind} ${e.qualifiedName} (${e.id})`));
    if (memories.length) lines.push("Connected memories:", ...memories.map((m) => formatMemory(m)));
    const returned = memories.map((m) => ({ id: m.id, reason: "connected" }));
    return { result: { text: lines.join("\n"), data: { reached } }, returned };
  });
}

export function opNote(ctx: OpContext, note: NoteInput): OpResult {
  return logged(ctx, "note", note, () => {
    const saved = recordNote(ctx.db, note, { actorId: ctx.actor.id, sessionId: ctx.sessionId });
    const text = `Saved (${saved.action}): ${saved.memory.id}`;
    return { result: { text, data: saved }, returned: [{ id: saved.memory.id, reason: saved.action }] };
  });
}
