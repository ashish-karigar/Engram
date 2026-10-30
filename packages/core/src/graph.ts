import { newId, type Db } from "./db.ts";

export interface Entity {
  id: string;
  kind: string;
  qualifiedName: string;
  path: string | null;
  status: "active" | "deleted";
}

type Row = Record<string, unknown>;

function toEntity(r: Row): Entity {
  return {
    id: r.id as string,
    kind: r.kind as string,
    qualifiedName: r.qualified_name as string,
    path: r.path as string | null,
    status: r.status as "active" | "deleted",
  };
}

export function upsertEntity(
  db: Db,
  e: { kind: string; qualifiedName: string; path?: string; commitSha?: string },
  now = new Date().toISOString(),
): Entity {
  const found = db
    .prepare("SELECT * FROM entities WHERE kind = ? AND qualified_name = ?")
    .get(e.kind, e.qualifiedName) as Row | undefined;
  if (found) {
    db.prepare("UPDATE entities SET status = 'active', last_seen_commit = COALESCE(?, last_seen_commit) WHERE id = ?").run(
      e.commitSha ?? null,
      found.id as string,
    );
    return toEntity({ ...found, status: "active" });
  }
  const id = newId("ent");
  db.prepare(
    `INSERT INTO entities (id, kind, qualified_name, path, first_seen_commit, last_seen_commit, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
  ).run(id, e.kind, e.qualifiedName, e.path ?? null, e.commitSha ?? null, e.commitSha ?? null, now);
  return toEntity(db.prepare("SELECT * FROM entities WHERE id = ?").get(id) as Row);
}

export function getEntity(db: Db, id: string): Entity | undefined {
  const r = db.prepare("SELECT * FROM entities WHERE id = ?").get(id) as Row | undefined;
  return r ? toEntity(r) : undefined;
}

/** Record that an entity moved/renamed; memories keep following it through the new id. */
export function renameEntity(db: Db, oldId: string, to: { qualifiedName: string; path?: string }, now = new Date().toISOString()): Entity {
  const old = getEntity(db, oldId);
  if (!old) throw new Error(`entity not found: ${oldId}`);
  const next = upsertEntity(db, { kind: old.kind, qualifiedName: to.qualifiedName, path: to.path }, now);
  db.prepare("UPDATE entities SET renamed_from = ? WHERE id = ?").run(oldId, next.id);
  db.prepare("UPDATE entities SET status = 'deleted' WHERE id = ?").run(oldId);
  db.prepare("UPDATE edges SET dst = ? WHERE dst = ? AND type = 'touches'").run(next.id, oldId);
  return next;
}

export interface Reached {
  id: string;
  depth: number;
  /** Decays with each hop and with edge weight. Higher is closer/more relevant. */
  score: number;
}

/**
 * Expand outward from seed nodes along live edges (either direction) up to `maxDepth` hops.
 * This is the "find major nodes, then explore their neighbours" step of recall.
 */
export function expand(
  db: Db,
  seeds: string[],
  opts: { maxDepth?: number; edgeTypes?: string[]; decay?: number; limit?: number } = {},
): Reached[] {
  if (seeds.length === 0) return [];
  return db
    .prepare(
      `WITH RECURSIVE walk(id, depth, score) AS (
         SELECT value, 0, 1.0 FROM json_each(?1)
         UNION
         SELECT CASE WHEN e.src = w.id THEN e.dst ELSE e.src END, w.depth + 1, w.score * e.weight * ?3
         FROM walk w JOIN edges e ON (e.src = w.id OR e.dst = w.id)
         WHERE w.depth < ?2 AND e.valid_to IS NULL
           AND (?4 IS NULL OR e.type IN (SELECT value FROM json_each(?4)))
       )
       SELECT id, MIN(depth) AS depth, MAX(score) AS score FROM walk
       GROUP BY id ORDER BY score DESC, depth LIMIT ?5`,
    )
    .all(
      JSON.stringify(seeds),
      opts.maxDepth ?? 2,
      opts.decay ?? 0.7,
      opts.edgeTypes ? JSON.stringify(opts.edgeTypes) : null,
      opts.limit ?? 50,
    ) as unknown as Reached[];
}
