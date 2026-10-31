import { useMemo, useRef, useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import type { DocumentView } from "../api/types.ts";
import { usePolling } from "../hooks/usePolling.ts";

type Filter = "all" | "docs" | "code" | "sessions" | "added";

const isCode = (d: DocumentView) => d.mime.startsWith("text/x-");
const FILTERS: [Filter, string, (d: DocumentView) => boolean][] = [
  ["all", "All", () => true],
  ["docs", "Docs", (d) => !isCode(d) && d.source === "project"],
  ["code", "Code", isCode],
  ["sessions", "Conversations", (d) => d.source === "session"],
  ["added", "Added", (d) => d.source !== "project" && d.source !== "session"],
];

/** Everything Engram can quote from: project docs and code (indexed automatically) and added documents. */
export function DocumentsPanel({ api, onOpen }: { api: ProjectApi; onOpen(id: string): void }) {
  const { data = [], reload } = usePolling<DocumentView[]>(() => api.documents(), 4000, [api]);
  const [filter, setFilter] = useState<Filter>("all");
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const shown = useMemo(() => data.filter(FILTERS.find(([f]) => f === filter)![2]), [data, filter]);

  const addFiles = async (files: FileList | null) => {
    for (const f of Array.from(files ?? [])) {
      if (f.name.toLowerCase().endsWith(".pdf")) { setError("PDFs: drop them into the project folder; they are indexed automatically."); continue; }
      await api.addDocument(f.name, await f.text()).catch((e: Error) => setError(e.message));
    }
    reload();
  };

  return (
    <div className="scroll">
      <div className="doc-head">
        <div className="seg">
          {FILTERS.map(([id, label, test]) => (
            <button key={id} className={filter === id ? "on" : ""} onClick={() => setFilter(id)}>
              {label} <span className="faint">{data.filter(test).length}</span>
            </button>
          ))}
        </div>
        <button className="btn" onClick={() => file.current?.click()}>Upload files</button>
        <button className="btn primary" onClick={() => setAdding(!adding)}>{adding ? "Close" : "New note"}</button>
        <input ref={file} type="file" multiple hidden onChange={(e) => addFiles(e.target.files)} />
      </div>
      {adding && <NewDocument api={api} onDone={() => { setAdding(false); reload(); }} />}
      {error && <p className="err">{error}</p>}
      <ul className="rows">
        {shown.map((d) => (
          <li key={d.id} className="row click" onClick={() => onOpen(d.id)}>
            <div className="head">
              <span className={`status ${d.status}`} title={d.error ?? d.status} />
              <b className="clip">{d.title}</b>
              <span className="faint">{d.chunkCount} chunks · {d.charCount < 1000 ? `${d.charCount} chars` : `${Math.round(d.charCount / 1000)}k chars`} · v{d.version}</span>
              <span className="chip small">{isCode(d) ? d.mime.replace("text/x-", "") : d.source}</span>
              {d.source !== "project" && d.source !== "session" && (
                <button className="btn ghost danger" onClick={(e) => { e.stopPropagation(); api.deleteDocument(d.id).then(reload); }}>Remove</button>
              )}
            </div>
            {d.error && <div className="err">{d.error}</div>}
          </li>
        ))}
        {shown.length === 0 && <li className="row muted">Nothing here yet. Markdown, text, HTML, PDF and source files in the project folder are indexed automatically.</li>}
      </ul>
    </div>
  );
}

function NewDocument({ api, onDone }: { api: ProjectApi; onDone(): void }) {
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const save = () => title.trim() && content.trim() && api.addDocument(title.trim(), content).then(onDone);
  return (
    <div className="card new-doc">
      <input className="field" placeholder="Title, e.g. auth-migration-plan.md" value={title} onChange={(e) => setTitle(e.target.value)} />
      <textarea className="field" rows={8} placeholder="Paste a spec, design note or meeting notes. Markdown headings become sections." value={content} onChange={(e) => setContent(e.target.value)} />
      <div className="actions"><button className="btn primary" onClick={save}>Save to memory</button></div>
    </div>
  );
}
