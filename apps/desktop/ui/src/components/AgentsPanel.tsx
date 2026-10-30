import { useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import type { ConnectResult, RequestEntry } from "../api/types.ts";

const AGENTS = [
  { id: "claude", name: "Claude Code", actor: "claude-code", note: "Registers an MCP server with the claude CLI." },
  { id: "codex", name: "Codex", actor: "codex", note: "Adds a marked block to ~/.codex/config.toml." },
  { id: "muse", name: "Muse Code", actor: "muse-code", note: "Adds an MCP entry to ~/.config/muse/settings.json." },
];

function AgentCard({ api, agent, requests }: { api: ProjectApi; agent: (typeof AGENTS)[number]; requests: RequestEntry[] }) {
  const [shown, setShown] = useState<ConnectResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const calls = requests.filter((r) => r.actor === agent.actor);

  const run = (apply: boolean) =>
    api.connect(agent.id, apply).then(
      (r) => { setShown(r); setError(null); },
      (e: Error) => setError(e.message),
    );

  return (
    <div className="card">
      <h3>{agent.name}</h3>
      <p className="muted">{agent.note}</p>
      <p className="faint">{calls.length ? `${calls.length} recent requests · last ${calls[0]!.ts.slice(11, 19)}` : "No requests yet."}</p>
      {shown && <pre>{shown.preview}</pre>}
      {shown?.result && <p className="muted">{shown.result}</p>}
      {error && <p className="err">{error}</p>}
      <div className="actions">
        <button className="btn" onClick={() => run(false)}>{shown ? "Refresh preview" : "Preview change"}</button>
        {shown && !shown.result && <button className="btn primary" onClick={() => run(true)}>Apply</button>}
      </div>
    </div>
  );
}

export function AgentsPanel({ api }: { api: ProjectApi }) {
  const { data = [] } = usePolling<RequestEntry[]>(() => api.requests(200), 3000);
  return (
    <div className="scroll">
      <p className="muted" style={{ marginBottom: 14 }}>Connect an agent to this project's memory. You always see the exact change before it is applied.</p>
      <div className="agent-grid">
        {AGENTS.map((a) => <AgentCard key={a.id} api={api} agent={a} requests={data} />)}
      </div>
    </div>
  );
}
