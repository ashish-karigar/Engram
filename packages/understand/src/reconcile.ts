import { searchMemories, supersede, createMemory, type Db, type Memory, type NewMemory } from "@engram/core";

export type SaveAction = "created" | "superseded" | "duplicate";
export interface SaveResult {
  action: SaveAction;
  memory: Memory;
}

const tokens = (s: string) => new Set(s.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []);

/** Jaccard similarity of word sets: |A∩B| / |A∪B|. */
export function similarity(a: string, b: string): number {
  const x = tokens(a);
  const y = tokens(b);
  if (x.size === 0 && y.size === 0) return 1;
  let shared = 0;
  for (const t of x) if (y.has(t)) shared++;
  return shared / (x.size + y.size - shared);
}

const SAME_TOPIC = 0.6;

/**
 * Store a memory while keeping the graph consistent:
 * same topic + same claim -> duplicate; same topic + new claim -> supersede; otherwise create.
 */
export function saveMemory(db: Db, input: NewMemory): SaveResult {
  const match = searchMemories(db, input.title, { limit: 5 }).find(
    (h) => h.memory.type === input.type && similarity(h.memory.title, input.title) >= SAME_TOPIC,
  )?.memory;
  if (!match) return { action: "created", memory: createMemory(db, input) };
  if (similarity(match.claim, input.claim) >= 0.9) return { action: "duplicate", memory: match };
  return { action: "superseded", memory: supersede(db, match.id, input) };
}
