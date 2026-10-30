import { createHash, randomUUID } from "node:crypto";
import { newId, tx, type Db } from "./db.ts";

export type ActorKind = "agent" | "user" | "system";

export interface Actor {
  id: string;
  kind: ActorKind;
  name: string;
}

export function ensureActor(db: Db, kind: ActorKind, name: string, now = new Date().toISOString()): Actor {
  const found = db.prepare("SELECT id, kind, name FROM actors WHERE kind = ? AND name = ?").get(kind, name) as
    | Actor
    | undefined;
  if (found) return { ...found };
  const id = newId("act");
  db.prepare("INSERT INTO actors (id, kind, name, created_at) VALUES (?, ?, ?, ?)").run(id, kind, name, now);
  return { id, kind, name };
}

export function startSession(db: Db, actorId: string, externalRef?: string, now = new Date().toISOString()): string {
  const id = newId("ses");
  db.prepare("INSERT INTO sessions (id, actor_id, external_ref, started_at) VALUES (?, ?, ?, ?)").run(
    id,
    actorId,
    externalRef ?? null,
    now,
  );
  return id;
}

export interface NewEvent {
  kind: string;
  payload: unknown;
  actorId?: string;
  sessionId?: string;
  commitSha?: string;
  deviceId?: string;
  /** Idempotency key. Re-sending the same opId returns the original event. */
  opId?: string;
  ts?: string;
}

export interface StoredEvent {
  seq: number;
  opId: string;
  ts: string;
  kind: string;
  actorId: string | null;
  sessionId: string | null;
  commitSha: string | null;
  payload: unknown;
  hash: string;
}

const GENESIS = "0".repeat(64);

type EventRow = {
  seq: number;
  op_id: string;
  ts: string;
  kind: string;
  actor_id: string | null;
  session_id: string | null;
  commit_sha: string | null;
  payload: string;
  hash: string;
};

function toEvent(r: EventRow): StoredEvent {
  return {
    seq: r.seq,
    opId: r.op_id,
    ts: r.ts,
    kind: r.kind,
    actorId: r.actor_id,
    sessionId: r.session_id,
    commitSha: r.commit_sha,
    payload: JSON.parse(r.payload),
    hash: r.hash,
  };
}

function chainHash(prev: string, opId: string, kind: string, payload: string): string {
  return createHash("sha256").update(`${prev}\n${opId}\n${kind}\n${payload}`).digest("hex");
}

export function appendEvent(db: Db, e: NewEvent): StoredEvent {
  const opId = e.opId ?? randomUUID();
  return tx(db, () => {
    const existing = db.prepare("SELECT * FROM events WHERE op_id = ?").get(opId) as EventRow | undefined;
    if (existing) return toEvent(existing);
    const last = db.prepare("SELECT hash FROM events ORDER BY seq DESC LIMIT 1").get() as { hash: string } | undefined;
    const prev = last?.hash ?? GENESIS;
    const payload = JSON.stringify(e.payload ?? null);
    const hash = chainHash(prev, opId, e.kind, payload);
    const res = db
      .prepare(
        `INSERT INTO events (op_id, ts, kind, actor_id, session_id, device_id, commit_sha, payload, prev_hash, hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        opId,
        e.ts ?? new Date().toISOString(),
        e.kind,
        e.actorId ?? null,
        e.sessionId ?? null,
        e.deviceId ?? "local",
        e.commitSha ?? null,
        payload,
        prev,
        hash,
      );
    return toEvent(db.prepare("SELECT * FROM events WHERE seq = ?").get(Number(res.lastInsertRowid)) as EventRow);
  });
}

export function getEvent(db: Db, seq: number): StoredEvent | undefined {
  const r = db.prepare("SELECT * FROM events WHERE seq = ?").get(seq) as EventRow | undefined;
  return r ? toEvent(r) : undefined;
}

export function listEvents(db: Db, opts: { afterSeq?: number; kind?: string; limit?: number } = {}): StoredEvent[] {
  const rows = db
    .prepare(
      `SELECT * FROM events WHERE seq > ? AND (? IS NULL OR kind = ?) ORDER BY seq LIMIT ?`,
    )
    .all(opts.afterSeq ?? 0, opts.kind ?? null, opts.kind ?? null, opts.limit ?? 1000) as EventRow[];
  return rows.map(toEvent);
}

/** Recompute the hash chain. Returns the first broken seq, or null if the log is intact. */
export function verifyChain(db: Db): number | null {
  const rows = db.prepare("SELECT seq, op_id, kind, payload, prev_hash, hash FROM events ORDER BY seq").all() as {
    seq: number;
    op_id: string;
    kind: string;
    payload: string;
    prev_hash: string;
    hash: string;
  }[];
  let prev = GENESIS;
  for (const r of rows) {
    if (r.prev_hash !== prev || r.hash !== chainHash(prev, r.op_id, r.kind, r.payload)) return r.seq;
    prev = r.hash;
  }
  return null;
}
