import type { ActivityItem, SessionMessage } from "../../api/types.ts";
import { agentColor } from "./Gutter.tsx";

// The app's own actors keep their stored names from before the rename; only the label changed.
const AGENTS: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  "muse-code": "Muse Code",
  "engram-ui": "MemoryOS app",
  "engram-extractor": "MemoryOS extractor",
  "engram-watcher": "MemoryOS watcher",
};
export const agentName = (id: string) => AGENTS[id] ?? id;

export const clock = (iso: string) => new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

function span(i: ActivityItem) {
  const [a, b] = [clock(i.start), clock(i.end)];
  return a === b ? a : `${a} – ${b}`;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

function FileList({ files, total }: { files: string[]; total?: number }) {
  return (
    <div className="tl-files">
      {files.map((f) => <code key={f}>{f}</code>)}
      {total !== undefined && total > files.length && <span className="faint">+{total - files.length} more</span>}
    </div>
  );
}

function Message({ m }: { m: SessionMessage }) {
  if (m.role === "compacted") return <div className="tl-msg compacted">Context compacted · {clock(m.ts)}</div>;
  return (
    <div className={`tl-msg ${m.role}`}>
      <span className="who">{m.role === "user" ? "You" : "Agent"}</span>
      <p>{m.text}</p>
    </div>
  );
}

const SYSTEM_LABELS: Record<string, string> = { "snapshot.created": "uncommitted-work snapshot", "document.ingested": "document indexed" };

/** One timeline row's content. Sessions and commits expand on click. */
export function Entry({ item, open }: { item: ActivityItem; open: boolean }) {
  switch (item.kind) {
    case "session": {
      const stats = [plural(item.userMessages + item.agentMessages, "message"), plural(item.toolCalls, "tool call"), plural(item.files.length, "file")];
      if (item.compactions) stats.push(`compacted ${item.compactions}×`);
      return (
        <>
          <div className="tl-head">
            <span className="tl-agent" style={{ color: agentColor(item.agent), borderColor: agentColor(item.agent) }}>{agentName(item.agent)}</span>
            <b className="clip">{item.title}</b>
            <span className="tl-time">{span(item)}</span>
          </div>
          <div className="tl-sub">{stats.join(" · ")}</div>
          {open && (
            <div className="tl-detail">
              {item.files.length > 0 && <FileList files={item.files} />}
              <div className="tl-msgs">{item.messages.map((m, i) => <Message key={i} m={m} />)}</div>
            </div>
          )}
        </>
      );
    }
    case "commit":
      return (
        <>
          <div className="tl-head">
            <code className="tl-sha">{item.sha.slice(0, 7)}</code>
            <b className="clip">{item.subject}</b>
            <span className="tl-time">{span(item)}</span>
          </div>
          <div className="tl-sub">{[item.author, plural(item.fileCount, "file")].filter(Boolean).join(" · ")}</div>
          {open && <div className="tl-detail"><FileList files={item.files} total={item.fileCount} /></div>}
        </>
      );
    case "memory":
      return (
        <>
          <div className="tl-head">
            <span className="tl-kind" style={{ color: `var(--t-${item.type})` }}>{item.type.replace("_", " ")}</span>
            <b className="clip">{item.title}</b>
            <span className="tl-time">{span(item)}</span>
          </div>
          <div className="tl-sub">
            Remembered{item.author ? ` by ${agentName(item.author)}` : ""}
            {item.replaces.length > 0 && <> · replaces <i>{item.replaces.join(", ")}</i></>}
            {item.status !== "current" && <> · now {item.status}</>}
          </div>
        </>
      );
    case "edits":
      return (
        <div className="tl-head quiet">
          <span>Edited {plural(item.files.length, "file")}</span>
          <span className="clip faint">{item.files.slice(0, 4).join(", ")}{item.files.length > 4 ? ` +${item.files.length - 4}` : ""}</span>
          <span className="tl-time">{span(item)}</span>
        </div>
      );
    case "system":
      return (
        <div className="tl-head quiet">
          <span>Background</span>
          <span className="clip faint">{Object.entries(item.counts).map(([k, n]) => plural(n, SYSTEM_LABELS[k] ?? k)).join(", ")}</span>
          <span className="tl-time">{span(item)}</span>
        </div>
      );
  }
}
