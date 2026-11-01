"""Memory score: does MemoryOS hold the answer to each suite E task? Free, takes seconds.

    python3 bench/agents/coverage.py            # against the running app
    python3 bench/agents/coverage.py --json     # also print the rows as JSON

Asks the running app exactly what an agent would get: the brief at session start, then one
search with the task's own prompt. A task's answer has parts (see tasks.json); the score counts
the parts found. No agent and no model calls, so run it after every change to memory or ranking.
Handoff tasks are skipped: their answer only exists after session 1 of a live run.
Pass/fail of a real agent is suite E itself (run.py); this only says whether memory could help.
"""

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from bench import Engram  # noqa: E402  (bench/bench.py: the MCP client with the app's admin token)


def found(groups, text):
    # Lines quoting the benchmark files themselves are not memory.
    text = "\n".join(l for l in text.splitlines() if "bench/" not in l)
    return sum(any(re.search(p, text, re.I | re.M) for p in g) for g in groups)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    spec = json.loads((HERE / "tasks.json").read_text())
    e = Engram()
    rows = []
    for t in spec["tasks"]:
        if t["kind"] == "handoff":
            continue
        root = spec["projects"][t["project"]]["source"]
        groups = t.get("answer") or t.get("diff")
        brief, _ = e.tool("brief", {"cwd": root})
        search, _ = e.tool("search", {"cwd": root, "query": t["prompt"]})
        rows.append({"task": t["id"], "kind": t["kind"], "parts": len(groups),
                     "in_brief": found(groups, brief), "in_brief_or_search": found(groups, brief + "\n" + search),
                     "tokens": (len(brief) + len(search)) // 4})

    print(f"{'task':<20} {'kind':<8} {'brief':>7} {'+search':>8} {'tokens':>7}")
    for r in rows:
        print(f"{r['task']:<20} {r['kind']:<8} {r['in_brief']:>3}/{r['parts']:<3} {r['in_brief_or_search']:>4}/{r['parts']:<3} {r['tokens']:>7}")
    full = lambda key, rs: sum(r[key] == r["parts"] for r in rs)
    history = [r for r in rows if r["kind"] == "history"]
    print(f"\nHistory answers complete in the brief:  {full('in_brief', history)}/{len(history)}")
    print(f"History answers complete with a search: {full('in_brief_or_search', history)}/{len(history)}")
    print(f"All answer parts found (brief + search): {sum(r['in_brief_or_search'] for r in rows)}/{sum(r['parts'] for r in rows)}")
    if args.json:
        print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
