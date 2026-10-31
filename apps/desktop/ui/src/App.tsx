import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectApi, projectsApi } from "./api/client.ts";
import type { Connection, Graph, Project } from "./api/types.ts";
import { AgentsPanel } from "./components/AgentsPanel.tsx";
import { AskBar } from "./components/AskBar.tsx";
import { DocumentsPanel } from "./components/DocumentsPanel.tsx";
import { ReviewPanel } from "./components/ReviewPanel.tsx";
import { EmptyState } from "./components/EmptyState.tsx";
import { ForceGraph, type GraphCommands } from "./components/graph/ForceGraph.tsx";
import { GraphControls } from "./components/graph/GraphControls.tsx";

import { loadSettings, saveSettings, type GraphSettings } from "./components/graph/settings.ts";
import { NodeDrawer } from "./components/NodeDrawer.tsx";
import { RequestLog } from "./components/RequestLog.tsx";
import { SettingsPage } from "./components/SettingsPage.tsx";
import { Sidebar } from "./components/Sidebar.tsx";
import { Timeline } from "./components/timeline/Timeline.tsx";
import { pickFolder, useDaemon } from "./hooks/useDaemon.ts";
import { usePolling } from "./hooks/usePolling.ts";

type Tab = "graph" | "documents" | "review" | "timeline" | "requests" | "agents";
const TABS: [Tab, string][] = [
  ["graph", "Memory graph"], ["documents", "Documents"], ["review", "Review"], ["timeline", "Timeline"], ["requests", "Requests"], ["agents", "Agents"],
];
const LAST_PROJECT = "engram.lastProject";

function Workspace({ connection, project }: { connection: Connection; project: Project }) {
  const api = useMemo(() => projectApi(connection, project.id), [connection, project.id]);
  const [tab, setTab] = useState<Tab>("graph");
  const [selected, setSelected] = useState<string | null>(null);
  const [graphSettings, setGraphSettings] = useState<GraphSettings>(loadSettings);
  const [zoom, setZoom] = useState(1);
  const commands = useRef<GraphCommands | null>(null);
  const updateGraphSettings = useCallback((s: GraphSettings) => { setGraphSettings(s); saveSettings(s); }, []);
  const archived = graphSettings.showArchived;
  const view = graphSettings.view;
  // The architecture view is the zoomable map: which nodes are open is kept per project.
  const [expanded, setExpanded] = useState<string[]>([]);
  useEffect(() => setExpanded([]), [api]);
  const toggle = useCallback((id: string) => setExpanded((e) => (e.includes(id) ? e.filter((x) => x !== id) : [...e, id])), []);
  const { data: graph, error } = usePolling<Graph>(() => (view === "architecture" ? api.zoom(expanded) : api.graph(archived)), 3000, [api, archived, view, expanded.join(",")]);
  // On first load, open every part one level so each shows as a galaxy of its folders.
  const opened = useRef<string | null>(null);
  useEffect(() => {
    if (view !== "architecture" || !graph || opened.current === project.id) return;
    opened.current = project.id;
    const parts = graph.nodes.filter((n) => n.kind === "area").map((n) => n.id);
    if (parts.length) setExpanded(parts);
  }, [graph, view, project.id]);
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
            <span className="chip"><b>{stats.documents}</b>documents</span>
            <span className="chip"><b>{stats.entities}</b>code nodes</span>
            <span className="chip"><b>{stats.requests}</b>requests</span>
          </div>
        )}
      </div>
      <nav className="tabs">
        {TABS.map(([id, label]) => (
          <button key={id} className={`tab ${tab === id ? "active" : ""}`} onClick={() => setTab(id)}>
            {label}{id === "review" && stats?.inferred ? <span className="count">{stats.inferred}</span> : null}
          </button>
        ))}
      </nav>
      <div className="content">
        {tab === "graph" && (
          <div className={`graph ${selected ? "with-drawer" : ""}`}>
            {error && <div className="banner err">{error}</div>}
            {graph && <ForceGraph graph={graph} settings={graphSettings} selectedId={selected} onSelect={select} onExpand={view === "architecture" ? toggle : undefined} onZoom={setZoom} commands={commands} />}
            <GraphControls graph={graph} settings={graphSettings} onChange={updateGraphSettings} selectedId={selected} zoom={zoom} commands={commands} />
            {graph && graph.nodes.length === 0 && <div className="graph-empty">No memories yet. Connect an agent, or add documents.</div>}
            {selected && (
              <NodeDrawer api={api} id={selected} onClose={() => setSelected(null)} onOpen={setSelected}
                expandable={view === "architecture" ? graph?.nodes.find((n) => n.id === selected) : undefined} onExpand={toggle}
                parts={graph?.nodes.filter((n) => n.kind === "area") ?? []} />
            )}
          </div>
        )}
        {tab === "documents" && <DocumentsPanel api={api} onOpen={(id) => { setSelected(id); setTab("graph"); }} />}
        {tab === "review" && <ReviewPanel api={api} />}
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
  const [settingsOpen, setSettingsOpen] = useState(false);
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
      <Sidebar
        projects={projects}
        selectedId={current?.id ?? null}
        onSelect={(id) => { setSelectedId(id); setSettingsOpen(false); }}
        onAdd={add}
        onRemove={remove}
        settingsOpen={settingsOpen}
        onSettings={() => setSettingsOpen(true)}
      />
      {settingsOpen && api ? <SettingsPage api={api} />
        : current ? <Workspace key={current.id} connection={connection} project={current} />
        : <EmptyState onAdd={add} error={addError} />}
    </div>
  );
}
