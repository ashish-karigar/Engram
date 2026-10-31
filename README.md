# Engram

Shared, local-first memory for coding agents. Engram watches your projects, keeps an immutable
record of what happened, distills it into a temporal graph of decisions, reasons, dead ends and
code links, and serves small, token-budgeted answers to any agent over MCP or HTTP.

Switch agents, or survive a context compaction, without re-briefing.

Built for the Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026). All code in this
repository was written during the event.

## How it works

1. **Capture**: a watcher records file changes and git commits. Uncommitted work is saved to
   `refs/engram/snapshots` (code stays in git, never in Engram's database).
2. **Understand**: tree-sitter indexes Python; events are grouped into tasks; a model extracts decisions
   and reasons, and each is accepted only if its quote appears in the evidence.
3. **Remember**: memories are versioned and time-bounded; newer facts supersede older ones; stale
   episodes decay and are archived.
4. **Recall**: keyword search finds entry nodes, a graph walk finds what is connected, a token-budget
   packer picks what fits. No model call at read time.
5. **Ground in documents**: READMEs, design docs, PDFs and source files in the project are chunked,
   embedded and searched next to memories ("SuperRAG", locally). Agents can add documents too.
6. **Read agent conversations**: Claude Code, Codex and Muse Code session logs are read locally,
   scrubbed, and turned into searchable transcripts and extracted decisions. A Claude Code hook
   re-injects the project brief after the agent compacts its context.
7. **Observe**: every agent request is logged with what came back and why.

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Decisions: [docs/ACTION_PLAN.md](docs/ACTION_PLAN.md).

## Run it

Requires Rust (stable), Node 20+, and git.

```bash
cargo test --workspace            # 65 tests, including a two-project, two-agent end-to-end run
npm install
npm run app:dev                   # desktop app (Tauri) with hot reload
npm run app:build                 # release bundle
```

Or from a terminal, without the desktop window:

```bash
cargo run -p engram-daemon -- daemon                 # start the daemon
cargo run -p engram-daemon -- add ~/path/to/project  # watch a project
cargo run -p engram-daemon -- connect codex ~/path/to/project          # preview the config change
cargo run -p engram-daemon -- connect codex ~/path/to/project --apply  # apply (claude, codex, muse)
```

In the app, add project folders from the sidebar, then connect agents from the Agents tab.
Models, embeddings and indexing rules are set on the Settings page (saved to `settings.json` in the
data folder; API keys are stored only there and never sent back to the UI in full).

Extraction from commits is optional and configured with environment variables:
`META_API_KEY` (Muse Spark; `META_BASE_URL`, `META_MODEL` to override) or `ENGRAM_LLM=ollama`.
Without one, agent notes still work.

Embeddings default to a built-in offline model (no download). For semantic quality, run Ollama and set
`ENGRAM_EMBED=ollama` (uses `nomic-embed-text`), or `ENGRAM_EMBED=openai` with
`ENGRAM_EMBED_BASE_URL` and `ENGRAM_EMBED_API_KEY`.

## Layout

```
crates/core        schema, event log, memories, graph, decay, review
crates/embed       embedders, vector store, top-k, rank fusion
crates/rag         documents: extractors, chunking, ingest pipeline, hybrid chunk search
crates/capture     git helpers, secret scrubber, snapshot ref, file watcher
crates/understand  Python index, task grouping, extraction, grounding, notes
crates/serve       recall, budget packer, MCP server, HTTP API, tokens
crates/daemon      multi-project runtime, agent connectors, CLI (`engram`)
apps/desktop       Tauri shell + React UI (ui/)
```

## Status

Working and tested: capture, snapshots, Python indexing, extraction pipeline with grounding, temporal
memory, graph recall, MCP + HTTP (checked with the official MCP TypeScript client), request log,
connectors, multiple projects, desktop UI.

Not done: adapters that read Codex/Muse session logs (agents use `note` instead),
cloud sync and billing. The desktop app launches and runs its daemon, but the native folder picker and
the Codex/Muse/Claude connectors have not been exercised on a real machine. The Muse API base URL and
model id defaults are unverified and configurable.
