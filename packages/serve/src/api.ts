import { getEntity, getMemory, history, listEvents, recentAccess, type Db } from "@engram/core";

/** Read-only JSON views for the desktop UI. */

interface GraphNode {
  id: string;
  label: string;
  kind: string;
  status?: string;
  isHub?: boolean;
}
interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
}

export function graphData(db: Db, opts: { includeArchived?: boolean; limit?: number } = {}) {
  const memories = db
    .prepare(
      `SELECT id, type, title, status, is_hub FROM memories
       WHERE (? = 1 OR status IN ('current','superseded')) ORDER BY observed_at DESC LIMIT ?`,
    )
    .all(opts.includeArchived ? 1 : 0, opts.limit ?? 300) as { id: string; type: string; title: string; status: string; is_hub: number }[];
  const nodes = new Map<string, GraphNode>(
    memories.map((m) => [m.id, { id: m.id, label: m.title, kind: m.type, status: m.status, isHub: m.is_hub === 1 }]),
  );

  const ids = JSON.stringify([...nodes.keys()]);
  const touching = db
    .prepare(
      `SELECT DISTINCT e.id, e.kind, e.qualified_name FROM entities e JOIN edges g ON (g.src = e.id OR g.dst = e.id)
       WHERE g.src IN (SELECT value FROM json_each(?1)) OR g.dst IN (SELECT value FROM json_each(?1))
       UNION SELECT id, kind, qualified_name FROM entities WHERE kind IN ('module','file') AND status = 'active'`,
    )
    .all(ids) as { id: string; kind: string; qualified_name: string }[];
  for (const e of touching) nodes.set(e.id, { id: e.id, label: e.qualified_name, kind: e.kind });

  const all = JSON.stringify([...nodes.keys()]);
  const edges = db
    .prepare(
      `SELECT id, src AS source, dst AS target, type FROM edges
       WHERE valid_to IS NULL AND src IN (SELECT value FROM json_each(?1)) AND dst IN (SELECT value FROM json_each(?1))`,
    )
    .all(all) as unknown as GraphEdge[];
  return { nodes: [...nodes.values()], edges };
}

export function nodeDetail(db: Db, id: string) {
  const memory = getMemory(db, id);
  if (memory) {
    const evidence = db
      .prepare(`SELECT e.seq, e.ts, e.kind, e.payload FROM memory_evidence me JOIN events e ON e.seq = me.event_seq WHERE me.memory_id = ?`)
      .all(id)
      .map((r) => ({ ...(r as object), payload: JSON.parse((r as { payload: string }).payload) }));
    return { type: "memory", memory, history: history(db, id), evidence };
  }
  const entity = getEntity(db, id);
  return entity ? { type: "entity", entity } : null;
}

export function stats(db: Db) {
  const count = (sql: string) => (db.prepare(sql).get() as { n: number }).n;
  return {
    events: count("SELECT COUNT(*) n FROM events"),
    memories: count("SELECT COUNT(*) n FROM memories WHERE status = 'current'"),
    archived: count("SELECT COUNT(*) n FROM memories WHERE status = 'archived'"),
    entities: count("SELECT COUNT(*) n FROM entities WHERE status = 'active'"),
    requests: count("SELECT COUNT(*) n FROM access_log"),
  };
}

/** Most recent events, oldest first. */
export function timeline(db: Db, limit = 100) {
  const { max } = db.prepare("SELECT COALESCE(MAX(seq), 0) AS max FROM events").get() as { max: number };
  return listEvents(db, { afterSeq: Math.max(0, max - limit), limit });
}
export const requestLog = (db: Db, limit = 100) => recentAccess(db, { limit });
