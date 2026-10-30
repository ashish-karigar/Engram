import { useState } from "react";
import type { Api } from "../api.ts";
import type { ConnectResult } from "../types.ts";

const AGENTS = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "muse", label: "Muse Code" },
];

/** Shows exactly what will change before it changes anything. */
export function ConnectPanel({ api }: { api: Api }) {
  const [shown, setShown] = useState<ConnectResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = (agent: string, apply: boolean) =>
    api.post<ConnectResult>("/api/connect", { agent, apply }).then(
      (r) => { setShown(r); setError(null); },
      (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
    );

  return (
    <section>
      <p className="muted">Connect an agent to this project's memory. You will see the change before it is applied.</p>
      {AGENTS.map((a) => (
        <button key={a.id} className={shown?.agent === a.id ? "on" : ""} onClick={() => run(a.id, false)}>
          {a.label}
        </button>
      ))}
      {error && <p className="err">{error}</p>}
      {shown && (
        <>
          <pre>{shown.preview}</pre>
          {shown.result ? <p>{shown.result}</p> : <button className="primary" onClick={() => run(shown.agent, true)}>Apply</button>}
        </>
      )}
    </section>
  );
}
