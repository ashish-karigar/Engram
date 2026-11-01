import { useEffect, useState } from "react";
import type { projectsApi } from "../api/client.ts";
import type { AppSettings, TestResult } from "../api/types.ts";

type Api = ReturnType<typeof projectsApi>;

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="s-field">
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}

function Choice<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange(v: T): void }) {
  return (
    <div className="seg">
      {options.map(([v, label]) => <button key={v} type="button" className={value === v ? "on" : ""} onClick={() => onChange(v)}>{label}</button>)}
    </div>
  );
}

function TestButton({ api, target }: { api: Api; target: "llm" | "embeddings" }) {
  const [state, setState] = useState<TestResult | { ok: false; detail: string } | "running" | null>(null);
  const run = () => {
    setState("running");
    api.testSettings(target).then(setState, (e: Error) => setState({ ok: false, detail: e.message }));
  };
  return (
    <div className="s-test">
      <button type="button" className="btn" onClick={run} disabled={state === "running"}>{state === "running" ? "Testing…" : "Test connection"}</button>
      {state && state !== "running" && (
        <span className={state.ok ? "ok" : "err"}>{state.ok ? "✓ " : "✕ "}{state.detail}{"latencyMs" in state ? ` · ${state.latencyMs} ms` : ""}</span>
      )}
    </div>
  );
}

/** App-wide settings. Saving restarts watched projects so new models and rules take effect. */
export function SettingsPage({ api }: { api: Api }) {
  const [saved, setSaved] = useState<AppSettings | null>(null);
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => { api.settings().then((s) => { setSaved(s); setDraft(s); }, (e: Error) => setStatus(e.message)); }, [api]);
  if (!draft || !saved) return <div className="empty"><p className="muted">{status ?? "Loading settings…"}</p></div>;

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const set = <S extends keyof AppSettings>(section: S, patch: Partial<AppSettings[S] & object>) =>
    setDraft({ ...draft, [section]: { ...(draft[section] as object), ...patch } });
  const save = () => {
    setSaving(true);
    setStatus(null);
    api.saveSettings(draft).then(
      (s) => { setSaved(s); setDraft(s); setStatus("Saved. Projects restarted with the new settings."); },
      (e: Error) => setStatus(e.message),
    ).finally(() => setSaving(false));
  };
  const { llm, embeddings: emb, indexing: idx, memory: mem } = draft;

  return (
    <section className="workspace">
      <header className="topbar" data-tauri-drag-region><h2>Settings</h2><span className="topbar-note">Applies to every project on this machine</span></header>
      <div className="content"><div className="scroll settings">

        <div className="s-card">
          <h3>Memory extraction model</h3>
          <p className="muted">Reads commits and activity and writes decisions, reasons and dead ends. Recall never calls a model.</p>
          <Choice value={llm.provider} options={[["none", "None"], ["meta", "Meta (Muse Spark)"], ["ollama", "Ollama (local)"]]} onChange={(provider) => set("llm", { provider })} />
          {llm.provider === "meta" && (
            <div className="s-grid">
              <Field label="API key" hint="Stored only on this machine, in a file only you can read.">
                <input className="field" type="password" placeholder={llm.metaApiKey ?? "Paste your Meta API key"} value={llm.metaApiKey?.startsWith("••••") ? "" : llm.metaApiKey ?? ""} onChange={(e) => set("llm", { metaApiKey: e.target.value || saved.llm.metaApiKey })} />
              </Field>
              <Field label="Model"><input className="field" value={llm.metaModel} onChange={(e) => set("llm", { metaModel: e.target.value })} /></Field>
              <Field label="Base URL"><input className="field" value={llm.metaBaseUrl} onChange={(e) => set("llm", { metaBaseUrl: e.target.value })} /></Field>
            </div>
          )}
          {llm.provider === "ollama" && (
            <div className="s-grid">
              <Field label="Host"><input className="field" value={llm.ollamaHost} onChange={(e) => set("llm", { ollamaHost: e.target.value })} /></Field>
              <Field label="Model"><input className="field" value={llm.ollamaModel} onChange={(e) => set("llm", { ollamaModel: e.target.value })} /></Field>
            </div>
          )}
          {llm.provider !== "none" && <TestButton api={api} target="llm" />}
          {dirty && llm.provider !== saved.llm.provider && <p className="faint">Save before testing a new provider.</p>}
        </div>

        <div className="s-card">
          <h3>Embeddings</h3>
          <p className="muted">Turn documents and memories into vectors for semantic search. Switching models re-embeds everything on save.</p>
          <Choice value={emb.provider} options={[["local", "Built-in model"], ["hash", "Word matching (no model)"], ["ollama", "Ollama"], ["openai", "OpenAI-compatible"]]} onChange={(provider) => set("embeddings", { provider })} />
          {emb.provider === "local" && (
            <>
              <Choice value={emb.localModel} options={[["bge-small-q", "bge-small, quantized (fast, 0.4 GB)"], ["bge-small", "bge-small (1.6 GB)"], ["nomic", "nomic-embed v1.5 (slow, 3.5 GB)"]]} onChange={(localModel) => set("embeddings", { localModel })} />
              <p className="faint">Runs inside Engram on the CPU; no server. Weights download once (130–550 MB) into the data folder. Search understands meaning, not just words.</p>
            </>
          )}
          {emb.provider === "hash" && <p className="faint">No download, no network. Matches words and word parts only.</p>}
          {emb.provider === "ollama" && (
            <div className="s-grid">
              <Field label="Host"><input className="field" value={emb.ollamaHost} onChange={(e) => set("embeddings", { ollamaHost: e.target.value })} /></Field>
              <Field label="Model" hint="e.g. nomic-embed-text (ollama pull nomic-embed-text)"><input className="field" value={emb.ollamaModel} onChange={(e) => set("embeddings", { ollamaModel: e.target.value })} /></Field>
            </div>
          )}
          {emb.provider === "openai" && (
            <div className="s-grid">
              <Field label="API key"><input className="field" type="password" placeholder={emb.openaiApiKey ?? "Paste a key"} value={emb.openaiApiKey?.startsWith("••••") ? "" : emb.openaiApiKey ?? ""} onChange={(e) => set("embeddings", { openaiApiKey: e.target.value || saved.embeddings.openaiApiKey })} /></Field>
              <Field label="Model"><input className="field" value={emb.openaiModel} onChange={(e) => set("embeddings", { openaiModel: e.target.value })} /></Field>
              <Field label="Base URL"><input className="field" value={emb.openaiBaseUrl} onChange={(e) => set("embeddings", { openaiBaseUrl: e.target.value })} /></Field>
            </div>
          )}
          <TestButton api={api} target="embeddings" />
        </div>

        <div className="s-card">
          <h3>Indexing</h3>
          <p className="muted">What becomes searchable. Conversations are read locally from each agent's own logs, scrubbed of secrets, and never leave this machine. Files git ignores are always skipped.</p>
          <div className="s-row"><label className="toggle"><span>Agent conversations (Claude Code, Codex, Muse Code session logs for this project)</span><input type="checkbox" checked={idx.sessions} onChange={(e) => set("indexing", { sessions: e.target.checked })} /><i /></label></div>
          <div className="s-row"><label className="toggle"><span>Documents (Markdown, text, HTML, PDF, configs)</span><input type="checkbox" checked={idx.indexDocs} onChange={(e) => set("indexing", { indexDocs: e.target.checked })} /><i /></label></div>
          <div className="s-row"><label className="toggle"><span>Source code</span><input type="checkbox" checked={idx.indexCode} onChange={(e) => set("indexing", { indexCode: e.target.checked })} /><i /></label></div>
          <div className="s-grid">
            <Field label="Largest file (KB)"><input className="field" type="number" min={16} value={idx.maxFileKb} onChange={(e) => set("indexing", { maxFileKb: Number(e.target.value) })} /></Field>
            <Field label="Also ignore" hint="One per line: data/ (folder), *.csv (extension), or any text in the path.">
              <textarea className="field" rows={4} value={idx.ignore.join("\n")} onChange={(e) => set("indexing", { ignore: e.target.value.split("\n") })} />
            </Field>
          </div>
        </div>

        <div className="s-card">
          <h3>Memory upkeep</h3>
          <div className="s-grid three">
            <Field label="Extract every (seconds)"><input className="field" type="number" min={10} value={mem.extractEverySecs} onChange={(e) => set("memory", { extractEverySecs: Number(e.target.value) })} /></Field>
            <Field label="Forgetting sweep (hours)"><input className="field" type="number" min={1} value={mem.decayEveryHours} onChange={(e) => set("memory", { decayEveryHours: Number(e.target.value) })} /></Field>
            <Field label="Save uncommitted work (seconds)"><input className="field" type="number" min={5} value={mem.snapshotEverySecs} onChange={(e) => set("memory", { snapshotEverySecs: Number(e.target.value) })} /></Field>
          </div>
        </div>

        <div className="s-card">
          <h3>Data</h3>
          <p className="muted">Everything Engram stores lives here, outside your projects. Code itself stays in git.</p>
          <code>{saved.dataDir}</code>
          <div className="actions" style={{ marginTop: 10 }}>
            <button type="button" className="btn" onClick={() => { localStorage.clear(); setStatus("Graph view preferences reset."); }}>Reset graph view preferences</button>
          </div>
        </div>
      </div></div>
      <div className="s-footer">
        <span className={status?.startsWith("Saved") || status?.startsWith("Graph") ? "ok" : "err"}>{status}</span>
        <button className="btn" disabled={!dirty || saving} onClick={() => setDraft(saved)}>Discard</button>
        <button className="btn primary" disabled={!dirty || saving} onClick={save}>{saving ? "Applying…" : "Save and apply"}</button>
      </div>
    </section>
  );
}
