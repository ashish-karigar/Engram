import { appendEvent, type Db, type MemoryType } from "@engram/core";
import { linkFiles } from "./link.ts";
import { saveMemory, type SaveResult } from "./reconcile.ts";

export interface NoteInput {
  type?: MemoryType;
  title: string;
  claim: string;
  rationale?: string;
  files?: string[];
  confidence?: number;
}

export interface NoteContext {
  actorId: string;
  sessionId?: string;
  commitSha?: string;
}

/**
 * An agent's own note is first-hand evidence: log it as an event, then promote it
 * to a memory that cites that event. No model call is needed.
 */
export function recordNote(db: Db, note: NoteInput, ctx: NoteContext): SaveResult & { eventSeq: number } {
  const event = appendEvent(db, {
    kind: "agent.note",
    actorId: ctx.actorId,
    sessionId: ctx.sessionId,
    commitSha: ctx.commitSha,
    payload: note,
  });
  const files = note.files ?? [];
  const result = saveMemory(db, {
    type: note.type ?? "fact",
    title: note.title,
    claim: note.claim,
    rationale: note.rationale,
    confidence: note.confidence ?? 0.8,
    salience: 0.7,
    commitSha: ctx.commitSha,
    authoredBy: ctx.actorId,
    sessionId: ctx.sessionId,
    evidence: [event.seq],
    touches: linkFiles(db, files, new Set(files)),
  });
  return { ...result, eventSeq: event.seq };
}
