import { useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import type { RequestEntry } from "../api/types.ts";
import { agentName } from "./timeline/Entry.tsx";

/** Who asked for what, and why each memory came back. Click a row for the reasons. */
export function RequestLog({ api }: { api: ProjectApi }) {
  const { data = [] } = usePolling<RequestEntry[]>(() => api.requests(100), 2500);
  const [open, setOpen] = useState<number | null>(null);
  return (
    <div className="scroll">
      <ul className="rows">
        {data.map((r) => (
          <li key={r.id} className="row click" onClick={() => setOpen(open === r.id ? null : r.id)}>
            <div className="head">
              <code>{r.ts.slice(11, 19)}</code>
              <b>{r.actor ? agentName(r.actor) : "?"}</b>
              <span>called <b>{r.tool}</b></span>
              <span className="faint">{r.returned.length} returned · {r.tokensOut ?? 0} tok · {r.latencyMs ?? 0} ms</span>
              {r.error && <span className="err">{r.error}</span>}
            </div>
            <div className="muted clip">{JSON.stringify(r.request)}</div>
            {open === r.id && (
              <ul className="why">
                {r.returned.map((x) => (
                  <li key={x.id}><code>{x.id}</code> {x.score !== undefined && <span className="muted">score {x.score} · </span>}{x.reason}</li>
                ))}
                {r.returned.length === 0 && <li className="faint">Nothing returned.</li>}
              </ul>
            )}
          </li>
        ))}
        {data.length === 0 && <li className="row muted">No agent requests yet. Connect an agent in the Agents tab.</li>}
      </ul>
    </div>
  );
}
