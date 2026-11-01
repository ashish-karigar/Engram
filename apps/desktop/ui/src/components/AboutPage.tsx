import { useEffect, useState } from "react";
import type { Connection } from "../api/types.ts";
import { Mark } from "./icons.tsx";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

async function appVersion(): Promise<string | null> {
  if (!(window as TauriWindow).__TAURI_INTERNALS__) return null;
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

export function AboutPage({ connection, projects }: { connection: Connection; projects: number }) {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => { appVersion().then(setVersion, () => setVersion(null)); }, []);

  return (
    <section className="workspace">
      <header className="topbar" data-tauri-drag-region><h2>About</h2></header>
      <div className="content"><div className="scroll page">
        <div className="about-head">
          <Mark size={22} />
          <div>
            <h1>MemoryOS</h1>
            <p className="faint">Version {version ?? "0.1.0"}</p>
          </div>
        </div>
        <p className="lead">Local-first shared memory for coding agents.</p>

        <h3>What it does</h3>
        <p>MemoryOS watches your project folders and keeps the decisions, facts and dead ends your agents work through, with their reasons and the code they touch. It also maps the code: its parts, the endpoints between them, and the services they rely on.</p>
        <h3>Shared by every agent</h3>
        <p>Claude Code, Codex and other agents connect over MCP and read the same memory, so the next session starts where the last one stopped.</p>
        <h3>Stays on this Mac</h3>
        <p>Each project's memory is a SQLite file under ~/Library/Application Support/Engram. Nothing is uploaded; models run wherever you point them in Settings.</p>

        <dl className="facts">
          <div><dt>Memory service</dt><dd className="mono">{connection.url}</dd></div>
          <div><dt>Projects</dt><dd>{projects}</dd></div>
          <div><dt>Protocol</dt><dd>MCP</dd></div>
          <div><dt>License</dt><dd>MIT</dd></div>
        </dl>
      </div></div>
    </section>
  );
}
