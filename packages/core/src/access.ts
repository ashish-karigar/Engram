import type { Db } from "./db.ts";

export interface AccessRecord {
  actorId?: string;
  sessionId?: string;
  tool: string;
  request: unknown;
  /** What was returned and why, e.g. [{ id, score, reason }]. */
  returned: unknown;
  tokensOut?: number;
  latencyMs?: number;
  error?: string;
  ts?: string;
}

export function logAccess(db: Db, a: AccessRecord): number {
  const res = db
    .prepare(
      `INSERT INTO access_log (ts, actor_id, session_id, tool, request, returned, tokens_out, latency_ms, error)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    )
    .run(
      a.ts ?? new Date().toISOString(),
      a.actorId ?? null,
      a.sessionId ?? null,
      a.tool,
      JSON.stringify(a.request ?? null),
      JSON.stringify(a.returned ?? null),
      a.tokensOut ?? null,
      a.latencyMs ?? null,
      a.error ?? null,
    );
  return Number(res.lastInsertRowid);
}

export function recentAccess(db: Db, opts: { actorId?: string; limit?: number } = {}) {
  const rows = db
    .prepare(
      `SELECT l.id, l.ts, l.tool, l.request, l.returned, l.tokens_out, l.latency_ms, l.error, a.name AS actor
       FROM access_log l LEFT JOIN actors a ON a.id = l.actor_id
       WHERE (? IS NULL OR l.actor_id = ?) ORDER BY l.id DESC LIMIT ?`,
    )
    .all(opts.actorId ?? null, opts.actorId ?? null, opts.limit ?? 50) as Record<string, unknown>[];
  return rows.map((r) => ({
    id: r.id as number,
    ts: r.ts as string,
    actor: r.actor as string | null,
    tool: r.tool as string,
    request: JSON.parse(r.request as string),
    returned: JSON.parse(r.returned as string),
    tokensOut: r.tokens_out as number | null,
    latencyMs: r.latency_ms as number | null,
    error: r.error as string | null,
  }));
}
