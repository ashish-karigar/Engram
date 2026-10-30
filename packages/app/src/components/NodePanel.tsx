import type { Api } from "../api.ts";
import { usePolling } from "../usePolling.ts";
import type { NodeDetail } from "../types.ts";

const day = (iso: string) => iso.slice(0, 10);

export function NodePanel({ api, id }: { api: Api; id: string | null }) {
  const { data } = usePolling<NodeDetail>(() => (id ? api.get(`/api/node?id=${encodeURIComponent(id)}`) : Promise.resolve(null)), 3000, [id]);
  if (!id) return <p className="muted">Select a node in the graph to see its reasons, history and evidence.</p>;
  if (!data) return <p className="muted">Loading…</p>;
  if (data.type === "entity") {
    return (
      <section>
        <h3>{data.entity.qualifiedName}</h3>
        <p className="muted">{data.entity.kind}</p>
      </section>
    );
  }
  const { memory: m, history, evidence } = data;
  return (
    <section>
      <span className={`badge ${m.type}`}>{m.type.replace("_", " ")}</span>
      <h3>{m.title}</h3>
      <p>{m.claim}</p>
      {m.rationale && (
        <>
          <h4>Why</h4>
          <p>{m.rationale}</p>
        </>
      )}
      <p className="muted">
        {m.status} · {m.validTo ? `${day(m.validFrom)} → ${day(m.validTo)}` : `since ${day(m.validFrom)}`}
        {m.commitSha && ` · ${m.commitSha.slice(0, 7)}`} · confidence {m.confidence.toFixed(2)}
      </p>
      {history.length > 1 && (
        <>
          <h4>History</h4>
          <ul>
            {history.map((v) => (
              <li key={v.id}>
                v{v.version} <span className="muted">[{v.status}]</span> {v.claim}
              </li>
            ))}
          </ul>
        </>
      )}
      <h4>Evidence</h4>
      {evidence.length === 0 && <p className="muted">None recorded.</p>}
      <ul>
        {evidence.map((e) => (
          <li key={e.seq}>
            <code>#{e.seq}</code> {e.kind} <span className="muted">{e.ts.slice(0, 19)}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
