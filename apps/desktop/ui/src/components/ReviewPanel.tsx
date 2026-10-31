import type { ProjectApi } from "../api/client.ts";
import type { InferredMemory } from "../api/types.ts";
import { usePolling } from "../hooks/usePolling.ts";

/** Inferred memories wait here until a person approves or declines them. */
export function ReviewPanel({ api }: { api: ProjectApi }) {
  const { data = [], reload } = usePolling<InferredMemory[]>(() => api.review(), 4000, [api]);
  const decide = (id: string, action: "approve" | "decline") => api.decide(id, action).then(reload);
  return (
    <div className="scroll">
      <p className="muted" style={{ marginBottom: 12 }}>Engram flags facts it inferred rather than read. Approve the right ones; declined guesses leave search.</p>
      <ul className="rows">
        {data.map(({ memory: m, parentCount }) => (
          <li key={m.id} className="row">
            <div className="head"><b>{m.title}</b><span className="faint">from {parentCount} memories</span></div>
            <p>{m.claim}</p>
            <div className="actions">
              <button className="btn primary" onClick={() => decide(m.id, "approve")}>Approve</button>
              <button className="btn" onClick={() => decide(m.id, "decline")}>Decline</button>
            </div>
          </li>
        ))}
        {data.length === 0 && <li className="row muted">Nothing to review.</li>}
      </ul>
    </div>
  );
}
