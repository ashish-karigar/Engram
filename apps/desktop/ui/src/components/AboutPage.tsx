import { useEffect, useState } from "react";
import type { Connection } from "../api/types.ts";
import { GraphIcon, LockIcon, Logo, PlugIcon } from "./icons.tsx";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

async function appVersion(): Promise<string | null> {
  if (!(window as TauriWindow).__TAURI_INTERNALS__) return null;
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

const POINTS = [
  {
    icon: GraphIcon,
    title: "Remembers the why",
    text: "Engram watches your project folders and keeps the decisions, facts and dead ends your agents work through, with their reasons and the code they touch. It also maps the code itself: the parts, the endpoints between them, and the services they rely on.",
  },
  {
    icon: PlugIcon,
    title: "Shared by every agent",
    text: "Claude Code, Codex and other agents connect over MCP and read the same memory. What one agent learns, the next one starts with, in a few hundred tokens instead of a fresh search of the repository.",
  },
  {
    icon: LockIcon,
    title: "Stays on this Mac",
    text: "Everything lives in a SQLite file per project under ~/Library/Application Support/Engram. Nothing is uploaded; models run wherever you point them in Settings.",
  },
];

export function AboutPage({ connection, projects }: { connection: Connection; projects: number }) {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => { appVersion().then(setVersion, () => setVersion(null)); }, []);

  return (
    <section className="workspace">
      <div className="topbar" data-tauri-drag-region><div className="title"><h2>About</h2></div></div>
      <div className="content"><div className="scroll about">
        <div className="about-hero">
          <Logo size={64} />
          <div>
            <h1>Engram</h1>
            <p className="muted">Local-first shared memory for coding agents</p>
            <p className="faint">Version {version ?? "0.1.0"}</p>
          </div>
        </div>

        <div className="about-points">
          {POINTS.map(({ icon: I, title, text }) => (
            <div key={title} className="about-point">
              <span className="about-icon"><I size={18} /></span>
              <h3>{title}</h3>
              <p>{text}</p>
            </div>
          ))}
        </div>

        <dl className="about-facts">
          <div><dt>Memory service</dt><dd className="mono">{connection.url}</dd></div>
          <div><dt>Projects watched</dt><dd>{projects}</dd></div>
          <div><dt>Agent protocol</dt><dd>MCP (Model Context Protocol)</dd></div>
          <div><dt>License</dt><dd>MIT</dd></div>
        </dl>
      </div></div>
    </section>
  );
}
