export function EmptyState({ onAdd, error }: { onAdd(): void; error?: string | null }) {
  return (
    <div className="empty">
      <h1>Shared memory for your coding agents</h1>
      <p>Pick a project folder. Engram watches it, remembers decisions and their reasons, and hands that context to any agent you connect.</p>
      {error && <p className="err">{error}</p>}
      <button className="btn primary" onClick={onAdd}>Add a project folder</button>
    </div>
  );
}
