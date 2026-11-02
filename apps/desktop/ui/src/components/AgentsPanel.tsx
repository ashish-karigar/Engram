import { useCallback, useEffect, useState } from "react";
import type { projectsApi } from "../api/client.ts";
import type { ConnectResult } from "../api/types.ts";

type Api = ReturnType<typeof projectsApi>;

const AGENTS = [
  { id: "claude", name: "Claude Code" },
  { id: "codex", name: "Codex" },
  { id: "muse", name: "Muse Code" },
];

/** Each agent's status, read from its own config. Connecting is once per machine, not per project. */
export function useAgents(api: Api | null) {
  const [agents, setAgents] = useState<Record<string, ConnectResult>>({});
  const reload = useCallback(() => {
    if (!api) return;
    for (const a of AGENTS) api.connect(a.id, false).then((r) => setAgents((s) => ({ ...s, [a.id]: r })), () => {});
  }, [api]);
  useEffect(reload, [reload]);
  const anyConnected = Object.values(agents).some((r) => r.server);
  return { agents, anyConnected, loaded: Object.keys(agents).length === AGENTS.length, reload };
}

// The token is shown masked; Copy puts the real one on the clipboard.
const mask = (text: string) => text.replace(/egm_[0-9a-f]{8,}/g, "egm_••••••••");

function status(r: ConnectResult | undefined) {
  if (!r) return { label: "Checking…", cls: "faint" };
  if (r.server && r.hook) return { label: "Connected", cls: "ok" };
  if (r.server) return { label: "Connected, but without the session hook: it may not use the memory", cls: "warn" };
  return { label: "Not connected", cls: "faint" };
}

function AgentCard({ api, agent, state, onChange }: { api: Api; agent: (typeof AGENTS)[number]; state?: ConnectResult; onChange(): void }) {
  const [mode, setMode] = useState<"auto" | "manual" | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const s = status(state);

  const apply = () =>
    api.connect(agent.id, true).then(
      (r) => { setResult(r.result); setError(null); onChange(); },
      (e: Error) => setError(e.message),
    );
  const reset = async () => {
    const question = `Give ${agent.name} a new key? The current key stops working right away.`;
    const yes = (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
      ? await (await import("@tauri-apps/plugin-dialog")).ask(question, { title: "MemoryOS", kind: "warning", okLabel: "Reset key" })
      : window.confirm(question);
    if (!yes) return;
    api.resetKey(agent.id).then(
      (r) => { setResult(r.result); setError(null); setMode(null); onChange(); },
      (e: Error) => setError(e.message),
    );
  };
  const copy = () => {
    if (!state) return;
    void navigator.clipboard.writeText(state.manual).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); });
  };

  return (
    <div className="card">
      <div className="agent-head">
        <h3>{agent.name}</h3>
        <span className={`agent-status ${s.cls}`}>{s.label}</span>
      </div>
      <div className="actions">
        <button className={`btn ${mode === "auto" ? "on" : ""}`} onClick={() => { setMode("auto"); setResult(null); }}>Do it for me</button>
        <button className={`btn ${mode === "manual" ? "on" : ""}`} onClick={() => setMode("manual")}>Set it up yourself</button>
        <span className="spacer" />
        <button className="btn ghost" onClick={reset} title="Issue a new key and revoke the current one">Reset key</button>
      </div>
      {mode === "auto" && state && (
        <>
          <p className="faint">MemoryOS will make this change in {agent.name}'s settings, and keep a backup:</p>
          <pre>{state.preview}</pre>
          {!result && <div className="actions"><button className="btn primary" onClick={apply}>Apply</button></div>}
        </>
      )}
      {mode === "manual" && state && (
        <>
          <p className="faint">Nothing is changed for you. Follow these steps in {agent.name}:</p>
          <pre>{mask(state.manual)}</pre>
          <div className="actions"><button className="btn" onClick={copy}>{copied ? "Copied" : "Copy steps"}</button><button className="btn" onClick={onChange}>Check again</button></div>
          <p className="faint">The copied steps include this agent's access token. Keep it private.</p>
        </>
      )}
      {result && <p className="ok">{result}</p>}
      {error && <p className="err">{error}</p>}
    </div>
  );
}

export function AgentCards({ api, agents, onChange }: { api: Api; agents: Record<string, ConnectResult>; onChange(): void }) {
  return (
    <div className="agent-list">
      {AGENTS.map((a) => <AgentCard key={a.id} api={api} agent={a} state={agents[a.id]} onChange={onChange} />)}
    </div>
  );
}

/** The Agents page in the sidebar. */
export function AgentsPage({ api, agents, onChange }: { api: Api; agents: Record<string, ConnectResult>; onChange(): void }) {
  return (
    <section className="workspace">
      <header className="topbar" data-tauri-drag-region><h2>Agents</h2><span className="topbar-note">Once for all projects</span></header>
      <div className="content"><div className="scroll page">
        <p className="lead">Connect your coding agents to MemoryOS.</p>
        <p>Each agent is connected once and then reads and writes the memory of whichever project folder it is working in. Without a connected agent, MemoryOS still records commits and conversations, but no agent can use what it remembers.</p>
        <AgentCards api={api} agents={agents} onChange={onChange} />
      </div></div>
    </section>
  );
}
