import type { Project } from "../api/types.ts";
import { CloseIcon, InfoIcon, Logo, PlusIcon, SettingsIcon } from "./icons.tsx";

export type Page = "project" | "settings" | "about";

interface Props {
  projects: Project[];
  selectedId: string | null;
  page: Page;
  onSelect(id: string): void;
  onAdd(): void;
  onRemove(project: Project): void;
  onPage(page: Page): void;
}

/** A quiet, stable colour per project name, for its initial. */
function hue(name: string): number {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

export function Sidebar({ projects, selectedId, page, onSelect, onAdd, onRemove, onPage }: Props) {
  return (
    <aside className="sidebar">
      <div className="side-drag" data-tauri-drag-region />
      <div className="brand" data-tauri-drag-region>
        <Logo size={28} />
        <div className="brand-text">
          <b>Engram</b>
          <small>Shared memory for agents</small>
        </div>
      </div>

      <div className="side-label">
        <span>Projects{projects.length > 0 && <em>{projects.length}</em>}</span>
        <button className="icon-btn" onClick={onAdd} title="Add a project folder" aria-label="Add a project folder"><PlusIcon size={15} /></button>
      </div>
      <div className="projects">
        {projects.map((p) => (
          <div
            key={p.id}
            role="button"
            tabIndex={0}
            className={`project ${p.id === selectedId && page === "project" ? "active" : ""}`}
            onClick={() => onSelect(p.id)}
            onKeyDown={(e) => { if (e.key === "Enter") onSelect(p.id); }}
            title={p.root}
          >
            <span className="avatar" style={{ "--h": hue(p.name) } as React.CSSProperties}>{p.name.slice(0, 1).toUpperCase()}</span>
            <span className="project-text">
              <b className="clip">{p.name}</b>
              <small className="clip">{p.stats ? `${p.stats.memories} memories · ${p.stats.entities} nodes` : "Indexing…"}</small>
            </span>
            <button
              className="icon-btn remove"
              title="Stop watching this folder"
              aria-label={`Stop watching ${p.name}`}
              onClick={(e) => { e.stopPropagation(); onRemove(p); }}
            ><CloseIcon size={13} /></button>
          </div>
        ))}
        {projects.length === 0 && (
          <button className="project-add" onClick={onAdd}><PlusIcon size={14} />Add a project folder</button>
        )}
      </div>

      <nav className="side-nav">
        <button className={`nav-item ${page === "settings" ? "active" : ""}`} onClick={() => onPage("settings")}><SettingsIcon size={15} />Settings</button>
        <button className={`nav-item ${page === "about" ? "active" : ""}`} onClick={() => onPage("about")}><InfoIcon size={15} />About</button>
      </nav>
      <div className="side-foot">
        <span className="dot" />
        {projects.length === 0 ? "Running locally" : `Watching ${projects.length} ${projects.length === 1 ? "project" : "projects"} locally`}
      </div>
    </aside>
  );
}
