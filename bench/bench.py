"""Engram benchmark: speed, answer size and recall against a running daemon.

Calls the shared MCP endpoint exactly as an agent does (tools/call with `cwd`), using the admin
token from runtime.json, so the requests show up as `engram-ui`, not as a real agent.

    python3 bench/bench.py [--runs 20] [--out bench/results.json]

Recall questions are facts known to be in each project's history; a hit means the expected
phrase is in what the agent would actually receive (the packed answer within the token budget).
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import time
import urllib.request

RUNTIME = os.path.expanduser("~/Library/Application Support/Engram/runtime.json")

PROJECTS = {
    "engram": "/Users/ashishkarigar/Projects/Hackathons/Meta/engram",
    "careerbricks": "/Users/ashishkarigar/Projects/Product Development/careerbricks",
    "LiTheR": "/Users/ashishkarigar/Projects/Miscellaneous/LiTheR",
}

# (project, question, any of these phrases counts as found)
QUESTIONS = [
    ("engram", "what port does the engram daemon listen on", ["41741"]),
    ("engram", "where is uncommitted work saved", ["refs/engram/snapshots"]),
    ("engram", "how are keyword and vector rankings combined", ["Reciprocal Rank Fusion", "RRF"]),
    ("engram", "why is the event log hash chained", ["tamper", "hash chain"]),
    ("engram", "which way may crates depend on each other", ["depend downward", "downward only"]),
    ("engram", "why are codex reviewer sessions skipped", ["guardian", "approval reviewer"]),
    ("engram", "how do agents connect to engram", ["/mcp", "MCP endpoint"]),
    ("engram", "why did we stop running ollama", ["slow", "14B", "lag"]),
    ("engram", "how does the 3d graph rotate", ["rotat"]),
    ("engram", "what local model was used for extraction", ["qwen"]),
    ("careerbricks", "what is the tech stack", ["FastAPI", "Angular"]),
    ("careerbricks", "how is the backend deployed", ["GitHub Actions", "Docker"]),
    ("careerbricks", "how does the request access button work", ["request access", "request-access"]),
    ("LiTheR", "what is the first feature to build", ["segmentation", "drivable"]),
    ("LiTheR", "how is the phone mounted on the car", ["mount", "bumper"]),
]

ID = re.compile(r"\b(mem|doc|chk)_[0-9a-f]{8,}")


class Engram:
    def __init__(self):
        rt = json.load(open(RUNTIME))
        self.url, self._token = rt["url"], rt["adminToken"]

    def _post(self, path, body):
        req = urllib.request.Request(self.url + path, json.dumps(body).encode(),
                                     {"Authorization": "Bearer " + self._token, "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=120) as r:
            return json.load(r)

    def _get(self, path):
        req = urllib.request.Request(self.url + path, headers={"Authorization": "Bearer " + self._token})
        with urllib.request.urlopen(req, timeout=120) as r:
            return json.load(r)

    def tool(self, name, args):
        """One MCP tool call; returns (text, milliseconds)."""
        msg = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}
        t = time.perf_counter()
        out = self._post("/mcp", msg)
        ms = (time.perf_counter() - t) * 1000
        result = out["result"]
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return result["content"][0]["text"], ms

    def timed_get(self, path):
        t = time.perf_counter()
        body = self._get(path)
        return body, (time.perf_counter() - t) * 1000


def tokens(text):
    return len(text) // 4  # same estimate the packer uses


def summary(samples):
    s = sorted(samples)
    return {"p50": round(statistics.median(s), 1), "p95": round(s[min(len(s) - 1, int(len(s) * 0.95))], 1), "max": round(s[-1], 1)}


def latency(e, runs):
    queries = ["why", "how is auth handled", "deployment", "what depends on the database", "decisions about the graph"]
    out = {}
    for name, root in PROJECTS.items():
        per = {}
        calls = {
            "brief": lambda i: e.tool("brief", {"cwd": root}),
            "profile": lambda i: e.tool("profile", {"cwd": root}),
            "search (hybrid)": lambda i: e.tool("search", {"cwd": root, "query": queries[i % len(queries)]}),
            "search (memories)": lambda i: e.tool("search", {"cwd": root, "query": queries[i % len(queries)], "mode": "memories"}),
            "search (documents)": lambda i: e.tool("search", {"cwd": root, "query": queries[i % len(queries)], "mode": "documents"}),
        }
        last = ""
        for label, call in calls.items():
            times, sizes = [], []
            for i in range(runs):
                text, ms = call(i)
                times.append(ms)
                sizes.append(tokens(text))
                last = text
            per[label] = {**summary(times), "tokens": round(statistics.mean(sizes))}
        ids = [m.group(0) for m in ID.finditer(last)]
        if ids:
            times = [e.tool("expand", {"cwd": root, "id": ids[i % len(ids)]})[1] for i in range(runs)]
            per["expand"] = summary(times)
        out[name] = per
    return out


def ui_endpoints(e, runs):
    projects = {p["name"]: p["id"] for p in e._get("/api/projects")}
    out = {}
    for path in ["/api/projects"] + [f"/p/{projects['engram']}/api/{x}" for x in ("graph?archived=0", "activity?days=3", "stats", "documents?limit=500")]:
        times = [e.timed_get(path)[1] for _ in range(runs)]
        out[path.split("/api/")[-1].split("?")[0] if path.startswith("/p/") else "projects"] = summary(times)
    return out


def footprint():
    pid = subprocess.run(["pgrep", "-f", "target/debug/engram-desktop"], capture_output=True, text=True).stdout.split()
    if not pid:
        return None
    samples = []
    for _ in range(10):
        cpu, rss = subprocess.run(["ps", "-o", "pcpu=,rss=", "-p", pid[0]], capture_output=True, text=True).stdout.split()
        samples.append((float(cpu), int(rss)))
        time.sleep(1)
    base = os.path.expanduser("~/Library/Application Support/Engram/projects")
    disk = sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fs in os.walk(base) for f in fs)
    return {"cpu_percent_avg": round(statistics.mean(c for c, _ in samples), 1), "rss_mb": round(max(r for _, r in samples) / 1024, 1),
            "data_dir_mb": round(disk / 1e6, 1), "build": "debug"}


def recall(e):
    rows = []
    for project, question, expected in QUESTIONS:
        text, ms = e.tool("search", {"cwd": PROJECTS[project], "query": question})
        # This script is indexed too and contains every expected answer; it must not grade itself.
        graded = "\n".join(l for l in text.splitlines() if "bench/" not in l)
        hit = next((p for p in expected if p.lower() in graded.lower()), None)
        rows.append({"project": project, "question": question, "found": hit is not None, "matched": hit, "tokens": tokens(text), "ms": round(ms, 1)})
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "results.json"))
    args = ap.parse_args()
    e = Engram()
    results = {"latency_ms": latency(e, args.runs), "ui_ms": ui_endpoints(e, args.runs), "footprint": footprint(), "recall": recall(e)}
    found = sum(r["found"] for r in results["recall"])
    results["recall_summary"] = f"{found}/{len(results['recall'])} questions answered within the token budget"
    json.dump(results, open(args.out, "w"), indent=2)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
