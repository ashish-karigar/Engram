import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import type { NodeDetail } from "../api/types.ts";

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
          <p className="muted">Code node. Memories that touch it are highlighted in the graph.</p>
        </>
      )}
      {data?.type === "memory" && <MemoryBody detail={data} onOpen={onOpen} />}
    </div>
  );
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
