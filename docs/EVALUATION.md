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

### A. Conversational memory: LongMemEval-S *(planned)*

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

### D. Scale and speed *(planned)*

Synthetic projects of 10k, 100k and 1M chunks (and 100k memories), built from public text and code.

- Approximate nearest-neighbour recall@10 against exact search (target ≥ 0.95).
- Search latency p50/p95 at each size (target p95 under 100 ms at 1M chunks).
- Index build time, memory use, disk size.

### E. Live agents, with and without Engram *(planned)*

The test that matters most: real Claude Code and Codex sessions on a real repository.

- A fixed list of tasks, each run twice from the same commit: once with Engram connected, once without.
- Some tasks are seeded with history (an earlier decision or a known dead end) to see whether the agent
  repeats the mistake.
- Measured: tokens used, turns, wall time, whether the task passed its tests, whether a recorded
  decision was contradicted or a dead end repeated, and how often the agent called `search` and `note`.
- Also: one agent records a decision, a second agent in a new session must act on it.

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
| Embeddings (in-process, no Ollama) | BAAI/bge-small-en-v1.5 (384 dims), nomic-embed-text-v1.5 (768 dims, long context, code-aware) | suites A–C dev split, then latency in D |
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

Commands for suites A–E are added here as each one lands.

## Log

- **2026-10-31:** the smoke benchmark's question file was being indexed as part of the Engram project,
  so search found the answer key. Fixed by excluding `bench/` from indexing and from grading; results
  above are after the fix.
