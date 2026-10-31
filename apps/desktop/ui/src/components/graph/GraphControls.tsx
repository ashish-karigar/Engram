import { useState } from "react";
import type { Graph } from "../../api/types.ts";
import type { GraphCommands } from "./ForceGraph.tsx";
import { DEFAULTS, GROUPS, type GraphSettings } from "./settings.ts";

interface Props {
  graph: Graph | undefined;
  settings: GraphSettings;
  onChange(s: GraphSettings): void;
  selectedId: string | null;
  zoom: number;
  commands: React.MutableRefObject<GraphCommands | null>;
}

function Slider({ label, value, min, max, step, onChange }: { label: string; value: number; min: number; max: number; step: number; onChange(v: number): void }) {
  return (
    <label className="slider">
      <span>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} />
    </label>
  );
}

function Toggle({ label, value, onChange }: { label: string; value: boolean; onChange(v: boolean): void }) {
  return (
    <label className="toggle">
      <span>{label}</span>
      <input type="checkbox" checked={value} onChange={(e) => onChange(e.target.checked)} />
      <i />
    </label>
  );
}

function Section({ title, children, open: initial = false }: { title: string; children: React.ReactNode; open?: boolean }) {
  const [open, setOpen] = useState(initial);
  return (
    <div className="gc-section">
      <button className="gc-title" onClick={() => setOpen(!open)}>{open ? "▾" : "▸"} {title}</button>
      {open && <div className="gc-body">{children}</div>}
    </div>
  );
}

/** Floating graph controls: filters, groups, display and forces, plus fit / center / zoom. */
export function GraphControls({ graph, settings: s, onChange, selectedId, zoom, commands }: Props) {
  const [open, setOpen] = useState(false);
  const set = <K extends keyof GraphSettings>(key: K, value: GraphSettings[K]) => onChange({ ...s, [key]: value });
  const counts = new Map<string, number>();
  for (const n of graph?.nodes ?? []) counts.set(n.kind, (counts.get(n.kind) ?? 0) + 1);
  const toggleGroup = (kind: string) => set("hidden", s.hidden.includes(kind) ? s.hidden.filter((k) => k !== kind) : [...s.hidden, kind]);

  return (
    <>
      <div className="graph-search">
        <input placeholder="Filter nodes…" value={s.search} onChange={(e) => set("search", e.target.value)} />
        {selectedId && (
          <label className="local-toggle">
            <input type="checkbox" checked={s.local} onChange={(e) => set("local", e.target.checked)} /> Local graph
            {s.local && (
              <select value={s.localDepth} onChange={(e) => set("localDepth", Number(e.target.value))}>
                {[1, 2, 3].map((d) => <option key={d} value={d}>{d} hop{d > 1 ? "s" : ""}</option>)}
              </select>
            )}
          </label>
        )}
      </div>

      <div className="graph-camera">
        <span className="seg mode">
          <button className={s.mode === "2d" ? "on" : ""} onClick={() => set("mode", "2d")}>2D</button>
          <button className={s.mode === "3d" ? "on" : ""} onClick={() => set("mode", "3d")}>3D</button>
        </span>
        <button onClick={() => commands.current?.fit()}>Fit <kbd>Z</kbd></button>
        <button onClick={() => commands.current?.center()}>Center <kbd>C</kbd></button>
        <span className="zoom">
          <button onClick={() => commands.current?.zoomBy(1 / 1.25)}>−</button>
          {s.mode === "2d" ? `${Math.round(zoom * 100)}%` : "zoom"}
          <button onClick={() => commands.current?.zoomBy(1.25)}>+</button>
        </span>
      </div>

      <div className={`graph-panel ${open ? "open" : ""}`}>
        <button className="gc-gear" onClick={() => setOpen(!open)} title="Graph settings">⚙</button>
        {open && (
          <div className="gc-scroll">
            <Section title="Filters" open>
              <Toggle label="Orphans" value={s.showOrphans} onChange={(v) => set("showOrphans", v)} />
              <Toggle label="Archived memories" value={s.showArchived} onChange={(v) => set("showArchived", v)} />
            </Section>
            <Section title="Groups" open>
              {GROUPS.filter((g) => counts.has(g.kind)).map((g) => (
                <button key={g.kind} className={`group ${s.hidden.includes(g.kind) ? "off" : ""}`} onClick={() => toggleGroup(g.kind)}>
                  <i style={{ background: `var(--t-${g.kind})` }} />{g.label}<span>{counts.get(g.kind)}</span>
                </button>
              ))}
            </Section>
            <Section title="Display">
              <Toggle label="Arrows" value={s.arrows} onChange={(v) => set("arrows", v)} />
              <Slider label="Text fade threshold" value={s.textFade} min={0.2} max={3} step={0.05} onChange={(v) => set("textFade", v)} />
              <Slider label="Node size" value={s.nodeSize} min={0.4} max={2.5} step={0.05} onChange={(v) => set("nodeSize", v)} />
              <Slider label="Link thickness" value={s.linkWidth} min={0.2} max={4} step={0.1} onChange={(v) => set("linkWidth", v)} />
            </Section>
            <Section title="Forces">
              <Slider label="Center force" value={s.centerForce} min={0} max={1} step={0.01} onChange={(v) => set("centerForce", v)} />
              <Slider label="Repel force" value={s.repel} min={0} max={40} step={0.5} onChange={(v) => set("repel", v)} />
              <Slider label="Link force" value={s.linkForce} min={0} max={1} step={0.01} onChange={(v) => set("linkForce", v)} />
              <Slider label="Link distance" value={s.linkDistance} min={20} max={300} step={1} onChange={(v) => set("linkDistance", v)} />
            </Section>
            <button className="btn ghost reset" onClick={() => onChange({ ...DEFAULTS, search: s.search })}>Reset to defaults</button>
          </div>
        )}
      </div>
    </>
  );
}
