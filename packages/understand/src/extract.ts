import type { MemoryType } from "@engram/core";
import type { LlmClient } from "./llm.ts";

export interface Candidate {
  type: MemoryType;
  title: string;
  claim: string;
  rationale?: string;
  /** Verbatim text from the evidence that supports the claim. */
  quote: string;
  evidence: number[];
  files: string[];
  confidence: number;
}

const ALLOWED = new Set<MemoryType>(["decision", "rejected_option", "dead_end", "fact", "preference"]);

export const SYSTEM_PROMPT = `You extract durable engineering memory from a log of coding-agent and developer activity.
Return ONLY JSON: {"memories":[{"type","title","claim","rationale","quote","evidence","files","confidence"}]}.
- type: decision | rejected_option | dead_end | fact | preference
- decision: something chosen, with WHY in "rationale". rejected_option: an alternative considered and not chosen, with why. dead_end: an attempt that failed.
- quote: a short VERBATIM snippet copied from one cited event that supports the claim. Never invent quotes.
- evidence: the event numbers (the #N in the log) you used.
- files: project file paths involved. confidence: 0..1.
Skip trivia, formatting, and anything you cannot ground in the log. An empty list is a good answer.`;

export async function extractCandidates(llm: LlmClient, digest: string): Promise<Candidate[]> {
  const raw = await llm.complete({ system: SYSTEM_PROMPT, prompt: `Activity log:\n\n${digest}`, maxTokens: 2500 });
  return parseCandidates(raw);
}

export function parseCandidates(raw: string): Candidate[] {
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start < 0 || end <= start) return [];
  let data: unknown;
  try {
    data = JSON.parse(raw.slice(start, end + 1));
  } catch {
    return [];
  }
  const list = (data as { memories?: unknown }).memories;
  if (!Array.isArray(list)) return [];
  return list.flatMap((item) => {
    const c = toCandidate(item);
    return c ? [c] : [];
  });
}

const str = (v: unknown) => (typeof v === "string" && v.trim() ? v.trim() : null);
const strings = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);

function toCandidate(item: unknown): Candidate | null {
  if (!item || typeof item !== "object") return null;
  const o = item as Record<string, unknown>;
  const type = o.type as MemoryType;
  const title = str(o.title);
  const claim = str(o.claim);
  const quote = str(o.quote);
  const evidence = (Array.isArray(o.evidence) ? o.evidence : []).filter((n): n is number => Number.isInteger(n));
  if (!ALLOWED.has(type) || !title || !claim || !quote || evidence.length === 0) return null;
  const confidence = typeof o.confidence === "number" ? Math.min(1, Math.max(0, o.confidence)) : 0.5;
  return { type, title, claim, quote, evidence, confidence, rationale: str(o.rationale) ?? undefined, files: strings(o.files) };
}
