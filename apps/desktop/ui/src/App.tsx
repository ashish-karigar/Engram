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
import { AboutPage } from "./components/AboutPage.tsx";
import { RequestLog } from "./components/RequestLog.tsx";
import { SettingsPage } from "./components/SettingsPage.tsx";
import { Sidebar, type Page } from "./components/Sidebar.tsx";
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
  const { data: graph, error } = usePolling<Graph>(() => api.graph(archived), 3000, [api, archived]);
  const { data: stats } = usePolling(() => api.stats(), 3000, [api]);
  // The drawer and the selection are separate: closing the drawer keeps the node selected (and its
  // traced flow lit); clicking empty canvas or pressing Escape twice clears it.
  const [drawer, setDrawer] = useState(false);
  const select = useCallback((id: string | null) => { setSelected(id); setDrawer(id !== null); }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || (e.target as HTMLElement).closest("input, textarea, select")) return;
      if (drawer) setDrawer(false);
      else if (selected) setSelected(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawer, selected]);

  return (
    <section className="workspace">
      <div className="topbar" data-tauri-drag-region>
        <div className="title">
          <h2 className="clip">{project.name}</h2>
          <div className="path clip">{project.root}</div>
        </div>
        <AskBar api={api} onPick={(id) => { setTab("graph"); select(id); }} />
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
          <div className={`graph ${selected && drawer ? "with-drawer" : ""}`}>
            {error && <div className="banner err">{error}</div>}
            {graph && <ForceGraph graph={graph} settings={graphSettings} selectedId={selected} onSelect={select} onZoom={setZoom} commands={commands} />}
            <GraphControls graph={graph} settings={graphSettings} onChange={updateGraphSettings} selectedId={selected} zoom={zoom} commands={commands} />
            {graph && graph.nodes.length === 0 && <div className="graph-empty">No memories yet. Connect an agent, or add documents.</div>}
            {selected && drawer && <NodeDrawer api={api} id={selected} onClose={() => setDrawer(false)} onOpen={select} />}
          </div>
        )}
        {tab === "documents" && <DocumentsPanel api={api} onOpen={(id) => { select(id); setTab("graph"); }} />}
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
  const [page, setPage] = useState<Page>("project");
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
  // Stopping is reversible (the memory stays on disk), but it still asks: it is one click away in the list.
  const remove = async (p: Project) => {
    const question = `Stop watching ${p.name}? Its memory stays on disk and comes back if you add the folder again.`;
    const yes = (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
      ? await (await import("@tauri-apps/plugin-dialog")).ask(question, { title: "Engram", kind: "warning", okLabel: "Stop watching" })
      : window.confirm(question);
    if (yes) api?.remove(p.id).then(reload);
  };

  if (daemonError) return <div className="empty" data-tauri-drag-region><h1>Engram</h1><p className="err">{daemonError}</p></div>;
  if (!connection) return <div className="empty" data-tauri-drag-region><p className="muted">Starting…</p></div>;

  return (
    <div className="shell">
      {/* The title bar is drawn by the page: this strip along the top moves the window from anywhere. */}
      <div className="drag-strip" data-tauri-drag-region />
      <Sidebar
        projects={projects}
        selectedId={current?.id ?? null}
        page={page}
        onSelect={(id) => { setSelectedId(id); setPage("project"); }}
        onAdd={add}
        onRemove={remove}
        onPage={setPage}
      />
      {page === "settings" && api ? <SettingsPage api={api} />
        : page === "about" ? <AboutPage connection={connection} projects={projects.length} />
        : current ? <Workspace key={current.id} connection={connection} project={current} />
        : <EmptyState onAdd={add} error={addError} />}
    </div>
  );
}
