import { Mark } from "./icons.tsx";

export function EmptyState({ onAdd, error }: { onAdd(): void; error?: string | null }) {
  return (
    <section className="workspace">
      <div className="welcome" data-tauri-drag-region>
        <Mark size={28} />
        <h1>Shared memory for your coding agents</h1>
        <p>Add a project folder. MemoryOS watches it, remembers decisions and their reasons, and gives that context to every agent you connect.</p>
        {error && <p className="err">{error}</p>}
        <button className="btn primary" onClick={onAdd}>Add project folder</button>
      </div>
    </section>
  );
}
