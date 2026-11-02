# MemoryOS

Shared, local-first memory for coding agents. MemoryOS watches your projects, keeps an immutable
record of what happened, distills it into a temporal graph of decisions, reasons, dead ends and
code links, and serves small, token-budgeted answers to any agent over MCP or HTTP.

Switch agents, or survive a context compaction, without re-briefing.

**[Download MemoryOS for Mac](https://github.com/ashish-karigar/Engram/releases/latest/download/MemoryOS_0.1.0_aarch64.dmg)**
(Apple silicon, M1 and later) · [Release notes](https://github.com/ashish-karigar/Engram/releases/latest)

Built for the Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026). All code in this
repository was written during the event.

## Install

1. Open the `.dmg` and drag MemoryOS into Applications. Open it from there: agent connectors
   remember where the app lives, so connecting while it runs from the disk image breaks them later.
2. The app is not notarized yet, so macOS blocks the first launch. Go to System Settings →
   Privacy & Security and click **Open Anyway**. This is needed once.
3. Add a project folder from the sidebar, then connect your agents (Claude Code, Codex, Muse Code)
   from the Agents tab. New sessions start with the project brief and can search memory over MCP.

Everything stays on your Mac. A hosted model is used only if you add one in Settings.

## Results

Live agents, same tasks with and without MemoryOS (Muse Code, `muse-spark-1.3`, 14 tasks × 3 runs
per condition, 84 trials):

| | With MemoryOS | Without |
|---|---|---|
| Trials passed | **39 / 42** | 26 / 42 |
| Median tokens per trial | **145k** | 216k |
| Median uncached tokens per trial | **31k** | 58k |
| Median tool calls per trial | **6** | 10 |

- Tasks whose answer existed only in past sessions: **12 / 12** with memory, 0 / 12 without.
- Uncached tokens were lower with memory on all 14 tasks.
- LongMemEval-S test split, never tuned on: evidence in the top 5 for 99% of questions, and
  **86.7%** answer accuracy with Muse Spark.
- Model-judged memory links: 93–97% correct relations over 3 runs, with no unrelated pairs linked.

Method, per-task numbers and the failures: [docs/EVALUATION.md](docs/EVALUATION.md).

## How it works

1. **Capture**: a watcher records file changes and git commits. Uncommitted work is saved to
   `refs/engram/snapshots` (code stays in git, never in MemoryOS's database).
2. **Understand**: tree-sitter indexes Python, Rust, TypeScript and JavaScript (symbols and imports, Cargo workspaces included); events are grouped into tasks; a model extracts decisions
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

How to use it: [docs/USAGE.md](docs/USAGE.md). How it is evaluated: [docs/EVALUATION.md](docs/EVALUATION.md).
Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Decisions: [docs/ACTION_PLAN.md](docs/ACTION_PLAN.md).

## Build from source

Requires Rust (stable), Node 20+, and git.

```bash
cargo test --workspace            # includes a two-project, two-agent end-to-end run
npm install
npm run app:dev                   # desktop app (Tauri) with hot reload
npm run app:build                 # release bundle: .app and .dmg in target/release/bundle
```

Or from a terminal, without the desktop window. The project was first called Engram, and the
crates, environment variables and data folder still use that name:

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
crates/code        languages: symbols, imports, import resolution
crates/embed       embedders, vector store, top-k, rank fusion
crates/rag         documents: extractors, chunking, ingest pipeline, hybrid chunk search
crates/capture     git helpers, secret scrubber, snapshot ref, file watcher
crates/understand  Python index, task grouping, extraction, grounding, notes
crates/serve       recall, budget packer, MCP server, HTTP API, tokens
crates/daemon      multi-project runtime, agent connectors, CLI (`engram`)
apps/desktop       Tauri shell + React UI (ui/)
```

## Status

Working and tested: capture, snapshots, code indexing, extraction with grounding, temporal memory,
graph recall, documents, Claude Code / Codex / Muse Code session logs, MCP + HTTP, request log,
agent connectors, multiple projects, and the packaged desktop app.

Not done: Intel Mac and Windows builds, notarization, cloud sync. The Muse API base URL and model id
defaults are configurable.

## License

Copyright (c) 2026 Ashish Karigar. All rights reserved. This is not open source: the code is public
for viewing and for evaluating the project, including hackathon judging. Any other use needs written
permission. See [LICENSE](LICENSE).
