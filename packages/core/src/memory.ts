import { newId, tx, type Db } from "./db.ts";

export type MemoryType = "decision" | "fact" | "preference" | "episode" | "rejected_option" | "dead_end" | "task";
export type MemoryStatus = "current" | "superseded" | "archived" | "expired";

export interface Memory {
  id: string;
  type: MemoryType;
  title: string;
  claim: string;
  rationale: string | null;
  scope: string | null;
  summary: string | null;
  status: MemoryStatus;
  version: number;
  isHub: boolean;
  observedAt: string;
  validFrom: string;
  validTo: string | null;
  eventDate: string | null;
  commitSha: string | null;
  confidence: number;
  salience: number;
  accessCount: number;
  lastAccessedAt: string | null;
  authoredBy: string | null;
  sessionId: string | null;
}

export interface NewMemory {
  type: MemoryType;
  title: string;
  claim: string;
  rationale?: string;
  scope?: string;
  summary?: string;
  isHub?: boolean;
  observedAt?: string;
  validFrom?: string;
  eventDate?: string;
  commitSha?: string;
  confidence?: number;
  salience?: number;
  authoredBy?: string;
  sessionId?: string;
  /** Raw events that support this memory. Memories with no evidence are allowed but rank lower. */
  evidence?: number[];
  /** Entities (files, symbols, ...) this memory is about. Creates `touches` edges. */
  touches?: string[];
}

type Row = Record<string, unknown>;

function toMemory(r: Row): Memory {
  return {
    id: r.id as string,
    type: r.type as MemoryType,
    title: r.title as string,
    claim: r.claim as string,
    rationale: r.rationale as string | null,
    scope: r.scope as string | null,
    summary: r.summary as string | null,
    status: r.status as MemoryStatus,
    version: r.version as number,
    isHub: r.is_hub === 1,
    observedAt: r.observed_at as string,
    validFrom: r.valid_from as string,
    validTo: r.valid_to as string | null,
    eventDate: r.event_date as string | null,
    commitSha: r.commit_sha as string | null,
    confidence: r.confidence as number,
    salience: r.salience as number,
    accessCount: r.access_count as number,
    lastAccessedAt: r.last_accessed_at as string | null,
    authoredBy: r.authored_by as string | null,
    sessionId: r.session_id as string | null,
  };
}

export function getMemory(db: Db, id: string): Memory | undefined {
  const r = db.prepare("SELECT * FROM memories WHERE id = ?").get(id) as Row | undefined;
  return r ? toMemory(r) : undefined;
}

export function getMemories(db: Db, ids: string[]): Memory[] {
  if (ids.length === 0) return [];
  const rows = db
    .prepare("SELECT * FROM memories WHERE id IN (SELECT value FROM json_each(?))")
    .all(JSON.stringify(ids)) as Row[];
  return rows.map(toMemory);
}

export function addEdge(
  db: Db,
  src: string,
  dst: string,
  type: string,
  opts: { weight?: number; validFrom?: string; createdBy?: string } = {},
): string {
  const existing = db
    .prepare("SELECT id FROM edges WHERE src = ? AND dst = ? AND type = ?")
    .get(src, dst, type) as { id: string } | undefined;
  if (existing) return existing.id;
  const id = newId("edg");
  db.prepare(
    "INSERT INTO edges (id, src, dst, type, weight, valid_from, created_by) VALUES (?, ?, ?, ?, ?, ?, ?)",
  ).run(id, src, dst, type, opts.weight ?? 1, opts.validFrom ?? new Date().toISOString(), opts.createdBy ?? null);
  return id;
}

export function createMemory(db: Db, m: NewMemory, now = new Date().toISOString()): Memory {
  return tx(db, () => insertMemory(db, m, now, 1));
}

function insertMemory(db: Db, m: NewMemory, now: string, version: number): Memory {
  const id = newId("mem");
  db.prepare(
    `INSERT INTO memories (id, type, title, claim, rationale, scope, summary, version, is_hub,
       observed_at, valid_from, event_date, commit_sha, confidence, salience, authored_by, session_id, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  ).run(
    id,
    m.type,
    m.title,
    m.claim,
    m.rationale ?? null,
    m.scope ?? null,
    m.summary ?? null,
    version,
    m.isHub ? 1 : 0,
    m.observedAt ?? now,
    m.validFrom ?? m.observedAt ?? now,
    m.eventDate ?? null,
    m.commitSha ?? null,
    m.confidence ?? 0.5,
    m.salience ?? 0.5,
    m.authoredBy ?? null,
    m.sessionId ?? null,
    now,
  );
  for (const seq of new Set(m.evidence ?? [])) {
    db.prepare("INSERT INTO memory_evidence (memory_id, event_seq) VALUES (?, ?)").run(id, seq);
  }
  for (const entityId of new Set(m.touches ?? [])) {
    addEdge(db, id, entityId, "touches", { validFrom: m.validFrom ?? now, createdBy: m.authoredBy });
  }
  return getMemory(db, id)!;
}

/** Replace a memory with a newer version. The old one is kept as history, not deleted. */
export function supersede(db: Db, oldId: string, next: NewMemory, now = new Date().toISOString()): Memory {
  return tx(db, () => {
    const old = getMemory(db, oldId);
    if (!old) throw new Error(`memory not found: ${oldId}`);
    if (old.status !== "current") throw new Error(`memory ${oldId} is ${old.status}, cannot supersede`);
    const validFrom = next.validFrom ?? next.observedAt ?? now;
    const created = insertMemory(db, { ...next, validFrom }, now, old.version + 1);
    db.prepare("UPDATE memories SET status = 'superseded', valid_to = ?, updated_at = ? WHERE id = ?").run(
      validFrom,
      now,
      oldId,
    );
    addEdge(db, created.id, oldId, "supersedes", { validFrom, createdBy: next.authoredBy });
    return created;
  });
}

export function setStatus(db: Db, id: string, status: MemoryStatus, now = new Date().toISOString()): void {
  if (status === "current") {
    db.prepare("UPDATE memories SET status = 'current', valid_to = NULL, updated_at = ? WHERE id = ?").run(now, id);
  } else {
    db.prepare("UPDATE memories SET status = ?, valid_to = COALESCE(valid_to, ?), updated_at = ? WHERE id = ?").run(
      status,
      now,
      now,
      id,
    );
  }
}

/** All versions of a memory, oldest first, following supersedes edges in both directions. */
export function history(db: Db, id: string): Memory[] {
  const rows = db
    .prepare(
      `WITH RECURSIVE chain(id) AS (
         SELECT ? UNION
         SELECT CASE WHEN e.src = c.id THEN e.dst ELSE e.src END
         FROM chain c JOIN edges e ON e.type = 'supersedes' AND (e.src = c.id OR e.dst = c.id)
       )
       SELECT m.* FROM memories m JOIN chain USING (id) ORDER BY m.version`,
    )
    .all(id) as Row[];
  return rows.map(toMemory);
}

const STOP_WORDS = new Set(
  "a an and are as at be by for from how in is it of on or that the this to was we what when where which who why with not do does did".split(" "),
);

export interface SearchHit {
  memory: Memory;
  score: number;
}

export function searchMemories(
  db: Db,
  query: string,
  opts: { limit?: number; includeHistory?: boolean; asOf?: string } = {},
): SearchHit[] {
  const words = query.match(/[\p{L}\p{N}_]+/gu) ?? [];
  const content = words.filter((w) => !STOP_WORDS.has(w.toLowerCase()));
  const terms = content.length > 0 ? content : words;
  if (terms.length === 0) return [];
  const match = terms.map((t) => `"${t}"`).join(" OR ");
  const rows = db
    .prepare(
      `SELECT m.*, bm25(memories_fts, 4.0, 2.0, 1.0, 1.0) AS rank
       FROM memories_fts JOIN memories m ON m.rowid = memories_fts.rowid
       WHERE memories_fts MATCH ?
         AND (? = 1 OR m.status = 'current')
         AND (? IS NULL OR (m.valid_from <= ? AND (m.valid_to IS NULL OR m.valid_to > ?)))
       ORDER BY rank LIMIT ?`,
    )
    .all(
      match,
      opts.includeHistory ? 1 : 0,
      opts.asOf ?? null,
      opts.asOf ?? null,
      opts.asOf ?? null,
      opts.limit ?? 10,
    ) as Row[];
  // bm25 is lower-is-better and negative; flip so higher is better.
  return rows.map((r) => ({ memory: toMemory(r), score: -(r.rank as number) }));
}

export function recordAccess(db: Db, ids: string[], now = new Date().toISOString()): void {
  if (ids.length === 0) return;
  db.prepare(
    `UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?
     WHERE id IN (SELECT value FROM json_each(?))`,
  ).run(now, JSON.stringify(ids));
}
