import { useState } from "react";
import type { Api } from "../api.ts";
import { usePolling } from "../usePolling.ts";
import type { RequestEntry } from "../types.ts";

const brief = (r: unknown) => JSON.stringify(r).slice(0, 90);

/** Who asked for what, and why each memory came back. Click a row for the reasons. */
export function RequestLog({ api }: { api: Api }) {
  const { data = [] } = usePolling<RequestEntry[]>(() => api.get("/api/requests?limit=60"), 2500);
  const [open, setOpen] = useState<number | null>(null);
  return (
    <ul className="rows">
      {data.map((r) => (
        <li key={r.id} onClick={() => setOpen(open === r.id ? null : r.id)} className="click">
          <span className="muted">{r.ts.slice(11, 19)}</span> <b>{r.actor ?? "?"}</b> → {r.tool}
          <span className="muted"> · {r.returned.length} returned · {r.tokensOut ?? 0} tok · {r.latencyMs ?? 0} ms</span>
          {r.error && <span className="err"> · {r.error}</span>}
          <div className="clip muted">{brief(r.request)}</div>
          {open === r.id && (
            <ul className="why">
              {r.returned.map((x) => (
                <li key={x.id}>
                  <code>{x.id}</code> {x.score !== undefined && `score ${x.score}`} {x.reason}
                </li>
              ))}
            </ul>
          )}
        </li>
      ))}
      {data.length === 0 && <li className="muted">No agent requests yet.</li>}
    </ul>
  );
}
