# Engram roadmap: a local Supermemory with code-aware memory at its core

Goal: everything Supermemory does for context (memories, documents/RAG, profiles, forgetting,
review, MCP), running locally and free, plus what it cannot do: watch the project folder and git,
understand code, recover uncommitted work, and attribute every memory and request to an agent.

Deadline: Nov 4, 9 PM ET. Phases are ordered so each one leaves a working, demoable product.

## Parity map

| Supermemory feature | Engram equivalent | Phase |
|---|---|---|
| Documents → chunks (SuperRAG) | `documents` + `chunks`, type-aware chunking, contextual headers | B |
| Extractors (text, HTML, PDF, code AST) | text/markdown, HTML, PDF (pure Rust), code via tree-sitter | B |
| Pipeline statuses | queued → extracting → chunking → embedding → indexing → done/failed | B |
| Embeddings + vector search | `Embedder` trait: built-in hashing (no download), Ollama, OpenAI-compatible | A |
| Hybrid search (memories + chunks) | BM25 + vectors + graph walk, fused with Reciprocal Rank Fusion; modes hybrid/memories/chunks | A |
| Updates / extends / derives | supersedes (have), extends (new), derives (inferred, flagged) | C |
| Preferences strengthen, episodes decay, expiry | reinforcement count, half-life decay (have), `expires_at` | C |
| Inferred-memory review | `is_inference` + approve / decline / undo | C |
| Profiles (static + dynamic) | project profile: conventions and standing decisions vs current focus | C |
| Namespaces | projects (have), one database each | — |
| MCP tools | brief, search (modes), expand, note, profile, add_document | D |
| Connectors | project folder is the connector: docs and code auto-ingested on change | B |
| Console graph + memory browser | desktop app: Obsidian-style graph, documents, review, settings | E |

## Engram-only (the differentiators)

- Watches files and git; uncommitted work saved on `refs/engram/snapshots`.
- Code entities (files, modules, symbols, imports) linked to memories and documents.
- "What breaks if I change X" via graph walk from code nodes.
- Per-agent tokens, attribution, and a request log with reasons.
- Fully local, open code, no document cap.

## Phases

**A. Embeddings and hybrid search** — `engram-embed` crate; vectors stored in SQLite; brute-force cosine
top-k (fine to ~100k vectors; HNSW later); RRF fusion of keyword, vector and graph ranks.

**B. Documents (SuperRAG)** — `engram-rag` crate: schema, extractors, chunkers, ingest pipeline with
content-hash dedupe and versioning, document → code-entity `mentions` edges, auto-ingest of project
docs and code from the watcher.

**C. Memory parity** — extends edges, reinforcement on repeat, expiry, inferred flag and review,
project profile.

**D. Agent surface** — MCP/HTTP: search modes, `add_document`, `profile`, review endpoints.

**E. Desktop app** — Documents tab, search modes, then: Obsidian-style force graph (canvas, physics,
filters, local graph), Settings (models, embeddings, ignore rules, decay), project overview, review queue.

**F. Capture depth** — session-log adapters (Claude Code, Codex, Muse `session.jsonl`), hook injection
on session start and after compaction, Muse Spark extraction and `derives`.

**G. Ship** — benchmark (repeated mistakes, tokens with/without Engram), demo video, README, submit.

## Status

- [ ] A  - [ ] B  - [ ] C  - [ ] D  - [ ] E  - [ ] F  - [ ] G
