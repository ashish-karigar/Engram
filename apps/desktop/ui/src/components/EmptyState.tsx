import { useState } from "react";
import type { projectsApi } from "../api/client.ts";
import { AgentCards, type useAgents } from "./AgentsPanel.tsx";
import { Mark } from "./icons.tsx";

interface Props {
  api: ReturnType<typeof projectsApi>;
  agents: ReturnType<typeof useAgents>;
  onAdd(): void;
  error?: string | null;
}

/** First run: connect an agent, then add a project. Connecting comes first because memory no agent
 *  reads is of little use; it can still be skipped. */
export function EmptyState({ api, agents, onAdd, error }: Props) {
  const [skipped, setSkipped] = useState(false);
  const step = agents.anyConnected || skipped ? 2 : 1;

  return (
    <section className="workspace">
      <div className="content"><div className="scroll onboarding" data-tauri-drag-region>
        <Mark size={28} />
        <h1>Shared memory for your coding agents</h1>
        <p className="muted">MemoryOS watches your project folders, remembers decisions and their reasons, and gives that context to every agent you connect.</p>

        <ol className="steps">
          <li className={step === 1 ? "current" : "done"}>
            <h3>1. Connect your agents</h3>
            <p className="muted">Once for all projects. Connect at least one.</p>
            {step === 1 && (
              <>
                <AgentCards api={api} agents={agents.agents} onChange={agents.reload} />
                {agents.loaded && <button className="link-btn" onClick={() => setSkipped(true)}>Skip for now</button>}
              </>
            )}
          </li>
          <li className={step === 2 ? "current" : ""}>
            <h3>2. Add a project folder</h3>
            <p className="muted">Agents working in this folder share its memory.</p>
            {step === 2 && <button className="btn primary" onClick={onAdd}>Add project folder</button>}
            {error && <p className="err">{error}</p>}
          </li>
        </ol>
      </div></div>
    </section>
  );
}
