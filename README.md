# Engram

Shared, local-first memory for coding agents. Engram watches a project, keeps an immutable
record of what happened, distills it into a temporal graph of decisions, reasons, dead ends
and code links, and serves small, token-budgeted answers to any agent over MCP or HTTP.

Switch from one agent to another, or survive a context compaction, without re-briefing.

Built for the Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026). All code in this
repository was written during the event.

## How it works

1. **Capture** – a watcher records file changes and git commits; uncommitted work is saved
   to `refs/engram/snapshots` (code stays in git, never in Engram's database).
2. **Understand** – tree-sitter indexes Python; events are grouped into tasks; a model extracts
   decisions and reasons, and each one is accepted only if its quote appears in the evidence.
3. **Remember** – memories are versioned and time-bounded; newer facts supersede older ones;
   stale episodes decay and are archived.
4. **Recall** – keyword search finds entry nodes, a graph walk finds what is connected, and a
   token-budget packer picks what fits. No model call at read time.
5. **Observe** – every agent request is logged with what came back and why.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and [docs/ACTION_PLAN.md](docs/ACTION_PLAN.md) for decisions.

## Run it

Requires Node 24+ and git.

```bash
npm install
npm test                                  # 38 tests, including a two-agent end-to-end run
node packages/daemon/src/cli.ts start ~/path/to/project
node packages/daemon/src/cli.ts connect codex ~/path/to/project          # preview
node packages/daemon/src/cli.ts connect codex ~/path/to/project --apply  # apply (also: claude, muse)
```

Desktop app (Electron + React + Cytoscape):

```bash
npm run build -w @engram/app && npm run electron -w @engram/app
```

Choose a folder in the app; it starts the daemon for that project.

Models for extraction are optional and configured by environment variables:
`META_API_KEY` (Muse Spark; `META_BASE_URL`, `META_MODEL` to override) or `ENGRAM_LLM=ollama`.
Without one, agent notes still work; automatic extraction from commits is off.

## Layout

```
packages/core        schema, event log, memories, graph, decay
packages/capture     git helpers, secret scrubber, snapshot ref, file watcher
packages/understand  Python index, task grouping, extraction, grounding, notes
packages/serve       recall, budget packer, MCP server, HTTP API, tokens
packages/daemon      project runtime, agent connectors, CLI
packages/app         Electron + React UI
```

## Status

Working and tested: capture, snapshots, Python indexing, extraction pipeline with grounding,
temporal memory, graph recall, MCP + HTTP, request log, connectors, UI.

Not done: vector search, adapters that read Codex/Muse session logs (agents use `note` instead),
cloud sync and billing, folder picker flow in Electron is untested beyond launch.
The Muse API base URL and model id defaults are unverified and configurable.
