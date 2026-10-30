const RULES: readonly [string, RegExp][] = [
  ["private-key", /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g],
  ["aws-key", /\b(AKIA|ASIA)[0-9A-Z]{16}\b/g],
  ["github-token", /\bgh[pousr]_[A-Za-z0-9]{30,}\b/g],
  ["api-key", /\bsk-[A-Za-z0-9_-]{20,}\b/g],
  ["bearer", /\bBearer\s+[A-Za-z0-9._~+/=-]{16,}/g],
  ["assignment", /\b(password|passwd|secret|token|api[_-]?key)\b(\s*[:=]\s*)(["']?)[^\s"']{6,}\3/gi],
];

/** Remove likely secrets before anything is written to the event log. */
export function scrub(text: string): string {
  let out = text;
  for (const [name, re] of RULES) {
    out = out.replace(re, (...m) => (name === "assignment" ? `${m[1]}${m[2]}[REDACTED]` : `[REDACTED:${name}]`));
  }
  return out;
}

export function scrubDeep<T>(value: T): T {
  if (typeof value === "string") return scrub(value) as T;
  if (Array.isArray(value)) return value.map(scrubDeep) as T;
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrubDeep(v)])) as T;
  }
  return value;
}
