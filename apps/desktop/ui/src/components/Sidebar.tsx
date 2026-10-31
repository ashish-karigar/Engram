import type { Project } from "../api/types.ts";

interface Props {
  projects: Project[];
  selectedId: string | null;
  onSelect(id: string): void;
  onAdd(): void;
  onRemove(id: string): void;
  settingsOpen: boolean;
  onSettings(): void;
}

export function Sidebar({ projects, selectedId, onSelect, onAdd, onRemove, settingsOpen, onSettings }: Props) {
  return (
    <aside className="sidebar" data-tauri-drag-region>
      <div className="brand"><i />Engram</div>
      <div className="side-label">
        <span>Projects</span>
        <button className="btn ghost" onClick={onAdd} title="Add project">＋</button>
      </div>
      <div className="projects">
        {projects.map((p) => (
          <button key={p.id} className={`project ${p.id === selectedId && !settingsOpen ? "active" : ""}`} onClick={() => onSelect(p.id)}>
            <b className="clip">{p.name}</b>
            <small>{p.stats ? `${p.stats.memories} memories · ${p.stats.entities} code nodes` : "starting…"}</small>
            <span
              className="btn ghost danger x"
              role="button"
              title="Stop watching"
              onClick={(e) => { e.stopPropagation(); onRemove(p.id); }}
            >✕</span>
          </button>
        ))}
      </div>
      <button className={`side-settings ${settingsOpen ? "active" : ""}`} onClick={onSettings}>⚙ Settings</button>
      <div className="side-foot"><span className="dot" />Watching locally</div>
    </aside>
  );
}
