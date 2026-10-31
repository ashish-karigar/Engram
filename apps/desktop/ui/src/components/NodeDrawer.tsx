import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import { useEffect, useState } from "react";
import type { GraphNode, ImpactResult, NodeDetail, Provenance } from "../api/types.ts";
import { authorColor } from "./authorColor.ts";

const day = (iso: string) => iso.slice(0, 10);

export function Type({ kind }: { kind: string }) {
  return <span className="badge" style={{ ["--dot" as string]: `var(--t-${kind})` }}>{kind.replace("_", " ")}</span>;
}

interface Props {
  api: ProjectApi;
  id: string;
  onClose(): void;
  onOpen(id: string): void;
  /** The node as shown in the zoomable map, when it can be opened. */
  expandable?: GraphNode;
  onExpand?(id: string): void;
  /** Parts in the current map, to find which part an API body belongs to. */
  parts?: GraphNode[];
}

export function NodeDrawer({ api, id, onClose, onOpen, expandable, onExpand, parts = [] }: Props) {
  const folder = id.startsWith("dir:");
  const apiNode = id.startsWith("api:") || id.startsWith("apigroup:");
  const { data } = usePolling<NodeDetail>(() => (folder || apiNode ? Promise.resolve(null) : api.node(id)), 4000, [id]);
  const openButton = expandable && onExpand && (expandable.children ?? 0) > 0 && (
    <button className="btn" onClick={() => onExpand(id)}>{expandable.expanded ? "Close" : `Open (${expandable.children} inside)`}</button>
  );
  return (
    <div className="drawer">
      <button className="btn ghost close" onClick={onClose}>✕</button>
      {folder && (
        <>
          <Type kind="dir" />
          <h3>{id.slice(4)}</h3>
          {expandable?.owner && <p className="muted">Mostly written by {expandable.owner.who} ({Math.round(expandable.owner.share * 100)}% of {expandable.owner.lines.toLocaleString()} lines)</p>}
          {openButton}
        </>
      )}
      {apiNode && (
        <>
          <Type kind="api" />
          <h3>{expandable?.label ?? id}</h3>
          {openButton}
          <EndpointTable api={api} hubId={id} parts={parts} />
        </>
      )}
      {!folder && !apiNode && !data && <p className="muted">Loading…</p>}
      {data?.type === "entity" && (
        <>
          <Type kind={data.entity.kind} />
          <h3>{data.entity.qualifiedName}</h3>
          {data.signature && <pre className="signature">{data.signature}</pre>}
          {openButton}
          {data.provenance && data.provenance.lines > 0 && <ProvenanceBody p={data.provenance} onOpen={onOpen} />}
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

/** Who wrote it: lines per author or agent, when it was created and last changed, and the agent
 * sessions involved (each opens the conversation). */
function ProvenanceBody({ p, onOpen }: { p: Provenance; onOpen(id: string): void }) {
  const change = (c: NonNullable<Provenance["created"]>) => `${(c.ts ?? "").slice(0, 10)} · ${c.sha} · ${c.who}`;
  return (
    <>
      <h4>Who wrote it</h4>
      <ul className="authors">
        {p.contributors.map((c) => (
          <li key={c.who}>
            <span className="bar" style={{ width: `${Math.max(4, (100 * c.lines) / p.lines)}%`, background: authorColor(c.who) }} />
            <span>{c.who} <span className="faint">{c.lines} lines{c.evidence === "session" ? " · likely (was working here)" : c.evidence === "edited" ? " · edited these files" : ""}</span></span>
          </li>
        ))}
      </ul>
      {p.created && <p className="muted">Created {change(p.created)}<br />{p.created.subject}</p>}
      {p.lastChanged && p.lastChanged.sha !== p.created?.sha && <p className="muted">Last changed {change(p.lastChanged)}<br />{p.lastChanged.subject}</p>}
      {p.sessions.length > 0 && (
        <>
          <h4>Agent sessions</h4>
          <ul className="impact-list">
            {p.sessions.map((s) => (
              <li key={s.session}>
                {s.documentId ? <a style={{ cursor: "pointer" }} onClick={() => onOpen(s.documentId!)}>{s.agent} · open the conversation</a> : <span>{s.agent}</span>}
                <span className="faint">{s.session}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  );
}

/** The endpoints in an API body (or one resource group of it): method, path, handler, callers. */
function EndpointTable({ api, hubId, parts }: { api: ProjectApi; hubId: string; parts: GraphNode[] }) {
  const { data } = usePolling(() => api.map(), 10_000, [api]);
  if (!data) return <p className="muted">Loading…</p>;
  const map = data.data as { endpoints: { name: string; handlers: string[]; callers: string[] }[] };
  // `api:<part>` or `apigroup:api:<part>:/jobs`.
  const [hub, resource] = hubId.startsWith("apigroup:") ? [hubId.slice(9, hubId.lastIndexOf(":")), hubId.slice(hubId.lastIndexOf(":") + 1)] : [hubId, null];
  const partPath = parts.find((p) => `api:${p.id}` === hub)?.path;
  const short = (q: string) => q.split("/").pop() ?? q;
  const isTest = (q: string) => /(^|\/)tests?\//.test(q) || q.includes(".spec.") || /(^|\/)test_/.test(q);
  const rows = map.endpoints.filter((e) => {
    const served = e.handlers.length > 0;
    const inPart = hub === "api:unserved" ? !served : served && partPath !== undefined && e.handlers.some((h) => partPath === "" || h.startsWith(`${partPath}/`));
    const path = e.name.split(" ")[1] ?? "";
    return inPart && (!resource || path === resource || path.startsWith(`${resource}/`));
  });
  return (
    <table className="endpoints">
      <tbody>
        {rows.map((e) => {
          const [method, path] = e.name.split(" ");
          const callers = e.callers.filter((c) => !isTest(c));
          const tests = e.callers.length - callers.length;
          return (
            <tr key={e.name}>
              <td><code className={`method ${method?.toLowerCase()}`}>{method}</code></td>
              <td>
                <code>{path}</code>
                <div className="faint">{e.handlers.map(short).join(", ") || "no handler here"}</div>
                <div className="faint">← {callers.map(short).join(", ") || "no caller in the repo"}{tests ? ` · ${tests} test${tests > 1 ? "s" : ""}` : ""}</div>
              </td>
            </tr>
          );
        })}
        {rows.length === 0 && <tr><td className="muted">No endpoints.</td></tr>}
      </tbody>
    </table>
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
