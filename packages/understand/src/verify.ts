import type { Candidate } from "./extract.ts";

export type Verdict = { ok: true } | { ok: false; reason: string };

const normalise = (s: string) => s.toLowerCase().replace(/\s+/g, " ").trim();

/**
 * Grounding check: a memory is accepted only if every cited event was in the task
 * and its quote appears verbatim (modulo whitespace/case) in one of them.
 */
export function verifyCandidate(c: Candidate, bySeq: ReadonlyMap<number, string>): Verdict {
  const cited = c.evidence.map((seq) => bySeq.get(seq));
  if (cited.some((text) => text === undefined)) return { ok: false, reason: "cites an event outside the task" };
  const quote = normalise(c.quote);
  const found = cited.some((text) => normalise(text!).includes(quote));
  return found ? { ok: true } : { ok: false, reason: "quote not found in cited evidence" };
}
