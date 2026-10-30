# Engram — Action Plan (DRAFT v1)

Hackathon: Meta Global AI Developer Hackathon. Window Oct 26 – Nov 4. Submit by **Nov 4, 9:00 PM ET**.
Track: Indie Developer, solo. Theme: Push the Frontier (Muse Spark on a hard software problem).

## 1. What Engram is

A local-first, continuously running **project memory** for coding agents. Not RAG over notes.
It observes the project (files, git, agent session logs), keeps an immutable event log, distills
that into a **temporal memory graph** (decisions, reasons, rejected options, tasks, code symbols),
and serves a small token-budgeted slice back to any agent via MCP and hooks.

Closest reference: supermemory.ai. Differences: Engram *watches* instead of waiting to be told,
it is grounded in raw evidence, it understands code symbols, and it can block contradicting edits.

### Brain-like properties (the real differentiators)
| Property | Mechanism |
|---|---|
| Never loses anything | Append-only event log; memory is a *derived* view and can be rebuilt |
| Temporal | Every memory has `valid_from`, `valid_to`, `observed_at`; supersede, never overwrite |
| Forgetting | Decay score from recency, access count, code-liveness; stale items archived, not deleted |
| Updating | New evidence creates a new version and a `supersedes` edge; contradictions flagged |
| Consolidation | Periodic "sleep" job merges episodes into semantic facts, dedupes, re-scores |
| Grounded | Each memory links to evidence event ids; unverifiable extractions are rejected |
| Fast recall | Hot-path retrieval is SQL + FTS5 + vectors + graph walk, no LLM call at read time |
| Code-aware | Memories link to tree-sitter symbols; if the symbol is deleted/changed the memory is re-checked |
| Cross-project | Edges can span projects (e.g. two sites sharing one auth pipeline) |

## 2. Architecture (from R1 docs, scoped for hackathon)

```
Agents (Claude Code, Muse Code)        Project folder + git
        │ session logs / hooks                │ chokidar / simple-git
        ▼                                     ▼
   ┌──────────── Capture (daemon) ───────────────┐
   │ adapters → scrub secrets → append events    │
   └───────────────────┬─────────────────────────┘
                       ▼
        SQLite (WAL): events | tasks | memories | edges | symbols | FTS5 | sqlite-vec
                       ▲                 ▲
   ┌───── Understand (workers) ──┐  ┌──── Serve (Fastify + MCP) ────┐
   │ tree-sitter · task grouping │  │ recall · query · impact · API │
   │ extract (Llama local /      │  └───────┬───────────────┬───────┘
   │ Muse Spark) · verify · link │          │ MCP/hooks      │ local API
   └─────────────────────────────┘          ▼                ▼
                                       Agents          Desktop UI (Electron+React+Cytoscape) + CLI
```

Out of scope for hackathon: cloud sync, Postgres, WorkOS, seats/billing. (Mention as roadmap only.)

## 3. Proposed repo layout (pnpm monorepo, TypeScript)

```
engram/
  packages/
    core/        schema, migrations, event types, db access, shared types
    capture/     file watcher, git watcher, secret scrubber, adapter interface
    adapters/    claude-code, muse-code (log tailers + hook installers)
    understand/  tree-sitter indexer, task grouper, extractor, verifier, linker, decay/consolidation
    models/      Muse Spark client, Ollama client, router (local vs Muse)
    serve/       Fastify API, MCP server, retrieval + budget packer, query, impact
    cli/         `engram` command
    app/         Electron + React + Cytoscape UI
  bench/         repeated-mistake + token benchmark harness, fixtures, results
  docs/
```

## 4. Build phases (today is Oct 30; ~5 days left, so slices must be demoable)

**Phase 0 — Today (foundation)**
- Repo scaffold, TS config, lint, SQLite schema + migrations, event log API.
- Confirm Muse Spark API key + credits; confirm Muse Code log format/hook/MCP support.

**Phase 1 — Capture**
- File + git watcher, secret scrubber, Claude Code adapter, Muse Code adapter.
- Done when: working in a folder produces ordered events in SQLite.

**Phase 2 — Understand**
- tree-sitter symbol index; task grouping; extractor (decision / reason / rejected option);
  evidence verifier; symbol linker; temporal versioning + supersede.
- Done when: a real session yields verified, linked memories.

**Phase 3 — Recall**
- Hybrid retrieval (FTS5 + vector + graph), token-budget packer, MCP server, hooks injection
  at task start and on context compaction.
- Done when: a fresh agent session receives correct context without being told.

**Phase 4 — Query, impact, decay**
- Natural-language query over the graph ("what breaks if auth goes session -> token?", "why was it session based?"), dependency/impact answers, decay/archive job.
- NO guard / no blocking. Memory only informs.

**Phase 5 — UI**
- Graph view (Cytoscape), timeline, memory detail with evidence, archived toggle.

**Phase 6 — Benchmark, demo, submit (do not build features here)**
- Scripted tasks run with/without Engram: repeated mistakes, tokens spent, time.
- 3-min demo video, README clearly marking what was built in-window, public repo, submit.

Out of scope: blocking guard.
Cut order if behind: Electron shell (use browser UI) → impact view → local Llama (Muse only) → consolidation job.

## 5. Working agreement
- Claude Code builds the core. Muse Code takes self-contained pieces (e.g. its own adapter, benchmark fixtures) and acts as the second agent in the demo.
- **No author or co-author lines for Claude or Muse anywhere** (commits, files, README, headers).
- Small commits from the start so history proves in-window work. Public repo.
- Never commit secrets or API keys; `.env` is git-ignored.

## 6. Open questions
See the questions asked in chat; answers get recorded here.

## 7. Product decisions so far (from owner)
- Purpose: agent-agnostic memory so context survives compaction and agent switches (Claude -> Codex -> Muse) with no re-briefing.
- Must remember: code changes, decisions + reasons, what connects where and why, all with time.
- Primary use: an agent or user asks "I'm changing X, what's affected / what are the complications / why was it built this way?" and gets a grounded answer from memory.
- Decay/forgetting modeled on supermemory: typed memories (facts persist until superseded; preferences strengthen with repetition; episodes decay unless significant; temporary items expire after their date), versions kept, newest marked current.
- Two timestamps per memory like supermemory: document date (when observed) and event date (when it applies), plus validity range.
- Platform: run several agents at once (e.g. Claude on backend auth, Codex on billing page) sharing one memory.
- Demo project: owner has one in mind (TBD).
- Credits: Meta API key available, credits pending; develop against local Ollama until they arrive.

## 8. Update: observe-only, open endpoint, pricing (from owner)
- NO building until the owner approves the plan. Planning only.
- Observe-only for now. The multi-agent hosting platform (launching agents) is later.
- Any authorized AI reaches Engram through one endpoint (MCP + HTTP API, token auth). Read path (recall/query/impact) is agent-agnostic; adding an agent should not need per-agent code.
- Passive capture is separate: file + git watching works for any agent with zero code; richer reasoning capture from an agent's own session logs is an optional adapter.
- Pricing (design for it, stub billing at hackathon):
  - Free: local only, no account.
  - Paid: cloud sync (charge).
  - Additional charge: adding project members.
  - Implications: event schema carries device id + operation id (sync-ready), entitlement checks live in one module, endpoint tokens are scoped per project.

## 9. Update: capture model and Muse role
- Agents send their own notes (decisions, reasons) to Engram through the endpoint. This is the primary source of "why".
- The local watchdog independently records file and git changes. Purpose: recovery. Work that was never committed/pushed (a lost feature) can be reconstructed from the event log. This requires storing diffs/content snapshots, not just file names.
- All three agents (Claude Code, Muse Code, Codex) use the same endpoint. Log adapters are optional extras.
- Muse Spark must be central, not an add-on (hackathon theme fit). R1 diagram sends only "hard cases" to Muse; revised: Muse Spark is the primary reasoning model for extraction, linking, consolidation and query answering. A small local model is only a cheap pre-filter/fallback.

## 10. Update: decisions + open design work
- Demo project: existing Python autonomous-driving project (LiDAR + computer vision), not large. Needs tree-sitter-python.
- Engram stores CONTEXT (events, decisions, reasons, links), NOT file contents. Code is pulled from git by agents. Supersedes the "content snapshots" idea in section 9.
- Observability is a first-class feature: log every endpoint call (which agent, what it asked, what memories were returned and why, tokens, latency) so debugging is easy.
- Agents in demo: Claude Code, Muse Code, Codex (all three).
- Muse's exact role is deferred until memory/graph design is settled.
- Open design topics (in progress): graph schema, node/edge types, entity identity across renames, database choice, retrieval, decay.

## 11. Recall design principle (owner-driven)
- Design from the agent's post-compaction problem: it must re-orient fast without re-reading the codebase or re-asking the user.
- Graph recall = find a few MAJOR nodes (subsystems, decisions, active task), then expand to MINOR connected nodes under a token budget, then drill to evidence/code pointers only on demand.
- Uncommitted work is snapshotted to a hidden git ref (refs/engram/*), approved by owner. No file contents in the Engram DB.

## 12. Update: shared state and zero-config UX
- One shared task state per project, not per agent. Every contribution (event, decision, edit, note) is attributed to the agent/session/user that made it (`authored_by` edge), so "who did what" is queryable without duplicate state.
- User is hands-off. Flow: pick a directory in the UI -> it becomes a project -> click to connect the agents wanted -> everything else is automatic (watching, snapshots, extraction, graph maintenance, major-node promotion, summaries, decay).
- Consequence: no manual graph curation. Major nodes are model-proposed; user may rename later but never has to.
- "Connect agent" must auto-configure that agent (MCP entry, instructions telling it to call Engram) with no manual editing.

## 13. Update: connectors and project setup
- "Connect" = register Engram as a connector in each agent (Claude Code, Codex, Muse Code) pointing at a local, token-authenticated MCP endpoint served by the daemon.
- Transport: MCP (streamable HTTP on localhost) with a deliberately tiny tool surface (e.g. brief / search / expand / note). Cloudflare "Code Mode" is a pattern on top of MCP (agent writes code against a typed API), not a replacement; revisit later if tool-token cost matters.
- Projects require git. If the folder is not a repo, Engram runs git init.
- Engram data lives outside the project folder (app data dir, one SQLite file per project).
