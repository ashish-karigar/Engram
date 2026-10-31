import { useEffect, useMemo, useState } from "react";
import type { ProjectApi } from "../../api/client.ts";
import type { ActivityItem, ActivityPage } from "../../api/types.ts";
import { usePolling } from "../../hooks/usePolling.ts";
import { Entry, agentName } from "./Entry.tsx";
import { Gutter, agentColor } from "./Gutter.tsx";
import { gutterAt, itemKey, layout } from "./lanes.ts";

type Kind = "conversations" | "code" | "memories";
const KINDS: { id: Kind; label: string; match: (i: ActivityItem) => boolean }[] = [
  { id: "conversations", label: "Conversations", match: (i) => i.kind === "session" },
  { id: "code", label: "Code", match: (i) => i.kind === "commit" || i.kind === "edits" },
  { id: "memories", label: "Memories", match: (i) => i.kind === "memory" },
];
const DAY_HEADER = 38;

function dayLabel(t: number): string {
  const day = new Date(t), today = new Date();
  const diff = Math.round((new Date(today.toDateString()).getTime() - new Date(day.toDateString()).getTime()) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return day.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
}

/**
 * Project history as a git-style graph: the main line carries commits, edits and new memories;
 * each agent session branches off for as long as it runs. The newest page refreshes live;
 * "Load earlier" appends older pages.
 */
export function Timeline({ api }: { api: ProjectApi }) {
  const [system, setSystem] = useState(false);
  const [older, setOlder] = useState<ActivityPage[]>([]);
  const [hiddenAgents, setHiddenAgents] = useState<string[]>([]);
  const [hiddenKinds, setHiddenKinds] = useState<Kind[]>([]);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);
  const { data: latest, error } = usePolling(() => api.activity({ system }), 5000, [api, system]);
  useEffect(() => setOlder([]), [api, system]);

  const pages = latest ? [latest, ...older] : [];
  const last = pages[pages.length - 1];
  const all = useMemo(() => {
    const seen = new Map<string, ActivityItem>();
    for (const p of pages) for (const i of p.items) if (!seen.has(itemKey(i))) seen.set(itemKey(i), i);
    return [...seen.values()];
  }, [latest, older]);
  const agents = useMemo(() => [...new Set(all.flatMap((i) => (i.kind === "session" ? [i.agent] : [])))].sort(), [all]);
  const shown = all.filter((i) =>
    !(i.kind === "session" && hiddenAgents.includes(i.agent)) && !KINDS.some((k) => hiddenKinds.includes(k.id) && k.match(i)));
  const graph = useMemo(() => layout(shown), [shown.map(itemKey).join("|")]);

  const loadEarlier = async () => {
    if (!last) return;
    setLoading(true);
    try {
      const page = await api.activity({ system, before: last.from, days: 7 });
      setOlder((o) => [...o, page]);
    } finally {
      setLoading(false);
    }
  };
  const toggle = <T,>(list: T[], v: T) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const flip = (key: string) => setOpen((s) => { const n = new Set(s); n.has(key) ? n.delete(key) : n.add(key); return n; });

  return (
    <div className="scroll timeline">
      <div className="tl-filters">
        {KINDS.map((k) => (
          <button key={k.id} className={`tl-chip ${hiddenKinds.includes(k.id) ? "" : "on"}`} onClick={() => setHiddenKinds(toggle(hiddenKinds, k.id))}>{k.label}</button>
        ))}
        {agents.length > 0 && <span className="tl-sep" />}
        {agents.map((a) => (
          <button key={a} className={`tl-chip ${hiddenAgents.includes(a) ? "" : "on"}`} onClick={() => setHiddenAgents(toggle(hiddenAgents, a))}>
            <i style={{ background: agentColor(a) }} />{agentName(a)}
          </button>
        ))}
        <button className={`tl-chip quiet ${system ? "on" : ""}`} onClick={() => setSystem(!system)}>
          {system ? "Hide background" : `Show background${latest?.hiddenSystem ? ` (${latest.hiddenSystem})` : ""}`}
        </button>
      </div>

      {error && <div className="banner err">{error}</div>}
      <ul className="tl-rows">
        {graph.rows.map((r, i) => {
          const newDay = i === 0 || dayLabel(graph.rows[i - 1]!.t) !== dayLabel(r.t);
          const expandable = r.item.kind === "session" || r.item.kind === "commit";
          return (
            <li key={r.key} className={`tl-row ${r.item.kind}`}>
              <Gutter item={r.item} lane={r.lane} lanes={graph.lanes} offset={newDay ? DAY_HEADER : 0} {...gutterAt(graph, i)} />
              <div className="tl-body">
                {newDay && <div className="tl-day" style={{ height: DAY_HEADER }}>{dayLabel(r.t)}</div>}
                <div className={`tl-entry ${expandable ? "click" : ""}`} onClick={expandable ? () => flip(r.key) : undefined}>
                  <Entry item={r.item} open={open.has(r.key)} />
                </div>
              </div>
            </li>
          );
        })}
      </ul>
      {latest && graph.rows.length === 0 && <div className="row muted">Nothing here in this period.</div>}
      {last?.earlier && (
        <button className="btn tl-more" disabled={loading} onClick={loadEarlier}>{loading ? "Loading…" : "Load earlier"}</button>
      )}
    </div>
  );
}
