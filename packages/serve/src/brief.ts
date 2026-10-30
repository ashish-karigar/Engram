import type { Db, Memory } from "@engram/core";
import { getMemories } from "@engram/core";

export interface Orientation {
  /** Subsystems ranked by how connected they are: the "major nodes". */
  modules: { name: string; files: number }[];
  /** Most recent current memories worth knowing at the start of any task. */
  memories: Memory[];
  /** Latest activity per agent, so a newly joined agent sees who is doing what. */
  activity: { actor: string; kind: string; ts: string }[];
}

export function orientation(db: Db, opts: { memories?: number; modules?: number } = {}): Orientation {
  const modules = db
    .prepare(
      `SELECT m.qualified_name AS name, COUNT(*) AS files
       FROM entities m JOIN edges e ON e.src = m.id AND e.type = 'contains'
       WHERE m.kind = 'module' AND m.status = 'active'
       GROUP BY m.id ORDER BY files DESC LIMIT ?`,
    )
    .all(opts.modules ?? 6) as { name: string; files: number }[];

  const recent = db
    .prepare(
      `SELECT id FROM memories WHERE status = 'current' AND type != 'episode'
       ORDER BY (type IN ('decision','preference','dead_end')) DESC, observed_at DESC LIMIT ?`,
    )
    .all(opts.memories ?? 12) as { id: string }[];

  const activity = db
    .prepare(
      `SELECT a.name AS actor, e.kind, MAX(e.ts) AS ts FROM events e JOIN actors a ON a.id = e.actor_id
       WHERE a.kind = 'agent' GROUP BY a.id ORDER BY ts DESC LIMIT 5`,
    )
    .all() as { actor: string; kind: string; ts: string }[];

  return { modules, memories: getMemories(db, recent.map((r) => r.id)), activity };
}
