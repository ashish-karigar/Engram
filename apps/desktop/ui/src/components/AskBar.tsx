import { useEffect, useRef, useState } from "react";
import type { ProjectApi } from "../api/client.ts";

interface Props {
  api: ProjectApi;
  onPick(id: string): void;
}

const MEMORY_ID = /mem_[a-z0-9]+/;

/** Ask the project's memory a question, exactly as an agent would. */
export function AskBar({ api, onPick }: Props) {
  const [query, setQuery] = useState("");
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
    const res = await api.search(query).catch((err: Error) => ({ text: err.message, data: null }));
    setLines(res.text.split("\n").filter(Boolean));
  };

  return (
    <div className="ask" ref={box}>
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6"><circle cx="7" cy="7" r="5" /><path d="m11 11 3.5 3.5" /></svg>
      <form onSubmit={submit}>
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Ask memory: why is auth session based?" />
      </form>
      {lines && (
        <div className="results">
          {lines.map((line, i) => {
            const id = MEMORY_ID.exec(line)?.[0];
            const [main, reason] = line.split(/ \(mem_/);
            return (
              <button key={i} className="result" disabled={!id} onClick={() => { if (id) onPick(id); setLines(null); }}>
                <div>{main}</div>
                {reason && <div className="reason">mem_{reason.replace(/\)$/, "")}</div>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
