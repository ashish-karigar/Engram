import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import { useEffect, useState } from "react";
import type { ImpactResult, NodeDetail } from "../api/types.ts";

const day = (iso: string) => iso.slice(0, 10);

export function Type({ kind }: { kind: string }) {
  return <span className="badge" style={{ ["--dot" as string]: `var(--t-${kind})` }}>{kind.replace("_", " ")}</span>;
}

interface Props {
  api: ProjectApi;
  id: string;
  onClose(): void;
  onOpen(id: string): void;
}

export function NodeDrawer({ api, id, onClose, onOpen }: Props) {
  const { data } = usePolling<NodeDetail>(() => api.node(id), 4000, [id]);
  return (
    <div className="drawer">
      <button className="btn ghost close" onClick={onClose}>✕</button>
      {!data && <p className="muted">Loading…</p>}
      {data?.type === "entity" && (
        <>
          <Type kind={data.entity.kind} />
          <h3>{data.entity.qualifiedName}</h3>
          {data.entity.kind === "area" ? <PartBody api={api} path={data.entity.qualifiedName} /> : data.entity.kind === "module" ? (
            <p className="muted">Folder. Memories that touch it are highlighted in the graph.</p>
          ) : <ImpactBody api={api} target={data.entity.qualifiedName} />}
        </>
      )}
      {data?.type === "memory" && <MemoryBody detail={data} onOpen={onOpen} />}
      {data?.type === "document" && <DocumentBody detail={data} />}
    </div>
  );
}

/** What depends on this node: the files a change here reaches, grouped by part, and why. */
function ImpactBody({ api, target }: { api: ProjectApi; target: string }) {
  const [result, setResult] = useState<ImpactResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setResult(null);
    setError(null);
    api.impact(target).then((r) => setResult(r.data), (e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [api, target]);
  if (error) return <p className="muted">{error}</p>;
  if (!result) return <p className="muted">Working out the impact…</p>;
  const byArea = new Map<string, ImpactResult["affected"]>();
  for (const a of result.affected) byArea.set(a.area ?? "Outside any part", [...(byArea.get(a.area ?? "Outside any part") ?? []), a]);
  return (
    <>
      <h4>Impact of changing it</h4>
      <p className="muted">{result.affected.length === 0 ? "Nothing else in the project depends on it." : `${result.affected.length} file(s) depend on it.`}</p>
      {[...byArea.entries()].map(([area, items]) => (
        <div key={area} className="impact-area">
          <b>{area}</b>
          <ul className="impact-list">
            {items.map((a) => <li key={a.file}><code>{a.file}</code><span className="faint">{a.why}</span></li>)}
          </ul>
        </div>
      ))}
      {result.endpoints.length > 0 && (<><h4>Endpoints involved</h4><p className="muted">{result.endpoints.slice(0, 12).join(" · ")}{result.endpoints.length > 12 ? ` · +${result.endpoints.length - 12}` : ""}</p></>)}
      {result.decisions.length > 0 && (
        <>
          <h4>Recorded decisions on these files</h4>
          <ul className="impact-list">{result.decisions.map(([kind, title, claim]) => <li key={title}><Type kind={kind} /> <b>{title}</b>: {claim}</li>)}</ul>
        </>
      )}
    </>
  );
}

/** A part: the endpoints it serves and calls, from the system map. */
function PartBody({ api, path }: { api: ProjectApi; path: string }) {
  const { data } = usePolling(() => api.map(path === "." ? undefined : path), 10_000, [api, path]);
  if (!data) return <p className="muted">Loading…</p>;
  return <pre className="map-text">{data.text.split("\n").filter((l) => !l.startsWith("[project")).join("\n")}</pre>;
}

function MemoryBody({ detail, onOpen }: { detail: Extract<NodeDetail, { type: "memory" }>; onOpen(id: string): void }) {
  const { memory: m, history, evidence } = detail;
  return (
    <>
      <Type kind={m.type} />
      <h3>{m.title}</h3>
      <p>{m.claim}</p>
      {m.rationale && (<><h4>Why</h4><p>{m.rationale}</p></>)}
      <h4>Validity</h4>
      <p className="muted">
        {m.status} · {m.validTo ? `${day(m.validFrom)} → ${day(m.validTo)}` : `since ${day(m.validFrom)}`}
        {m.commitSha && <> · <code>{m.commitSha.slice(0, 7)}</code></>} · confidence {m.confidence.toFixed(2)}
      </p>
      {history.length > 1 && (
        <>
          <h4>History</h4>
          <ul className="versions">
            {history.map((v) => (
              <li key={v.id} className={v.status === "current" ? "current" : ""}>
                <a className="muted" onClick={() => onOpen(v.id)} style={{ cursor: "pointer" }}>v{v.version} · {v.status}</a>
                <div>{v.claim}</div>
              </li>
            ))}
          </ul>
        </>
      )}
      <h4>Evidence</h4>
      {evidence.length === 0 && <p className="faint">None recorded.</p>}
      <ul className="evidence">
        {evidence.map((e) => (
          <li key={e.seq}><code>#{e.seq}</code> {e.kind} <span className="faint">{e.ts.slice(0, 19).replace("T", " ")}</span></li>
        ))}
      </ul>
    </>
  );
}

function DocumentBody({ detail }: { detail: Extract<NodeDetail, { type: "document" }> }) {
  const { document: d, chunks, focus } = detail;
  return (
    <>
      <Type kind="document" />
      <h3>{d.title}</h3>
      <p className="muted">{d.source} · {d.mime} · {d.chunkCount} chunks · v{d.version} · {d.status}</p>
      {d.error && <p className="err">{d.error}</p>}
      <h4>Sections</h4>
      {chunks.map((c) => (
        <div key={c.id} className={`chunk ${c.id === focus ? "focus" : ""}`} ref={(el) => { if (el && c.id === focus) el.scrollIntoView({ block: "center" }); }}>
          <div>{c.heading ?? "(start)"} <span className="faint">{c.startLine ? `L${c.startLine}-${c.endLine}` : ""}</span></div>
          <pre>{c.preview}</pre>
        </div>
      ))}
    </>
  );
}
