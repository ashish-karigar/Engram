import type { ProjectApi } from "../api/client.ts";
import { usePolling } from "../hooks/usePolling.ts";
import type { EventEntry } from "../api/types.ts";

function summary(e: EventEntry): string {
  const p = e.payload as Record<string, any>;
  switch (e.kind) {
    case "git.commit": return `${String(p.sha).slice(0, 7)} ${p.subject}`;
    case "file.change": return (p.files as { path: string }[]).map((f) => f.path).join(", ");
    case "agent.note": return p.title;
    case "snapshot.created": return `Saved uncommitted work to ${p.ref}`;
    default: return "";
  }
}

export function Timeline({ api }: { api: ProjectApi }) {
  const { data = [] } = usePolling<EventEntry[]>(() => api.events(100), 2500);
  return (
    <div className="scroll">
      <ul className="rows">
        {[...data].reverse().map((e) => (
          <li key={e.seq} className="row">
            <div className="head"><code>{e.ts.slice(11, 19)}</code><b>{e.kind}</b></div>
            <div className="muted clip">{summary(e)}</div>
          </li>
        ))}
        {data.length === 0 && <li className="row muted">No events yet. Edit a file or make a commit.</li>}
      </ul>
    </div>
  );
}
