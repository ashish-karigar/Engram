import { useEffect, useRef, useState } from "react";
import type { ProjectApi } from "../api/client.ts";
import type { SearchMode } from "../api/types.ts";

interface Props {
  api: ProjectApi;
  onPick(id: string): void;
}

const NODE_ID = /\b(mem|chk)_[a-z0-9]+/;
const MODES: [SearchMode, string][] = [["hybrid", "All"], ["memories", "Memories"], ["documents", "Docs"]];

/** Ask the project's memory a question, exactly as an agent would. */
export function AskBar({ api, onPick }: Props) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("hybrid");
  const [lines, setLines] = useState<string[] | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setLines(null);
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, []);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim()) return;
    const res = await api.search(query, mode).catch((err: Error) => ({ text: err.message, data: null }));
    setLines(res.text.split("\n").filter(Boolean));
  };

  return (
    <div className="ask" ref={box}>
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6"><circle cx="7" cy="7" r="5" /><path d="m11 11 3.5 3.5" /></svg>
      <form onSubmit={submit}>
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Ask memory: why is auth session based?" />
      </form>
      <div className="seg">
        {MODES.map(([m, label]) => <button key={m} className={mode === m ? "on" : ""} onClick={() => setMode(m)}>{label}</button>)}
      </div>
      {lines && (
        <div className="results">
          {lines.map((line, i) => {
            if (!line.startsWith("•")) return <div key={i} className="result-head">{line.replace(/:$/, "")}</div>;
            const id = NODE_ID.exec(line)?.[0];
            const cut = id ? line.lastIndexOf(` (${id}`) : -1;
            const main = cut > 0 ? line.slice(2, cut) : line.slice(2);
            const reason = cut > 0 ? line.slice(cut + 2, -1) : "";
            return (
              <button key={i} className="result" disabled={!id} onClick={() => { if (id) onPick(id); setLines(null); }}>
                <div>{main}</div>
                {reason && <div className="reason">{reason}</div>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
