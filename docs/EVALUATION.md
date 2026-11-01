# How Engram is evaluated

This document says what we measure, on which data, how, and the rules that keep the numbers honest.
Each suite is marked **done**, **in progress** or **planned**; results are only reported for suites
that have run, with the commit and configuration that produced them.

**Why Supermemory.** We compare against Supermemory because it is the closest direct competitor we
found: a memory layer for AI agents that publishes its benchmark results. Its numbers are its own
(vendor-reported), and we say so wherever we put ours next to them.

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

### A. Conversational memory: LongMemEval-S *(done: test split run once)*

[LongMemEval](https://arxiv.org/abs/2410.10813) gives each question a haystack of about 50 chat sessions
(around 115k tokens) and marks which sessions hold the evidence (`answer_session_ids`). Categories:
single-session (user, assistant, preference), multi-session, knowledge update, temporal reasoning,
abstention.

**Test split result (2026-11-01, commit ed26c32).** This split was never run or tuned on before:
407 questions, 382 without abstention for retrieval. Run once, after tuning on dev ended; the numbers
are as measured.

| | MemoryOS (test, 407 questions) | Supermemory (vendor-reported, 500) |
|---|---|---|
| Evidence session in top 20 (R@20) | **100%** | 97% (with GPT-4o) |
| Evidence session in top 10 / top 5 | 99.7% / 99.0% | — |
| Every evidence session in top 5 (all@5) | 88.0% | — |
| Answer accuracy | **86.7%** (mean over types 85.6%), Muse Spark | 85.2% (Gemini 3 Pro), 84.6% (GPT-5) |

Answer accuracy by type (n): single-session-assistant 100 (43), single-session-user 98.2 (55),
temporal 91.4 (105), knowledge update 88.1 (59), preference 81.8 (22), multi-session 75.5 (98),
abstention 64.0 (25). Retrieval R@5 by type: 100 for all except multi-session 99.0 and preference
86.4. History given to the model: about 9.3k tokens per question.

How to read it:

- **Retrieval is ahead:** the evidence was in the top 20 for every test question, and in the top 5
  for 99%.
- **Answer accuracy is on par, slightly ahead:** 1.5 points over Supermemory's best published figure,
  which is within run-to-run noise. Dev scored 92.5%, so dev was optimistic; only test is reported.
- **Weakest:** multi-session counting ("how many … in total"), where the model misses an item, and
  abstention, where it answers from related but wrong details.

Differences in setup, stated with the numbers:

- Muse Spark answers and judges; the official judge, and Supermemory's, is GPT-4o.
- Our test split is 407 of the 500 questions (a fixed hash split); Supermemory reports on all 500.
- Our accuracy includes the 25 abstention questions.
- 7 model replies hit their token limit (4 answers at 6000, 3 verdicts at 2000) and count as scored.
  A cut-off verdict without "yes" counts as wrong. Raising the limits after seeing test results
  would be a new run, so it is left for the next release.

Result files: `bench/longmemeval/test-retrieval.json`, `bench/longmemeval/test-qa.jsonl`.

- **Ingest:** each question's sessions become a fresh Engram project, through the same transcript
  path real agent sessions use.
- **Retrieval metric (no model needed):** recall_any@K, whether any evidence session appears in the
  top K results, for K = 5, 10, 20, per category; abstention questions excluded. This is directly
  comparable to Supermemory's published LongMemEval-S retrieval figure (vendor-reported 97% Recall@20,
  judged with GPT-4o; we state any differences in setup next to our number).
- **Answer accuracy (needs a model):** retrieve, answer with an LLM, judge with an LLM, following the
  official protocol. The official judge is GPT-4o, and Supermemory reports about 85% with GPT-5 and
  Gemini 3 Pro answering. We use Meta's Muse Spark to answer and to judge, with the official judge
  prompt unchanged, and hand-check a sample of verdicts to report how often we agree with the
  judge. The saved answers can be re-judged with GPT-4o later without re-running anything else.
- **Also recorded:** tokens returned per question, latency p50/p95.
- **Stricter scores reported next to it:** all@5 (every evidence session in the top 5, which multi-session
  questions need), the random-guess baseline (with ~48 sessions per question, R@20 by chance is about 60%,
  so R@5 is the informative number), and whether evidence reached the answer an agent actually receives
  (after the cutoff and token budget).

Run: `cargo run --release -p engram-eval -- longmemeval --data longmemeval_s_cleaned.json --split dev --embed bge-small-q`

**Answer accuracy (`longmemeval-qa`).** LongMemEval's own protocol, with MemoryOS as the retriever:

1. Replay the question's history into a fresh project.
2. Take the top 40 chunks from the ranking `search` uses, counted from the question's date (40
   was chosen over 20 on dev; see below).
3. Group them by session, oldest first, in LongMemEval's layout (`### Session n`, `Session Date`).
4. The model answers with the official reader prompt (`run_generation.py`, retrieval, step by step).
5. The model judges the answer with the official per-type judge prompt (`evaluate_qa.py`), parsed as
   officially: any "yes" counts. Abstention questions are included, as in LongMemEval.

Differences from the official setup, stated with every score:

- Muse Spark answers and judges; the official judge is GPT-4o.
- Token limits are 6000 for answers and 2000 for verdicts, instead of 800 and 10, because a
  reasoning model's thinking counts against them.
- No temperature is set; Meta's API default is used.

Every answer and verdict is saved to a JSONL file as it finishes. A stopped run resumes where it
left off, and the saved answers can be judged again by another model.

```bash
# retrieval on test first: embeds the test histories once (about 2.5 hours, free)
cargo run --release -p engram-eval -- longmemeval --data longmemeval_s_cleaned.json --split test --embed bge-small-q --out results/test-retrieval.json
# then answers on test, with the embeddings cached (model calls; resumable)
cargo run --release -p engram-eval -- longmemeval-qa --data longmemeval_s_cleaned.json --split test --out results/test-qa.jsonl
```

**Dev split, all 93 questions, 2026-11-01** (20 chunks, about 4.7k tokens of history per question):

| Type | n | Accuracy |
|---|---|---|
| single-session-user | 9 | 100 |
| single-session-assistant | 13 | 100 |
| knowledge-update | 13 | 100 |
| multi-session | 23 | 87.0 |
| single-session-preference | 8 | 87.5 |
| temporal-reasoning | 22 | 86.4 |
| abstention | 5 | 80.0 |
| **all** | 93 | **91.4** (mean over types 91.5) |

All 8 wrong verdicts were checked by hand, and the judge was right each time. What went wrong:

- **Counting across sessions (4):** charity total, delivery services, rare items, events before a
  date. The model found most items and missed one. In two of these the missing session was not in
  the top 20 chunks.
- **Retrieval (2):** the smoker ("kitchen appliance", see above), and one of the three sports events
  missing from the context.
- **Preference (1):** the model refused to suggest a Miami hotel because Miami was never discussed,
  instead of using the user's stated taste in hotels.
- **Abstention (1):** the model answered from a bus fare the history did not contain for that route.

**40 chunks instead of 20** (about 9.3k tokens of history per question): **92.5%** (86/93); temporal
95.5, abstention 60.0, the rest unchanged; mean over types 90.0.

- Fixed (4): the charity total, the delivery services, the sports-events order and the smoker.
  These are the misses whose evidence fell outside the top 20.
- Broken (3): a tank count, the plants acquired "in the last month", and the Shinjuku abstention.
  The model reasoned wrongly over context that held the answer.

A net gain of one question is within run-to-run noise. We use 40 for the test run because its fixes
address a known limit (evidence ranked 21–40), and its losses are reasoning slips.

**Dev split (88 questions, abstention excluded), 2026-10-31.** Tuning set; the test split has not been run.

| Embeddings | R@5 | R@10 | R@20 | all@5 | In agent's answer | Answer tokens |
|---|---|---|---|---|---|---|
| Random guess | 18.5 | — | 60.2 | — | — | — |
| Word hashing (no model) | 90.9 | 96.6 | 97.7 | 76.1 | 89.8 | 467 |
| bge-small-en-v1.5, int8 | **97.7** | **97.7** | **98.9** | **90.9** | **97.7** | 519 |

By category with bge-small int8 (R@5 / all@5): knowledge update 100 / 100, multi-session 100 / 87.0,
single-session assistant 100 / 100, preference 100 / 100, single-session user 100 / 100, temporal
reasoning 90.9 / 77.3. Weakest: temporal reasoning, and gathering every session of a multi-session answer.

**Dates in search, dev split, 2026-11-01.** Search now reads time words in the question ("10 days
ago", "a week ago", "the past month", "in February") as a range of dates, counted from when the
question is asked, and adds 0.6 to chunks of sessions dated in that range. Agents ask relative to
now; in this suite "now" is LongMemEval's `question_date`, which the eval did not pass before. The
boost was tuned on the 22 dev temporal questions (0.15, 0.3, 0.6 and 1.0 tried; 0.6 and 1.0 tie).

| | R@5 | R@10 | R@20 | all@5 | In agent's answer |
|---|---|---|---|---|---|
| Before (bge-small int8) | 97.7 | 97.7 | 98.9 | 90.9 | 97.7 |
| With dates | **98.9** | **100** | **100** | 90.9 | 97.7 |
| Temporal only, before | 90.9 | 90.9 | 95.5 | 77.3 | 90.9 |
| Temporal only, with dates | 95.5 | 100 | 100 | 77.3 | 90.9 |

No other category changed. The remaining temporal miss at 5 ("What kitchen appliance did I buy 10
days ago?") is a wording gap: the session says "I just got a smoker", and 15 of its 43 sessions fall
in the date range. all@5 did not move: questions that need every session of a sequence ("the order
of the three sports events") still lose one of them. Still dev only; the test split has not been run.
Search latency with the model, one thread, no other load: 23 ms p50 (query embedding included).

Tuning ended here; the test split result is at the top of this section.

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

### E. Live agents, with and without MemoryOS *(done, 1 full run)*

Muse Code (`muse exec`, headless) does the same tasks twice, from the same commit: once connected to
MemoryOS and once without it. Same model, same settings, same prompt. The prompt never mentions
MemoryOS; in the "with" runs the agent learns about it only from the connector (MCP server and
SessionStart hook).

**Result (2026-11-01, commit c363c56, Muse's default model `muse-spark-1.3`, 3 runs per task and
condition, 84 trials, `bench/agents/results/live-20261101-012109.json`):**

| | With MemoryOS | Without |
|---|---|---|
| Trials passed | **39 / 42** | 26 / 42 |
| Median tokens per trial | 145k | 216k |
| Median uncached tokens per trial | 31k | 58k |
| Median tool calls per trial | 6 | 10 |
| Decisions contradicted (`never` patterns) | 0 | 0 |

Per task (passed of 3, median tokens):

| Task | Kind | With | Without |
|---|---|---|---|
| lt-first-feature | history | 3 · 90k | 0 · 497k |
| lt-baseline-claims | history | 3 · 175k | 0 · 212k |
| lt-mount | history | 3 · 145k | 0 · 262k |
| lt-recording | history | 3 · 88k | 0 · 539k |
| cb-extraction-rules | history | 3 · 234k | 3 · 182k |
| en-token-budget | history | 3 · 90k | 3 · 92k |
| en-retire-rule | history | 3 · 58k | 3 · 143k |
| lt-storage-format | handoff | 3 · 465k | 1 · 622k |
| cb-error-format | handoff | 0 · 2.3M | 1 · 1.5M |
| lt-setup | trap | 3 · 209k | 3 · 200k |
| cb-impact | impact | 3 · 335k | 3 · 339k |
| cb-deploy | in-repo | 3 · 118k | 3 · 116k |
| cb-stack | control | 3 · 83k | 3 · 79k |
| en-port | control | 3 · 145k | 3 · 167k |

What it says:

- **Memory decides the result when the answer exists only in past sessions.** The four LiTheR
  history tasks went 12/12 with memory and 0/12 without; without it the agent searched longer and
  answered wrong. The careerbricks and engram history answers are also written in their repos'
  docs, so both conditions passed those.
- **Where both pass, memory mostly lowers cost.** Uncached tokens, which drive price, were lower
  with memory on every one of the 14 tasks. Total tokens were lower on 9 of 14. Total tokens
  differ by about ±20% between single runs, so small gaps are noise.
- **The trap held.** With a pending venv/Conda conflict in memory, every "with" run still wrote
  Conda.
- **"Without" is not memoryless.** Muse keeps its own per-project `MEMORY.md` under its data folder.
  In one `cb-error-format` and one `lt-storage-format` "without" run, session 1 wrote the decision
  there and session 2 followed it. So the baseline is Muse with its built-in memory.

**The failure: `cb-error-format`, 0/3 with memory.** In all three runs, MemoryOS did its part:

1. session 1 saved the decision with `note`;
2. session 2 got it in the brief, then found it with `search` and `expand`.

The agent then chose the code over it. Every endpoint returns `{"detail": ...}`, the frontend reads
`detail`, and the repo has no mobile client. The agent said the memory was unsupported, wrote
`{"detail": ...}`, and saved a new memory to follow the code convention. That memory would mislead
later sessions. The memory showed the decision but not that the user stated it, while Muse's own
`MEMORY.md` reads as the user's instruction.

**Fix and re-run (commit 22f9111).** Two changes:

- Notes now record who stated them. The brief and search show `[decision, stated by the user]`, and
  agents are told to follow those memories even where the code does not yet.
- An agent's differing note, or a model link, can no longer retire a user-stated memory; it waits
  in Review.

Re-run of `cb-error-format` "with", 3 runs (`results/live-20261101-114056.json`): **3/3 passed**,
1.97M median tokens. In every run session 1 marked the decision as the user's, and session 2 saw the
mark and wrote the `{"error": {"code", "message"}}` shape. The other 13 tasks were not re-run after
the change, so the full-run table above stays as measured. With this task's re-run in place of its
first run, "with" would be 42/42.

The task is still a hard case: it asks for a convention that nothing in the code supports yet.
Handoff tasks are the costliest here (about 2M tokens): the agent sets up a virtualenv and runs the
full backend test suite in both conditions.

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
- Tool calls are counted from Muse's `tool.*` task events, which the live runs confirmed.

**Muse needs the hook.** In the first live trial Muse listed MemoryOS's 9 tools and never called
one. The MCP server alone does not make it use memory. The connector now also adds a SessionStart
hook, as for Claude Code, so the brief is in the context from the first turn. Muse runs hooks with
`HOME` but not `ENGRAM_HOME`, so each trial keeps its daemon's data in the default folder under the
trial's HOME.

**Two levels of score:**

- **Memory score** (`coverage.py`): free, a few seconds, run after any change to memory or ranking.
  It asks the running app what an agent would get (the brief, then one search with the task's
  prompt) and counts the answer parts found. It says whether memory *could* help, not whether an
  agent uses it.
- **Agent score** (`run.py --live`): costs money and takes hours, run before a release or a demo.
  The "without" side only changes when Muse does, so later runs can use `--conditions with` and
  compare with the saved baseline.

```bash
python3 bench/agents/coverage.py                     # memory score, free
python3 bench/agents/run.py                          # dry run: every trial set up, echo provider, free
python3 bench/agents/run.py --live --runs 1 --tasks lt-storage-format --conditions with   # handoff check
python3 bench/agents/run.py --live                   # 14 tasks x 2 x 3 = 96 Muse sessions
```

Results go to `bench/agents/results/`.

**Checks on 2026-11-01, before the full run:**

- dry run, all 28 trials: every "with" session got the brief and a `ready` MCP handshake; no
  "without" session got either; all diffs were empty;
- memory score: all 7 history answers are complete in the brief, and 23 of 26 answer parts are
  found with brief plus search;
- one live `lt-setup` pair: both conditions passed. The "with" agent saw the venv/Conda conflict in
  the brief, checked `environment.yml`, and wrote Conda. Tokens were 213k with and 192k without.
  A pair run before the hook existed came out the other way round, so single runs are noise;
- one live handoff (`lt-storage-format`, "with"): session 1 saved the decision with `note`, and
  session 2 found it and wrote MCAP storage.

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
- **2026-11-01:** suite E's first live trial showed Muse ignoring MemoryOS's MCP tools. The Muse
  connector now also installs a SessionStart hook. The full run above is after that change.
