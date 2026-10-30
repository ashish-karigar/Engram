import type { Api } from "../api.ts";
import { usePolling } from "../usePolling.ts";
import type { EventEntry } from "../types.ts";

function summary(e: EventEntry): string {
  const p = e.payload as Record<string, any>;
  switch (e.kind) {
    case "git.commit": return `${String(p.sha).slice(0, 7)} ${p.subject}`;
    case "file.change": return (p.files as { path: string }[]).map((f) => f.path).join(", ");
    case "agent.note": return p.title;
    case "snapshot.created": return `saved uncommitted work on ${p.ref}`;
    default: return "";
  }
}

export function Timeline({ api }: { api: Api }) {
  const { data = [] } = usePolling<EventEntry[]>(() => api.get("/api/events?limit=80"), 2500);
  return (
    <ul className="rows">
      {[...data].reverse().map((e) => (
        <li key={e.seq}>
          <span className="muted">{e.ts.slice(11, 19)}</span> <b>{e.kind}</b>
          <div className="clip">{summary(e)}</div>
        </li>
      ))}
      {data.length === 0 && <li className="muted">No events yet.</li>}
    </ul>
  );
}
