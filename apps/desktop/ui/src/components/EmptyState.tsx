import { FolderIcon, Logo } from "./icons.tsx";

const STEPS = [
  ["Add a folder", "Engram indexes the code and watches it for changes."],
  ["Connect an agent", "Claude Code, Codex and others read and write memory over MCP."],
  ["Keep working", "Decisions and their reasons are remembered for the next session."],
];

export function EmptyState({ onAdd, error }: { onAdd(): void; error?: string | null }) {
  return (
    <section className="workspace">
      <div className="welcome" data-tauri-drag-region>
        <Logo size={56} />
        <h1>Shared memory for your coding agents</h1>
        <p className="muted">Pick a project folder. Engram watches it, remembers decisions and their reasons, and hands that context to any agent you connect.</p>
        {error && <p className="err">{error}</p>}
        <button className="btn primary big" onClick={onAdd}><FolderIcon size={16} />Add a project folder</button>
        <ol className="steps">
          {STEPS.map(([title, text], i) => (
            <li key={title}><span>{i + 1}</span><b>{title}</b><small>{text}</small></li>
          ))}
        </ol>
      </div>
    </section>
  );
}
