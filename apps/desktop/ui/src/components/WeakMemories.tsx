import { useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import type { WeakMemory } from "../api/types.ts";
import { usePolling } from "../hooks/usePolling.ts";

const GROUPS: { reason: WeakMemory["reason"]; label: string; hint: string }[] = [
  { reason: "changelog", label: "Changelog, not decisions", hint: "A change restated without a reason. Git already has it." },
  { reason: "duplicate", label: "Duplicates", hint: "Says the same as another memory, which is kept." },
  { reason: "identifier", label: "Raw identifiers", hint: "IDs and keys belong in config, not in what agents read." },
  { reason: "low_confidence", label: "Low confidence", hint: "The extractor was unsure." },
];

/**
 * Suggested clean-up: memories that cost agents tokens without helping. Nothing is archived until
 * a person selects it, and the last batch can be restored.
 */
export function WeakMemories({ api }: { api: ProjectApi }) {
  const { data = [], reload } = usePolling<WeakMemory[]>(() => api.weak(), 8000, [api]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [lastBatch, setLastBatch] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const toggle = (ids: string[], on: boolean) =>
    setPicked((s) => { const n = new Set(s); ids.forEach((id) => (on ? n.add(id) : n.delete(id))); return n; });
  const run = async (ids: string[], restore: boolean) => {
    setBusy(true);
    try {
      await api.archive(ids, restore);
      setLastBatch(restore ? [] : ids);
      setPicked(new Set());
      reload();
    } finally {
      setBusy(false);
    }
  };

  if (data.length === 0 && lastBatch.length === 0) return null;
  return (
    <section className="weak">
      <div className="weak-head">
        <h3>Weak memories <span className="faint">{data.length}</span></h3>
        <p className="muted">These cost agents tokens without helping. Archived memories leave search and the brief; their history stays.</p>
        <div className="actions">
          <button className="btn primary" disabled={busy || picked.size === 0} onClick={() => run([...picked], false)}>Archive {picked.size || ""} selected</button>
          {lastBatch.length > 0 && <button className="btn ghost" disabled={busy} onClick={() => run(lastBatch, true)}>Undo last archive ({lastBatch.length})</button>}
        </div>
      </div>
      {GROUPS.map((g) => {
        const items = data.filter((w) => w.reason === g.reason);
        if (items.length === 0) return null;
        const ids = items.map((w) => w.memory.id);
        const all = ids.every((id) => picked.has(id));
        return (
          <div key={g.reason} className="weak-group">
            <label className="weak-group-head">
              <input type="checkbox" checked={all} onChange={(e) => toggle(ids, e.target.checked)} />
              <b>{g.label}</b> <span className="faint">{items.length} · {g.hint}</span>
            </label>
            <ul className="rows">
              {items.map((w) => (
                <li key={w.memory.id} className="row weak-row">
                  <label>
                    <input type="checkbox" checked={picked.has(w.memory.id)} onChange={(e) => toggle([w.memory.id], e.target.checked)} />
                    <span>
                      <span className="tl-kind" style={{ color: `var(--t-${w.memory.type})` }}>{w.memory.type.replace("_", " ")}</span>{" "}
                      <b>{w.memory.title}</b>: {w.memory.claim}
                      {(w.reason === "duplicate" || w.reason === "low_confidence") && <span className="faint weak-detail">{w.detail}</span>}
                    </span>
                  </label>
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </section>
  );
}
