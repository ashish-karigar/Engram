import type { Memory } from "@engram/core";

const day = (iso: string) => iso.slice(0, 10);

/** One compact, self-describing line per memory. The id is a handle for `expand`. */
export function formatMemory(m: Memory, reason?: string): string {
  const why = m.rationale ? ` Why: ${m.rationale}` : "";
  const when = m.validTo ? `${day(m.validFrom)}→${day(m.validTo)}` : `since ${day(m.validFrom)}`;
  const tail = [m.id, when, m.commitSha?.slice(0, 7), reason].filter(Boolean).join(" · ");
  return `• [${m.type}] ${m.title}: ${m.claim}.${why} (${tail})`;
}
