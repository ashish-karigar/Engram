import { expand, getMemories, searchMemories, strength, type Db, type Memory } from "@engram/core";

export interface RecallItem {
  memory: Memory;
  score: number;
  /** Human-readable explanation, stored in the access log. */
  reason: string;
}

const FTS_SEEDS = 8;
const GRAPH_WEIGHT = 0.6;

/** Entities whose path/name contains a query word act as extra graph entry points. */
function entitySeeds(db: Db, query: string): string[] {
  const words = (query.toLowerCase().match(/[\p{L}\p{N}_]{3,}/gu) ?? []).slice(0, 6);
  const ids = new Set<string>();
  for (const w of words) {
    const rows = db
      .prepare("SELECT id FROM entities WHERE status = 'active' AND lower(qualified_name) LIKE ? LIMIT 5")
      .all(`%${w}%`) as { id: string }[];
    rows.forEach((r) => ids.add(r.id));
  }
  return [...ids];
}

/**
 * Hybrid recall, no model call:
 *   1. keyword hits (BM25) and name-matched entities are the entry points ("major nodes")
 *   2. expand outward along live edges, scores decaying per hop
 *   3. rank = relevance × freshness, where freshness comes from decay + usage
 */
export function recall(db: Db, query: string, opts: { limit?: number; depth?: number; asOf?: string } = {}): RecallItem[] {
  const hits = searchMemories(db, query, { limit: FTS_SEEDS, asOf: opts.asOf });
  const top = hits[0]?.score || 1;
  const relevance = new Map<string, { score: number; reason: string }>();
  for (const h of hits) relevance.set(h.memory.id, { score: h.score / top, reason: "keyword match" });

  const seeds = [...hits.map((h) => h.memory.id), ...entitySeeds(db, query)];
  for (const node of expand(db, seeds, { maxDepth: opts.depth ?? 2, limit: 80 })) {
    if (!node.id.startsWith("mem_") || relevance.has(node.id)) continue;
    relevance.set(node.id, {
      score: node.score * GRAPH_WEIGHT,
      reason: `${node.depth} hop${node.depth === 1 ? "" : "s"} from a match`,
    });
  }

  const now = Date.now();
  const items = getMemories(db, [...relevance.keys()])
    .filter((m) => (opts.asOf ? true : m.status === "current"))
    .map((memory) => {
      const rel = relevance.get(memory.id)!;
      const fresh = 0.5 + Math.min(1, strength(memory, now)) / 2;
      return { memory, score: rel.score * fresh, reason: rel.reason };
    });
  return items.sort((a, b) => b.score - a.score).slice(0, opts.limit ?? 20);
}
