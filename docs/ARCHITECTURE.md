# Engram architecture

Engram is a local-first memory layer shared by every coding agent working in a folder.
It observes the project, keeps an immutable record, distills it into a temporal graph,
and serves small, token-budgeted answers over MCP and HTTP.

## Data flow

```
 agents (Claude Code, Codex, Muse Code)            project folder + git
        │  MCP: note / brief / search / expand             │ fs events, commits
        ▼                                                  ▼
 ┌─────────────────────────── daemon ────────────────────────────────────┐
 │ capture ──► events (append-only, hash-chained) ──► understand         │
 │   watcher, git, scrubber,        │                  tasks, extract,   │
 │   snapshots (refs/engram/*)      │                  verify, link,     │
 │                                  ▼                  decay             │
 │            SQLite: events · memories · entities · edges · FTS5        │
 │                                  ▲                                    │
 │                         serve: recall · budget · MCP · HTTP API       │
 └──────────────────────────────────┬────────────────────────────────────┘
                                    ▼
                         app (Electron + React + Cytoscape)
```

## Packages

| Package | Responsibility | Depends on |
|---|---|---|
| `core` | Schema, migrations, event log, memories, graph, access log. No I/O besides SQLite. | – |
| `capture` | Watch files and git, scrub secrets, save uncommitted work to a git ref. | core |
| `understand` | Index Python, group events into tasks, extract and verify memories, decay. | core |
| `serve` | Recall ranking, token budget packer, MCP server, HTTP API, auth. | core, understand (notes) |
| `daemon` | Wires everything for one project; project registry; `engram` CLI. | all above |
| `app` | Desktop UI: graph, timeline, memory detail, request log. | HTTP API only |

Rule: packages depend downward only. `app` never imports server code; it uses the HTTP API.

## Storage

One SQLite file per project, stored outside the project in the user's app data directory
(`~/Library/Application Support/Engram/projects/<project-id>/engram.sqlite` on macOS).
Engram never stores file contents. Code is read from git. Uncommitted work is saved as
commits on `refs/engram/snapshots`, built with a temporary index so the user's index,
branches and working tree are never touched.

### Tables
- `events`: immutable, sha256 hash chain, idempotent on `op_id`. Memory is derived and rebuildable.
- `memories`: typed (decision, fact, preference, episode, rejected_option, dead_end, task).
  Bitemporal: `observed_at`, `valid_from`/`valid_to`, `event_date`, plus `commit_sha`.
- `entities`: files, modules, symbols. `renamed_from` keeps identity across renames.
- `edges`: typed, weighted, time-bounded links between any two node ids.
- `memory_evidence`: memory → supporting event seqs. Unsupported memories are rejected.
- `access_log`: every agent request, what came back and why, tokens, latency.

## Algorithms and why

| Need | Choice | Why |
|---|---|---|
| Tamper-evident history | Hash chain over events | O(n) verify, O(1) append |
| Keyword recall | FTS5 + BM25, weighted columns | Built into SQLite, no service |
| Neighbour walk | Recursive CTE, per-hop score decay, edge-type filter | One query, depth-bounded, no graph DB |
| Fit a token budget | Greedy by value density (score / tokens) | Standard knapsack approximation; O(n log n) |
| Rank | `bm25 × graph proximity × recency × salience × validity` | Cheap, explainable, logged per hit |
| Forgetting | Exponential decay with half-life by memory type | Facts persist until superseded; episodes fade |
| Group events into tasks | Sessionization by idle gap | Linear scan over time-ordered events |
| Grounding | Quote check: cited text must appear in cited events | Rejects hallucinated memories cheaply |

Recall never calls an LLM. Models are only used on the write path.

## Ranking inputs
`recall.ts` merges FTS hits with a graph expansion from the best hits. Each result carries
a `reason` string (e.g. `bm25 3.2 · 1 hop via touches · recent`) that is written to the access log.

## Memory lifecycle
observe → event → task grouping → model extraction → grounding check → entity linking →
`current` memory → superseded (kept as history) / archived (decay) / expired (date passed).

## Models
`LlmClient` is a one-method interface. Providers: Meta API (Muse Spark), Ollama (local),
and a scripted fake for tests. Choice is configuration, not code. Embeddings and vector
search are not implemented yet; `memory_embeddings` reserves the table.

## Agent integration
Agents connect to a local MCP endpoint (streamable HTTP, bearer token scoped to the project).
Tools are deliberately few: `brief`, `search`, `expand`, `note`. Agents that lack MCP use the
same operations over HTTP. Agents write their reasoning with `note`; the watcher independently
records what changed, so lost or unpushed work is recoverable from snapshots.

## Why Electron + React, not Rust
The engine is TypeScript on Node's built-in SQLite, so the UI shares types and the HTTP API
with no FFI or second toolchain. The shell is thin: the heavy work is in the daemon, and the
graph is drawn by Cytoscape in a web view either way. Rust (Tauri) would shrink the install
size but add a second language and build chain, costing days we do not have. The UI only
speaks HTTP, so swapping the shell later is a small change.

## Plans (design only)
Free: local only. Paid: cloud sync (events carry `device_id` and `op_id`, so sync is
operation shipping). Extra charge: project members. Entitlements will live in one module;
none of this is built.

## Connectors
`engram connect <agent>` shows, then applies, one config change per agent: Codex (`~/.codex/config.toml`
marked block), Muse Code (`mcpServers` in `~/.config/muse/settings.json`, entry format unverified),
Claude Code (`claude mcp add`). Each agent gets its own token, so every request is attributed.

## Not built yet
Vector search, session-log adapters for Codex and Muse Code (they use `note` instead),
cloud sync, billing.
