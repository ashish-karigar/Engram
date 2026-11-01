import type { Project } from "../api/types.ts";
import { Credit } from "./Credit.tsx";
import { CloseIcon, FolderIcon, InfoIcon, Mark, PlusIcon, SettingsIcon } from "./icons.tsx";

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

export function Sidebar({ projects, selectedId, page, onSelect, onAdd, onRemove, onPage }: Props) {
  return (
    <aside className="sidebar">
      {/* Room for the window's traffic lights, and a handle to move the window. */}
      <div className="side-drag" data-tauri-drag-region />
      <div className="brand" data-tauri-drag-region><Mark size={15} /><span>MemoryOS</span></div>
      <Credit className="side-credit" />

      <div className="side-label">
        <span>Projects</span>
        <button className="icon-btn" onClick={onAdd} title="Add a project folder" aria-label="Add a project folder"><PlusIcon size={14} /></button>
      </div>
      <div className="projects">
        {projects.map((p) => (
          <div
            key={p.id}
            role="button"
            tabIndex={0}
            className={`side-item ${p.id === selectedId && page === "project" ? "active" : ""}`}
            onClick={() => onSelect(p.id)}
            onKeyDown={(e) => { if (e.key === "Enter") onSelect(p.id); }}
            title={p.stats ? `${p.root}\n${p.stats.memories} memories · ${p.stats.entities} code nodes` : p.root}
          >
            <FolderIcon size={14} />
            <span className="clip">{p.name}</span>
            <button
              className="icon-btn remove"
              title="Stop watching this folder"
              aria-label={`Stop watching ${p.name}`}
              onClick={(e) => { e.stopPropagation(); onRemove(p); }}
            ><CloseIcon size={12} /></button>
          </div>
        ))}
        {projects.length === 0 && <button className="side-item quiet" onClick={onAdd}><PlusIcon size={14} />Add a project</button>}
      </div>

      <nav className="side-nav">
        <button className={`side-item ${page === "settings" ? "active" : ""}`} onClick={() => onPage("settings")}><SettingsIcon size={14} />Settings</button>
        <button className={`side-item ${page === "about" ? "active" : ""}`} onClick={() => onPage("about")}><InfoIcon size={14} />About</button>
      </nav>
    </aside>
  );
}
