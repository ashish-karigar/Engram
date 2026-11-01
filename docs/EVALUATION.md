# How Engram is evaluated

This document says what we measure, on which data, how, and the rules that keep the numbers honest.
Each suite is marked **done**, **in progress** or **planned**; results are only reported for suites
that have run, with the commit and configuration that produced them.

## Rules

1. **Public data first.** Accuracy claims rest on public benchmarks other systems also report on, not
   on questions we wrote after seeing our own output.
2. **Dev and test are separate.** Each benchmark is split once, by a hash of the question id, into dev
   (20%) and test (80%). Ranking weights, cutoffs and model choices are tuned on dev only. Test is run
   once per release; a change made after looking at test results is a new release and says so.
3. **Report everything.** Per-category scores, failures and latency are published with the averages,
   including categories where Engram is weak.
4. **Same setup when comparing.** When we put a number next to another system's, we state any
   difference in setup (metric, judge model, subset). Vendor-reported numbers are labelled as such.
5. **No self-grading.** Evaluation files are excluded from indexing so a benchmark can never find its
   own answer key (we hit this once; see the log at the end).
6. **Ablations justify components.** Each part of the search pipeline (embeddings, reranker, graph walk,
   coverage scoring) is measured with and without, so every component earns its place.

## Suites

### A. Conversational memory: LongMemEval-S *(in progress: dev split done)*

[LongMemEval](https://arxiv.org/abs/2410.10813) gives each question a haystack of about 50 chat sessions
(around 115k tokens) and marks which sessions hold the evidence (`answer_session_ids`). Categories:
single-session (user, assistant, preference), multi-session, knowledge update, temporal reasoning,
abstention.

- **Ingest:** each question's sessions become a fresh Engram project, through the same transcript
  path real agent sessions use.
- **Retrieval metric (no model needed):** recall_any@K, whether any evidence session appears in the
  top K results, for K = 5, 10, 20, per category; abstention questions excluded. This is directly
  comparable to Supermemory's published LongMemEval-S retrieval figure (vendor-reported 97% Recall@20,
  judged with GPT-4o; we state any differences in setup next to our number).
- **Answer accuracy (needs a model):** retrieve, answer with an LLM, judge with an LLM, following the
  official protocol. The official judge is GPT-4o; if we use a different judge we say which.
- **Also recorded:** tokens returned per question, latency p50/p95.
- **Stricter scores reported next to it:** all@5 (every evidence session in the top 5, which multi-session
  questions need), the random-guess baseline (with ~48 sessions per question, R@20 by chance is about 60%,
  so R@5 is the informative number), and whether evidence reached the answer an agent actually receives
  (after the cutoff and token budget).

Run: `cargo run --release -p engram-eval -- longmemeval --data longmemeval_s_cleaned.json --split dev --embed bge-small-q`

**Dev split (88 questions, abstention excluded), 2026-10-31.** Tuning set; the test split has not been run.

| Embeddings | R@5 | R@10 | R@20 | all@5 | In agent's answer | Answer tokens |
|---|---|---|---|---|---|---|
| Random guess | 18.5 | — | 60.2 | — | — | — |
| Word hashing (no model) | 90.9 | 96.6 | 97.7 | 76.1 | 89.8 | 467 |
| bge-small-en-v1.5, int8 | **97.7** | **97.7** | **98.9** | **90.9** | **97.7** | 519 |

By category with bge-small int8 (R@5 / all@5): knowledge update 100 / 100, multi-session 100 / 87.0,
single-session assistant 100 / 100, preference 100 / 100, single-session user 100 / 100, temporal
reasoning 90.9 / 77.3. Weakest: temporal reasoning, and gathering every session of a multi-session answer.
Search latency with the model, one thread, no other load: 23 ms p50 (query embedding included).

Not yet comparable to Supermemory's figure: theirs is reported on the full set with GPT-4o in the loop;
ours is retrieval-only on dev. The test-split number will be reported once, after tuning ends.

### B. Code awareness: file localization on SWE-bench Lite *(planned)*

Given a real GitHub issue, can Engram point at the files that the accepted fix changed? This tests
what Supermemory does not claim: understanding a codebase.

- **Data:** a fixed subset of [SWE-bench Lite](https://www.swebench.com/) instances (repositories at the
  issue's base commit), chosen by hash, not by hand.
- **Metric:** Acc@1 / Acc@5, whether a file from the gold patch is in the top 1 / 5 files Engram returns
  for the issue text; plus MRR.
- **Baselines:** BM25 alone, and embeddings alone, on the same files.

### C. Your projects *(planned)*

Questions about careerbricks, LiTheR and Engram itself, **written by the project owner**, with the
expected answer and where it lives. Engram's author does not write or edit them. Stored as
`eval/owner/<project>.jsonl` with fields `question`, `answer`, `evidence` (file, commit or session).
Metric: found within the default token budget, judged by matching the evidence location, plus
tokens returned.

### D. Scale and speed *(done, synthetic)*

Synthetic projects of 10k, 100k and 1M chunks: Zipf-distributed words (so keyword search sees common
and rare words like real text) and structured unit vectors (a chunk is near its document, documents near
their topic), because embedding a million real texts on a laptop takes about 12 hours. Queries are new
noisy variants of existing documents. Measured on the real search path; Apple M3 Pro, release build.

Run: `cargo run --release -p engram-eval -- scale --dir <folder> --sizes 10000,100000,1000000`

| Chunks | Recall@10 vs exact | Similarity of results vs exact | Vector search p50 / p95 | Exact scan p50 | Full search p50 / p95 | Disk | Memory |
|---|---|---|---|---|---|---|---|
| 10k | 0.995 | 1.000 | 0.3 / 0.4 ms | 10 ms | 1.1 / 1.7 ms | 42 MB | 44 MB |
| 100k | 1.000 | 1.000 | 0.7 / 1.0 ms | 101 ms | 5.6 / 11.3 ms | 409 MB | 155 MB |
| 1M | 0.995 | 1.000 | 0.4 / 0.7 ms | 1.0 s | 45 / 106 ms | 4.1 GB | 1.3 GB |

Index build (first search after crossing 5,000 vectors, all cores): 0.3 s at 10k, 7 s at 100k, 65 s at 1M;
saved to disk and reloaded in under half a second afterwards.

Against the targets above: recall ≥ 0.95 is met at every size; p95 under 100 ms at 1M is **missed
narrowly (106 ms)**; the remaining time is keyword ranking of moderately common words.

What it took to get here, in order (all on this suite):

| Change | 1M recall@10 | 1M vector p50 | 1M full search p95 |
|---|---|---|---|
| HNSW, 16 links, search width 128 | 0.675 (results 0.82 as similar as exact) | 45 ms | 907 ms |
| 32 links, width 256; keyword search skips very common words at scale | 0.995 | 45 ms | 180 ms |
| Hits looked up by rowid (the query walked every vector of the model) | 0.995 | 0.4 ms | 133 ms |
| Common-word cutoff 20% → 5% (projects of 10k+ chunks only) | 0.995 | 0.4 ms | 106 ms |

Caveat: uniformly random vectors (no structure at all) are a worst case for any approximate index; on
them an early configuration scored 0.18 recall@10. Real embeddings are structured, which is why the
suite uses structured vectors, but this suite does not replace a measurement on real embeddings at scale.

### G. System map and impact *(done, small samples)*

The map is built from code alone (no model): parts from manifests, HTTP routes and calls across
languages, linked into endpoints with handlers and callers, plus external services.

Run: `cargo run --release -p engram-eval -- map <repo>` and `-- history <repo> --commits 400`

**Map accuracy against an independent answer.** For careerbricks, Codex had explained "how the frontend
and backend connect" by reading the code, before the map existed. Scored against that answer:

| Codex listed | Engram's map |
|---|---|
| 16 endpoints by name | 16 / 16, each with its handler function |
| the frontend service method calling each | 16 / 16 linked |
| "email … and related routes" | all 8 email routes, with handlers and callers |
| Auth0, Ollama `POST /api/generate`, Gmail / Google OAuth | all 3 |

Also found, not in Codex's answer: `GET /health/ready` (tests only) and `GET /email/callback` (no caller in
the repo; Google calls it). Unmatched calls after fixes: 0 in careerbricks, LiTheR and Engram. LiTheR and
Engram were checked by hand only (no independent answer exists), so they are not scored here.

**Impact against git history.** For each commit that changed 2–25 indexed files: did impact for one
changed file name the others? Compared with an imports-only walk (what the earlier graph could see).

| Repo | Companion files found | …in another part | …server file → caller in another part | Predicted files that changed |
|---|---|---|---|---|
| careerbricks (20 commits) | 12.7% (imports-only 10.6%) | 3.9% (0.0%) | **6 / 6 (0 / 6)** | 21% |
| Engram (33 commits) | 27.0% (26.2%) | 19.8% (18.2%) | **9 / 9 (3 / 9)** | 22% |

Reading it honestly: co-change is a weak truth (feature commits change unrelated files together), so
overall recall is low for any dependency graph. Where the map is meant to help, API-contract changes, it
found every companion file the imports-only graph missed, but on only 15 pairs. Precision (about a fifth
of predicted files actually changed) is limited by file-level granularity: a page that imports an API
service is listed even if it does not use the changed method. Function-level calls are the next step.

### H. Linking memories with the model *(done, small labelled set)*

Muse Spark labels how a memory relates to a neighbour (same, replaces, extends, depends on,
contradicts, or unrelated). Code then decides what that label may change; see
`crates/understand/src/judge.rs`. Each case is one memory and one candidate, with the relations a
person would accept. Cases go through the same call and the same apply/review/drop rule as the app.

- **Set:** 30 pairs (`crates/eval/data/links.json`): 4 same, 5 replaces, 4 extends, 3 depends on,
  4 contradicts (2 against facts from the code), 10 unrelated pairs chosen to share words or a
  component.
- **Run:** `cargo run -p engram-eval -- links` (uses `META_API_KEY`, else the model set in the app).

| Model | Correct relation | Unrelated pairs linked | Real links missed | Memories wrongly retired without review |
|---|---|---|---|---|
| muse-spark-1.3, 3 runs | 29, 29, 28 of 30 (93–97%) | 0 | 1–2 (borderline "adds detail") | 0 |

Caveats: we wrote the pairs ourselves, and the prompt's wording of "extends" was adjusted once after
seeing misses on this set, so this is a development set, not a held-out one. One earlier run put two
test pairs into the prompt as examples; that inflated the score and was discarded. The first run
scored 47% because Muse Spark's thinking counts against `max_tokens`: replies came back empty or cut
off. The budget is now 8000 tokens, and cut-off replies are logged.

On live projects the rules mattered more than the score. A memory from chat ("uses venv, not Conda")
retired a commit-backed decision; since then, a chat-only memory can't retire one a commit backs, a
memory in conflict with the code retires nothing, and every applied link can be undone.

### E. Live agents, with and without MemoryOS *(built, not yet run)*

Muse Code (`muse exec`, headless) does the same tasks twice, from the same commit: once connected to
MemoryOS and once without it. Same model, same settings, same prompt. The prompt never mentions
MemoryOS; in the "with" runs the agent learns about it only from the MCP server's own instructions.

The 14 tasks are in `bench/agents/tasks.json`. They and their pass checks were committed before any
run.

| Kind | Tasks | What it shows |
|---|---|---|
| history | 7 | Facts and reasons that exist only in past sessions (LiTheR's first feature, recording, mount; careerbricks' extraction rules; engram's token budget and retire rule) |
| handoff | 2 | Session 1 states a decision; a new session 2 must follow it without being told |
| trap | 1 | LiTheR setup docs. The code says Conda, but memory holds a pending conflict ("venv"). This shows whether memory can make things worse |
| impact | 1 | Callers and tests of `PATCH /jobs/{id}` |
| in-repo, control | 3 | Answers that are in the repo's docs or code. Here memory should not help much; any difference is cost, not correctness |

**Isolation.** Each trial runs:

- in a fresh clone at the commit pinned in `tasks.json`;
- under a temporary HOME, so the user's Muse settings and MCP servers are never loaded;
- in "with": against its own MemoryOS daemon, on a copy of the project's memory, connected through the
  normal Muse connector.

The real app's memory is never read by an agent or written to. The engram copy hides `bench/`, so the
agent can't read the expected answers. The trial daemon's background model is off, so memory stays as
copied. Handoff tasks are the exception: for them MemoryOS has to capture session 1.

**Measures** (median of 3 runs per task and condition):

- pass/fail, from regular expressions on the final answer and on the diff;
- decisions contradicted (the `never` patterns);
- input and output tokens, from Muse's `run.usage` event;
- tool calls;
- wall time.

**Caveats:**

- The tasks were written by MemoryOS's author, from facts known to be in memory, so this measures what
  memory adds when it holds the answer. It is not a random sample of work.
- Grading by pattern can miss a right answer worded differently. Every trial keeps its JSONL, prompt
  and diff for a manual check.
- The tool-call count comes from Muse's task events. The event names were seen only in dry runs, so
  check them against the first live run.

```bash
python3 bench/agents/run.py                          # dry run: every trial set up, echo provider, free
python3 bench/agents/run.py --live --runs 1 --tasks lt-setup   # one real trial first
python3 bench/agents/run.py --live                   # 14 tasks x 2 x 3 = 96 Muse sessions
```

Results go to `bench/agents/results/`. The dry run on 2026-11-01 completed all 28 trials:

- the MCP handshake was `ready` in every "with" trial and absent in every "without" trial;
- the brief from the trial daemon was 2.4–4.7k characters;
- all diffs were empty.

### F. Smoke benchmark *(done)*

`bench/bench.py`: tool latency, answer size, app footprint, and 15 questions about the three local
projects. Written by Engram's author, so it is a regression check only, not an accuracy claim.

| Date | Commit | Questions found | Avg answer tokens | Hybrid search p50 |
|---|---|---|---|---|
| 2026-10-31 | `d5b2ac8` (before ranking change) | 14 / 15 | 1,065 | 42–117 ms |
| 2026-10-31 | `ef76988` (ranking change) | 15 / 15 | 529 | 9–35 ms |

App CPU during an active agent session: 8.8% average before `2ae2fe5`, 0.4% after (2-minute samples,
debug build). The 9–35 ms latency was measured right after a restart, when the app was idle; part of the
drop from 42–117 ms is that, not the ranking change.

## Models under evaluation

| Role | Candidates | Chosen by |
|---|---|---|
| Embeddings (in-process, no Ollama) | BAAI/bge-small-en-v1.5 (384 dims; int8: 23 texts/s, 0.36 GB), nomic-embed-text-v1.5 (768 dims, long context, code-aware; 4 texts/s, 3.5 GB on this CPU; its int8 build is unusable because its vectors depend on the batch) | suites A–C dev split, then latency in D |
| Reranker | BAAI/bge-reranker-base (cross-encoder) | suites A–C dev split, with and without |
| Extraction and inferred memories | Muse Spark (when credits arrive); Meta Llama 3.3 70B via an OpenAI-compatible host until then | precision of extracted memories on a hand-checked sample, grounding rejection rate |

Training our own embedding model is not planned: off-the-shelf models trained on hundreds of millions of
pairs will beat anything we could train in days. If suite B shows code retrieval is weak, the next step
is fine-tuning a small model on pairs mined from the projects themselves (commit message ↔ changed code,
doc section ↔ file it describes), evaluated on the same splits.

## Running

```bash
python3 bench/bench.py                  # suite F, against the running app
```

Suite E commands are in its section above.

## Log

- **2026-10-31:** the smoke benchmark's question file was being indexed as part of the Engram project,
  so search found the answer key. Fixed by excluding `bench/` from indexing and from grading; results
  above are after the fix.
