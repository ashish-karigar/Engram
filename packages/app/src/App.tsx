import { useCallback, useMemo, useState } from "react";
import { connectionFromQuery, createApi } from "./api.ts";
import { ConnectPanel } from "./components/ConnectPanel.tsx";
import { GraphView, Legend } from "./components/GraphView.tsx";
import { NodePanel } from "./components/NodePanel.tsx";
import { RequestLog } from "./components/RequestLog.tsx";
import { Timeline } from "./components/Timeline.tsx";
import type { Connection, Graph, Stats } from "./types.ts";
import { usePolling } from "./usePolling.ts";

type Tab = "node" | "timeline" | "requests" | "connect";
const TABS: [Tab, string][] = [["node", "Memory"], ["timeline", "Timeline"], ["requests", "Requests"], ["connect", "Connect"]];

function Welcome({ onConnect }: { onConnect(c: Connection): void }) {
  const pick = () => window.engram?.pickProject().then((c) => c && onConnect(c));
  return (
    <div className="welcome">
      <h1>Engram</h1>
      <p>Shared memory for your coding agents. Choose a project folder to start.</p>
      {window.engram ? <button className="primary" onClick={pick}>Choose project folder</button> : <p className="muted">Open with ?url=…&amp;token=… to connect to a running daemon.</p>}
    </div>
  );
}

function Workspace({ connection }: { connection: Connection }) {
  const api = useMemo(() => createApi(connection), [connection]);
  const [selected, setSelected] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("node");
  const [archived, setArchived] = useState(false);
  const onSelect = useCallback((id: string | null) => { setSelected(id); if (id) setTab("node"); }, []);

  const { data: graph, error } = usePolling<Graph>(() => api.get(`/api/graph?archived=${archived ? 1 : 0}`), 3000);
  const { data: stats } = usePolling<Stats>(() => api.get("/api/stats"), 3000);

  return (
    <div className="app">
      <header>
        <b>Engram</b>
        <span className="muted clip">{connection.root}</span>
        {stats && <span className="stats">{stats.memories} memories · {stats.entities} code nodes · {stats.events} events · {stats.requests} requests</span>}
        <label><input type="checkbox" checked={archived} onChange={(e) => setArchived(e.target.checked)} /> archived</label>
      </header>
      <main>
        <div className="graph-wrap">
          {error && <div className="banner err">{error}</div>}
          {graph && <GraphView graph={graph} onSelect={onSelect} />}
          <Legend />
        </div>
        <aside>
          <nav>{TABS.map(([id, label]) => <button key={id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>{label}</button>)}</nav>
          {tab === "node" && <NodePanel api={api} id={selected} />}
          {tab === "timeline" && <Timeline api={api} />}
          {tab === "requests" && <RequestLog api={api} />}
          {tab === "connect" && <ConnectPanel api={api} />}
        </aside>
      </main>
    </div>
  );
}

export function App() {
  const [connection, setConnection] = useState<Connection | null>(connectionFromQuery);
  return connection ? <Workspace connection={connection} /> : <Welcome onConnect={setConnection} />;
}
