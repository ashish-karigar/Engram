import { useCallback, useEffect, useMemo, useState } from "react";
import { projectApi, projectsApi } from "./api/client.ts";
import type { Connection, Graph, Project } from "./api/types.ts";
import { AgentsPanel } from "./components/AgentsPanel.tsx";
import { AskBar } from "./components/AskBar.tsx";
import { EmptyState } from "./components/EmptyState.tsx";
import { GraphView, Legend } from "./components/GraphView.tsx";
import { NodeDrawer } from "./components/NodeDrawer.tsx";
import { RequestLog } from "./components/RequestLog.tsx";
import { Sidebar } from "./components/Sidebar.tsx";
import { Timeline } from "./components/Timeline.tsx";
import { pickFolder, useDaemon } from "./hooks/useDaemon.ts";
import { usePolling } from "./hooks/usePolling.ts";

type Tab = "graph" | "timeline" | "requests" | "agents";
const TABS: [Tab, string][] = [["graph", "Memory graph"], ["timeline", "Timeline"], ["requests", "Requests"], ["agents", "Agents"]];
const LAST_PROJECT = "engram.lastProject";

function Workspace({ connection, project }: { connection: Connection; project: Project }) {
  const api = useMemo(() => projectApi(connection, project.id), [connection, project.id]);
  const [tab, setTab] = useState<Tab>("graph");
  const [selected, setSelected] = useState<string | null>(null);
  const [archived, setArchived] = useState(false);
  const { data: graph, error } = usePolling<Graph>(() => api.graph(archived), 3000, [api, archived]);
  const { data: stats } = usePolling(() => api.stats(), 3000, [api]);
  const select = useCallback((id: string | null) => setSelected(id), []);

  return (
    <section className="workspace">
      <div className="topbar" data-tauri-drag-region>
        <div className="title">
          <h2 className="clip">{project.name}</h2>
          <div className="path clip">{project.root}</div>
        </div>
        <AskBar api={api} onPick={(id) => { setTab("graph"); setSelected(id); }} />
        {stats && (
          <div className="chips">
            <span className="chip"><b>{stats.memories}</b>memories</span>
            <span className="chip"><b>{stats.entities}</b>code nodes</span>
            <span className="chip"><b>{stats.requests}</b>requests</span>
          </div>
        )}
      </div>
      <nav className="tabs">
        {TABS.map(([id, label]) => (
          <button key={id} className={`tab ${tab === id ? "active" : ""}`} onClick={() => setTab(id)}>{label}</button>
        ))}
      </nav>
      <div className="content">
        {tab === "graph" && (
          <>
            {error && <div className="banner err">{error}</div>}
            {graph && <GraphView graph={graph} selectedId={selected} onSelect={select} />}
            <div className="graph-tools">
              <label><input type="checkbox" checked={archived} onChange={(e) => setArchived(e.target.checked)} /> show archived</label>
            </div>
            <Legend />
            {selected && <NodeDrawer api={api} id={selected} onClose={() => setSelected(null)} onOpen={setSelected} />}
          </>
        )}
        {tab === "timeline" && <Timeline api={api} />}
        {tab === "requests" && <RequestLog api={api} />}
        {tab === "agents" && <AgentsPanel api={api} />}
      </div>
    </section>
  );
}

export function App() {
  const { connection, error: daemonError } = useDaemon();
  const [selectedId, setSelectedId] = useState<string | null>(() => localStorage.getItem(LAST_PROJECT));
  const [addError, setAddError] = useState<string | null>(null);
  const api = useMemo(() => (connection ? projectsApi(connection) : null), [connection]);
  const { data: projects = [], reload } = usePolling<Project[]>(() => (api ? api.list() : Promise.resolve([])), 3000, [api]);

  const current = projects.find((p) => p.id === selectedId) ?? projects[0] ?? null;
  useEffect(() => { if (current) localStorage.setItem(LAST_PROJECT, current.id); }, [current]);

  const add = async () => {
    const root = await pickFolder();
    if (!root || !api) return;
    api.add(root).then(
      (p) => { setAddError(null); setSelectedId(p.id); reload(); },
      (e: Error) => setAddError(e.message),
    );
  };
  const remove = (id: string) => api?.remove(id).then(reload);

  if (daemonError) return <div className="empty"><h1>Engram</h1><p className="err">{daemonError}</p></div>;
  if (!connection) return <div className="empty"><p>Starting…</p></div>;

  return (
    <div className="shell">
      <Sidebar projects={projects} selectedId={current?.id ?? null} onSelect={setSelectedId} onAdd={add} onRemove={remove} />
      {current ? <Workspace key={current.id} connection={connection} project={current} /> : <EmptyState onAdd={add} error={addError} />}
    </div>
  );
}
