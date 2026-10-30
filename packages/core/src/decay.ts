import type { Db } from "./db.ts";
import { setStatus, type Memory, type MemoryType } from "./memory.ts";

const DAY_MS = 86_400_000;

/**
 * Half-life in days per memory type. Infinity means "persists until superseded".
 * Facts and decisions are only replaced by newer evidence; episodes and attempts fade.
 */
const HALF_LIFE_DAYS: Record<MemoryType, number> = {
  decision: Infinity,
  fact: Infinity,
  preference: 180,
  rejected_option: 120,
  dead_end: 60,
  episode: 14,
  task: 7,
};

const ARCHIVE_BELOW = 0.1;

/** 0..1+. Recency-decayed salience, boosted by use so frequently recalled memories resist fading. */
export function strength(m: Memory, now = Date.now()): number {
  const half = HALF_LIFE_DAYS[m.type];
  const last = Date.parse(m.lastAccessedAt ?? m.observedAt);
  const ageDays = Math.max(0, (now - last) / DAY_MS);
  const decay = half === Infinity ? 1 : Math.pow(0.5, ageDays / half);
  return m.salience * decay + Math.log1p(m.accessCount) * 0.05;
}

/** Archive current memories whose strength fell below the threshold. Hubs and pinned types are kept. */
export function archiveStale(db: Db, now = Date.now()): string[] {
  const rows = db.prepare("SELECT id FROM memories WHERE status = 'current' AND is_hub = 0").all() as { id: string }[];
  const archived: string[] = [];
  for (const { id } of rows) {
    const m = db.prepare("SELECT * FROM memories WHERE id = ?").get(id) as Record<string, unknown>;
    const candidate = rowToStrengthInput(m);
    if (strength(candidate, now) < ARCHIVE_BELOW) {
      setStatus(db, id, "archived", new Date(now).toISOString());
      archived.push(id);
    }
  }
  return archived;
}

function rowToStrengthInput(r: Record<string, unknown>): Memory {
  return {
    type: r.type as MemoryType,
    salience: r.salience as number,
    accessCount: r.access_count as number,
    lastAccessedAt: r.last_accessed_at as string | null,
    observedAt: r.observed_at as string,
  } as Memory;
}
