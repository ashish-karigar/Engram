# Engram architecture

Engram is a local-first memory layer shared by every coding agent working in a folder.
It observes the project, keeps an immutable record, distills it into a temporal graph,
and serves small, token-budgeted answers over MCP and HTTP. One app watches many projects.

## Data flow

```
 agents (Claude Code, Codex, Muse Code)            project folders + git
        │  MCP / HTTP: note · brief · search · expand      │ fs events, commits
        ▼                                                  ▼
 ┌───────────────────────────── daemon (Rust) ───────────────────────────┐
 │ capture ──► events (append-only, hash-chained) ──► understand         │
 │   watcher, git, scrubber,        │                  tasks, extract,   │
 │   snapshots (refs/engram/*)      │                  verify, link,     │
 │                                  ▼                  decay             │
 │      SQLite per project: events · memories · entities · edges · FTS5  │
 │                                  ▲                                    │
 │             serve: recall · budget · MCP · HTTP (/p/<project>/…)      │
 └──────────────────────────────────┬────────────────────────────────────┘
                                    ▼
                      desktop app (Tauri + React, canvas graph)
```

## Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `engram-core` | Schema, migrations, event log, memories, graph, decay, request log. Only SQLite I/O. | – |
| `engram-embed` | `Embedder` trait (built-in hashing, Ollama, OpenAI-compatible), vectors in SQLite, exact top-k, RRF. | core |
| `engram-rag` | Documents: extractors (markdown, HTML, PDF, text, code), heading/AST-aware chunking, ingest pipeline, hybrid chunk search. | core, embed |
| `engram-sessions` | Parsers for Claude Code, Codex and Muse session logs; incremental sync; transcripts. | core, capture, embed, rag |
| `engram-code` | Languages (Python, Rust, TypeScript, JavaScript/JSX): symbols with spans, imports, resolution (incl. Cargo workspaces). | – |
| `engram-capture` | Watch files and git, scrub secrets, save uncommitted work to a git ref. | core |
| `engram-understand` | Code graph from `engram-code`, group events into tasks, extract and verify memories, notes. | core, capture, code |
| `engram-serve` | Hybrid recall over memories and documents, profile, token-budget packer, MCP server, HTTP API, per-agent tokens. | core, understand, embed, rag |
| `engram-daemon` | Runs many projects, owns the server, agent connectors, `engram` CLI. | all above |
| `apps/desktop` | Tauri shell (runs the daemon in-process) and the React UI in `ui/`. | daemon (HTTP only from UI) |

Rule: crates depend downward only. The UI never links server code; it speaks HTTP.

## Many projects, one daemon

The daemon keeps a registry of project folders (restored on start) and one SQLite file per project.
The server exposes `/p/<project-id>/mcp` and `/p/<project-id>/api/...` for agents and the UI, and
`/api/projects` for the UI to add or remove folders. Tokens are per agent and per project: a token
from one project is rejected by another. The UI uses an admin token held by the desktop process.

## Storage

Data lives outside projects (`~/Library/Application Support/Engram` on macOS, `ENGRAM_HOME` to override),
so there is nothing to gitignore or leak. Engram never stores file contents: code is read from git.
Uncommitted work is saved as commits on `refs/engram/snapshots`, built with a temporary index so the
user's index, branches and working tree are never touched.

- `events`: immutable, sha256 hash chain, idempotent on `op_id`. Memory is derived and rebuildable.
- `memories`: typed (decision, fact, preference, episode, rejected_option, dead_end, task). Bitemporal:
  `observed_at`, `valid_from`/`valid_to`, `event_date`, plus `commit_sha`.
- `entities`: files, modules, symbols. `renamed_from` keeps identity across renames.
- `edges`: typed, weighted, time-bounded links between any two node ids.
- `memory_evidence`: memory → supporting event seqs. Unsupported memories are rejected.
- `access_log`: every agent request, what came back and why, tokens, latency.
- `documents` / `chunks` / `chunks_fts`: the documents layer (raw sources and their retrieval units).
- `embeddings`: vectors for chunks and memories, keyed by model id so providers never mix.

## Two layers of context

Engram keeps **documents** (raw sources, chunked for grounding) separate from
**memories** (distilled facts in the graph). Documents come from the project folder automatically
(docs, configs and source files that git would track), from agents via `add_document`, or from the UI.
Pipeline: queued → extracting → chunking → embedding → indexing → done (or failed, with the error).
Unchanged files are skipped by content hash; changed ones are re-chunked and bump the version.
Each chunk carries a contextual header (`title > section`) that is indexed and embedded with it.
Documents link into the graph: `describes` its own code file, `mentions` other files it names.

Memory relationships: `supersedes` (updates), `extends` (adds detail),
`derives` (inferred, flagged `is_inference` until approved or declined). Repeating a known fact
reinforces it; facts can carry `expires_at`; a project profile splits standing facts from current focus.

## Algorithms and why

| Need | Choice | Why |
|---|---|---|
| Tamper-evident history | Hash chain over events | O(n) verify, O(1) append |
| Keyword recall | FTS5 + BM25, weighted columns, stopwords dropped | Built into SQLite, no service |
| Semantic recall | Embeddings (in-process ONNX models) + exact top-k below 5,000 vectors, HNSW (usearch, 32 links, f16) above | Exact is correct and fast when small; HNSW keeps 1M vectors under a millisecond at 0.995 recall@10 |
| IDF and keyword pruning | Word document counts from FTS5 vocabulary tables; at 10k+ chunks, words in over 5% of chunks are left out of the keyword query | No per-search row scans; ranking every chunk with a common word was the slow part at 1M |
| Combine rankers | Reciprocal Rank Fusion (k = 60) | Uses ranks only, so BM25 and cosine scales never need calibrating |
| Chunking | Markdown by heading path; Python, Rust, TS/JS by top-level symbol (tree-sitter); other code by overlapping line windows | Chunks follow meaning, not character counts |
| Offline embeddings | Signed feature hashing of words, bigrams and 4-grams (FNV-1a) | No model download; deterministic; lexical, not deep semantics |
| Neighbour walk | Recursive CTE, per-hop score decay, edge-type filter | One query, depth-bounded, no graph DB |
| Mix memories and documents | Each ranker's score blended with IDF-weighted query coverage (light stemming) | Each ranker normalizes to its own best hit; coverage puts both on one scale and favours rare words |
| How much to return (search) | Best first, stop below 70% of the top score (max 6), then the token budget | Short answers when one result clearly wins; no padding to fill the budget |
| Snippets | The chunk window that covers the most of the query | The answer is often deep in a chunk; `expand` has the rest |
| Fit a token budget (brief) | Greedy by value density (score / tokens) | Knapsack approximation; O(n log n) |
| Rank | relevance × freshness (decay + usage) | Cheap, explainable, logged per hit |
| Forgetting | Exponential decay, half-life by memory type | Facts persist until superseded; episodes fade |
| Group events into tasks | Sessionization by idle gap | Single linear pass |
| Grounding | Cited quote must appear in cited events | Rejects hallucinated memories cheaply |
| Duplicate/update | Jaccard on titles and claims → duplicate or supersede | Keeps one current version, history intact |

Recall never calls an LLM. Models are only used on the write path, and the pipeline holds its own
database connection so a slow model call never blocks an agent request.

## Models
`LlmClient` is one method. Providers: Meta API (Muse Spark), Ollama, and a scripted client for tests.
Selected by environment variables (`META_API_KEY`, `META_BASE_URL`, `META_MODEL`, `ENGRAM_LLM`).
The Meta base URL and model id defaults are unverified. Embeddings: `ENGRAM_EMBED=hash|ollama|openai` (default `hash`, offline). `ollama` uses `nomic-embed-text` unless `ENGRAM_EMBED_MODEL` is set.

## Agent integration
Agents connect once to the shared MCP endpoint `http://127.0.0.1:41741/mcp` (streamable HTTP, JSON
responses, one bearer token per agent in `agents.json`, 0600). The port is fixed (`ENGRAM_PORT` overrides
it) so agent configs survive restarts. Each tool call is placed in a project: the deepest watched folder
containing the `cwd` argument, else the project where that agent was last active, else the only one; the
answer names the project. `/p/<project>/mcp` still addresses one project directly.
The MCP server is hand-written (initialize, ping, tools/list, tools/call) and was checked against the
official TypeScript MCP client. Tools are deliberately few: `brief`, `search` (modes hybrid/memories/documents), `expand`, `note`, `profile`, `add_document`.
`engram connect <claude|codex|muse>` shows, then applies (`--apply`), one config change per agent, backing
up each file: Codex (`~/.codex/config.toml`, marked block), Muse Code (`mcpServers` in
`~/.config/muse/settings.json`, entry format unverified), Claude Code (`claude mcp add`, or `mcpServers` in
`~/.claude.json` when the CLI is not installed, plus the SessionStart hook).

## Why Rust + Tauri
Engram is an always-on background process, so idle memory and startup matter. A Rust daemon plus
Tauri keeps it small, ships as one binary with no Node requirement, and the daemon runs inside the
desktop app process. The UI is web technology (React, with the graph drawn on a canvas by a d3-force simulation in 2D or 3D);
it only speaks HTTP, so the shell is replaceable.

## Agent conversations

`engram-sessions` reads each agent's own session log for sessions run inside a watched project:
Claude Code (`~/.claude/projects/<slug>/*.jsonl`), Codex (`~/.codex/sessions/**/rollout-*.jsonl`) and
Muse Code (`~/.local/share/muse/sessions/**/session.jsonl`, an event log with nested frames). Only
appended whole lines are read (byte offsets per file); each user or assistant turn becomes a scrubbed
`agent.turn` event attributed to that agent's actor, compactions become `agent.compacted`, and every
changed session is re-rendered as a Markdown transcript document, so conversations are searchable and
feed memory extraction. Thinking blocks and tool outputs are not stored.

`engram hook claude` (installed by the Claude connector as a SessionStart hook for startup, resume and
compact) prints the project brief as `additionalContext`, so the agent re-orients right after
compacting. It fails open: no daemon or no project means no output and exit 0.

## Settings

`<data dir>/settings.json` (0600) holds the extraction model, embeddings, indexing rules and upkeep
intervals; environment variables only seed the defaults. `GET/POST /api/settings` (admin token) read
and apply them; API keys come back masked and a masked value sent back keeps the stored key. Applying
restarts each project; chunks without a vector for the new embedding model are re-embedded on start.

## Plans (design only)
Free: local only. Paid: cloud sync (events carry `device_id` and `op_id`, so sync is operation
shipping). Extra charge: project members. None of this is built.

## Not built
Session-log adapters for Codex and Muse Code (they use `note`), LLM-driven `derives`, an ANN vector
index, connectors to external apps, cloud sync, billing. See [ROADMAP.md](ROADMAP.md).
