/** Agents keep their colours everywhere; people get stable colours from a hash of their name. */
const PEOPLE = ["#e8cf5c", "#b184ff", "#3fb6c9", "#ef6a62", "#f0a35e", "#9fb4ff"];
export function authorColor(who?: string): string {
  if (!who) return "#3a4252";
  const agent = { "Claude Code": "--a-claude-code", Codex: "--a-codex", "Muse Code": "--a-muse-code" }[who];
  if (agent) return getComputedStyle(document.documentElement).getPropertyValue(agent).trim();
  if (who === "uncommitted") return "#5b6478";
  let h = 0;
  for (const c of who) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PEOPLE[h % PEOPLE.length]!;
}
