import { useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import type { MemoryView, PendingLink } from "../api/types.ts";
import { usePolling } from "../hooks/usePolling.ts";

const VERB: Record<PendingLink["relation"], string> = {
  same: "says the same as",
  replaces: "replaces",
  extends: "adds detail to",
  depends_on: "depends on",
  contradicts: "conflicts with",
};

function Side({ m, label }: { m: MemoryView; label: string }) {
  return (
    <div className="link-side">
      <span className="faint">{label} · {m.validFrom.slice(0, 10)}</span>
      <b>{m.title}</b>
      <p>{m.claim}</p>
    </div>
  );
}

/**
 * Links the model was unsure of, and every contradiction it found. Nothing here changes the
 * memory until a person decides.
 */
export function LinkReview({ api }: { api: ProjectApi }) {
  const { data = [], reload } = usePolling<PendingLink[]>(() => api.links(), 5000, [api]);
  const { data: retired = [], reload: reloadRetired } = usePolling<PendingLink[]>(() => api.retiredLinks(), 5000, [api]);
  const [error, setError] = useState<string | null>(null);
  const decide = (id: string, action: "approve" | "decline" | "keep" | "undo", keep?: string) =>
    api.decideLink(id, action, keep).then(() => { setError(null); reload(); reloadRetired(); }, (e: Error) => setError(e.message));
  if (data.length === 0 && retired.length === 0) return null;

  return (
    <>
    {data.length > 0 && <section style={{ marginBottom: 20 }}>
      <h3>Links to confirm</h3>
      <p className="muted" style={{ marginBottom: 12 }}>Connections the model found between memories. Conflicts need you to say which one holds; the other is kept as history.</p>
      {error && <p className="err">{error}</p>}
      <ul className="rows">
        {data.map((l) => {
          // A replacement reads newer → older, the way Confirm applies it.
          const flip = l.relation === "replaces" && l.dst.validFrom > l.src.validFrom;
          const [first, second] = flip ? [l.dst, l.src] : [l.src, l.dst];
          const [firstCode, secondCode] = flip ? [l.dstFromCode, l.srcFromCode] : [l.srcFromCode, l.dstFromCode];
          return (
          <li key={l.id} className="row">
            <div className="head">
              <b>{l.relation === "contradicts" ? "Conflict" : `Possible link: ${VERB[l.relation]}`}</b>
              <span className="faint">{Math.round(l.confidence * 100)}% · {l.judgedBy}</span>
            </div>
            <Side m={first} label={firstCode ? "From the code" : "Memory"} />
            <p className="faint">{VERB[l.relation]}</p>
            <Side m={second} label={secondCode ? "From the code" : "Memory"} />
            <p className="muted">{l.reason}</p>
            <div className="actions">
              {/* Facts from the code change only when the code does, so they can't be the side dropped. */}
              {l.relation === "contradicts" ? (
                l.srcFromCode || l.dstFromCode ? (
                  <button className="btn" onClick={() => decide(l.id, "keep", l.srcFromCode ? l.src.id : l.dst.id)}>The code is right</button>
                ) : (
                  <>
                    <button className="btn" onClick={() => decide(l.id, "keep", first.id)}>Keep the first</button>
                    <button className="btn" onClick={() => decide(l.id, "keep", second.id)}>Keep the second</button>
                  </>
                )
              ) : (
                <button className="btn primary" onClick={() => decide(l.id, "approve")}>Confirm</button>
              )}
              <button className="btn" onClick={() => decide(l.id, "decline")}>{l.relation === "contradicts" ? "Not a conflict" : "Not related"}</button>
            </div>
          </li>
          );
        })}
      </ul>
    </section>}
    {retired.length > 0 && <section style={{ marginBottom: 20 }}>
      <h3>Retired by the model</h3>
      <p className="muted" style={{ marginBottom: 12 }}>Memories the model judged replaced or repeated. They are kept as history; undo brings one back.</p>
      {data.length === 0 && error && <p className="err">{error}</p>}
      <ul className="rows">
        {retired.map((l) => {
          const [kept, gone] = l.src.status === "current" ? [l.src, l.dst] : [l.dst, l.src];
          return (
            <li key={l.id} className="row">
              <div className="head">
                <b>{gone.title}</b>
                <span className="faint">{l.relation === "same" ? "repeat of" : "replaced by"} “{kept.title}” · {Math.round(l.confidence * 100)}%</span>
              </div>
              <p className="faint">{gone.claim}</p>
              <p className="muted">{l.reason}</p>
              <div className="actions"><button className="btn" onClick={() => decide(l.id, "undo")}>Undo</button></div>
            </li>
          );
        })}
      </ul>
    </section>}
    </>
  );
}
