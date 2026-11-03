# Using Engram

Engram gives your coding agents one shared, persistent memory per project. You pick folders and
connect agents once; everything after that is automatic. This guide covers setup, daily use, what
each screen shows, settings, and how to undo anything Engram changes.

## 1. Install and start

Requirements: macOS (Linux works for the daemon), Rust stable, Node 20+, git.

```bash
git clone https://github.com/ashish-karigar/MemoryOS.git
cd MemoryOS
npm install
npm run app:dev          # desktop app with the daemon inside it
```

Without the window (daemon and CLI only):

```bash
cargo run -p engram-daemon -- daemon
```

The daemon listens on `http://127.0.0.1:41741` (set `ENGRAM_PORT` to change it). If the port is taken
it uses a free one and logs that agents must be reconnected. Data lives in
`~/Library/Application Support/Engram` (macOS) or `~/.local/share/engram`, never inside your projects.

## 2. Add projects

In the app: **+** next to Projects, then choose a folder. From a terminal:

```bash
cargo run -p engram-daemon -- add ~/code/my-project
cargo run -p engram-daemon -- list
```

Engram runs `git init` if the folder is not a repository. On first add it indexes code (Python, Rust,
TypeScript, JavaScript), documents (Markdown, HTML, PDF, text, source files) and any agent
conversations that ran in that folder. Ignore rules are in Settings → Indexing.

## 3. Connect agents (once)

Each agent is connected once and then works in every watched project; Engram picks the project from
the folder the agent is working in.

```bash
cargo run -p engram-daemon -- connect claude           # shows the exact change
cargo run -p engram-daemon -- connect claude --apply   # makes it
cargo run -p engram-daemon -- connect codex --apply
cargo run -p engram-daemon -- connect muse --apply
```

| Agent | What changes | Backup |
|---|---|---|
| Claude Code | `engram` server in `~/.claude.json` (or `claude mcp add` when the CLI exists); a SessionStart hook in `~/.claude/settings.json` that injects the project brief on start, resume and after compaction | `<file>.engram.bak` |
| Codex | a marked `[mcp_servers.engram]` block in `~/.codex/config.toml` | `config.toml.engram.bak` |
| Muse Code | `mcpServers.engram` in `~/.config/muse/settings.json` | `settings.json.engram.bak` |

Start a **new** agent session afterwards; running sessions do not pick up new servers.
To disconnect, restore the `.engram.bak` file (or delete the `engram` entry).

## 4. Daily use

You work as usual. Engram, in the background:

- records file changes and commits, and saves uncommitted work to the hidden ref `refs/engram/snapshots`;
- reads each agent's session log (only new lines), scrubs secrets, and keeps a searchable transcript;
- extracts decisions, reasons, rejected options and dead ends when an extraction model is configured
  (each must quote the evidence it came from, or it is rejected);
- serves agents through six MCP tools.

| Tool | Agents use it to |
|---|---|
| `brief` | get the project map, standing decisions and recent activity at the start of a task or after compaction |
| `search` | ask "why is auth session based?" or "what depends on the lidar filter?"; memories and documents, best first |
| `expand` | open a result: full text, history, evidence, connected nodes |
| `note` | record a decision, rejected option or dead end with its reason, the moment it happens |
| `profile` | read standing facts and current focus |
| `add_document` | save a spec or design note |

Agents are told when to use these through the MCP instructions and the startup brief. You can also
ask your agent directly: "check Engram for why we chose X", "note that we rejected Y because Z".

## 5. The app, screen by screen

| Tab | Shows |
|---|---|
| Memory graph | memories, documents, files and modules as a graph, in 2D or rotatable 3D; click a node to highlight its neighbours and open its detail; filters, groups and forces in the ⚙ panel |
| Documents | everything indexed, filterable by docs, code, conversations and documents you added |
| Review | inferred memories to approve or decline, and weak memories (changelog lines, duplicates, raw identifiers, low confidence) to archive; archiving is reversible |
| Timeline | project history as a git-style graph: a lane per agent session, commits, edits and new memories on the main line |
| Requests | every agent request: which agent, which tool, what came back and why, tokens and latency |
| Agents | connection status and the change each connect makes |

The ask bar at the top searches the current project the same way agents do.

## 6. Settings

Settings are saved to `settings.json` in the data folder (permissions 0600). API keys are stored only
there and are always shown masked.

- **Extraction model**: none, Meta (Muse Spark) or Ollama. (Planned: any OpenAI-compatible provider,
  such as Groq serving Meta's Llama models.) Test connection sends a one-word request. With no model,
  memories come only from agent notes.
- **Embeddings**: the model used to search by meaning. Changing it re-embeds documents in the background.
- **Indexing**: conversations on/off, documents on/off, code on/off, maximum file size, ignore patterns
  (`dir/`, `*.ext`, or a substring).
- **Upkeep**: how often extraction, decay and snapshots run.

## 7. Privacy

Everything stays on your machine unless you configure a hosted model, in which case only the text of
the events being summarized is sent to it. Secrets are scrubbed from conversation turns before they are
stored. Engram never stores file contents for code; it reads them from git.

## 8. Troubleshooting

| Symptom | Check |
|---|---|
| Agent has no Engram tools | It was started before connecting; start a new session. Run `connect <agent>` again to see the entry. |
| "Engram is not watching …" from a tool | The agent's folder is not inside a watched project; add it. |
| Agents stopped reaching Engram after a restart | The port was busy and Engram picked another; reconnect the agents. |
| Brief missing after compaction (Claude) | The hook is in `~/.claude/settings.json` under `SessionStart`; it prints nothing when the daemon is not running. |
| High CPU or memory | A local model (Ollama) is configured for extraction; it runs in short bursts and unloads after each. Switch to a hosted model or none. |
