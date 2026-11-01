"""Suite E: the same coding agent on the same tasks, with and without MemoryOS.

    python3 bench/agents/run.py                 # dry run: sets every trial up, Muse's free echo provider, no model calls
    python3 bench/agents/run.py --live          # real runs with Muse Spark (asks before spending)
    python3 bench/agents/run.py --live --tasks lt-setup,cb-deploy --runs 1

Every trial is isolated, so nothing touches your real memory, repos or Muse settings:
  - the repo is a fresh clone at the commit pinned in tasks.json, with the remote removed;
  - Muse runs with its own HOME, so your Muse settings, rules and MCP servers are not loaded;
  - "with": a separate MemoryOS daemon (own ENGRAM_HOME, free port) on a copy of the project's
    memory, connected to Muse through the normal connector. It sees only this trial's sessions;
  - "without": no daemon and no MCP server. Muse has only the repo.
The background model is off in the trial daemon (memory stays as copied), except for handoff
tasks, where MemoryOS has to capture session 1 for session 2 to use it.
"""

import argparse
import hashlib
import json
import os
import random
import re
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
APP = Path.home() / "Library/Application Support/Engram"
MUSE = Path.home() / ".local/bin/muse"
DAEMON = REPO / "target/debug/engram"


def load_tasks(only):
    spec = json.loads((HERE / "tasks.json").read_text())
    tasks = spec["tasks"]
    if only:
        wanted = set(only.split(","))
        unknown = wanted - {t["id"] for t in tasks}
        if unknown:
            sys.exit(f"unknown task ids: {', '.join(sorted(unknown))}")
        tasks = [t for t in tasks if t["id"] in wanted]
    return spec["projects"], tasks


def app_settings():
    return json.loads((APP / "settings.json").read_text())


def muse_key():
    """The Meta key Muse is given on stdin. Read, never printed."""
    if os.environ.get("MUSE_API_KEY"):
        return os.environ["MUSE_API_KEY"]
    llm = app_settings().get("llm", {})
    return llm.get("metaApiKey") if llm.get("provider") == "meta" else None


def sh(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True, capture_output=True)


# ---------- the trial's repo ----------

def clone(project, dest):
    sh("git", "clone", "--quiet", "--no-hardlinks", "--no-checkout", project["source"], str(dest))
    sh("git", "checkout", "--quiet", project["commit"], cwd=dest)
    sh("git", "remote", "remove", "origin", cwd=dest)
    for hidden in project.get("hide", []):
        target = dest / hidden
        shutil.rmtree(target, ignore_errors=True) if target.is_dir() else target.unlink(missing_ok=True)
    if project.get("hide"):
        sh("git", "-c", "user.name=bench", "-c", "user.email=bench@localhost", "commit", "--quiet", "-am", "hide benchmark files", cwd=dest)
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=dest, capture_output=True, text=True).stdout.strip()


def changes(repo, base):
    sh("git", "add", "-A", cwd=repo)
    return subprocess.run(["git", "diff", "--cached", base], cwd=repo, capture_output=True, text=True).stdout


# ---------- the trial's MemoryOS ----------

class Daemon:
    """A MemoryOS daemon of its own, on a copy of one project's memory."""

    def __init__(self, project, repo, home, base, with_model):
        self.base = base
        root = os.path.realpath(repo)
        self.id = hashlib.sha256(root.encode()).hexdigest()[:12]
        dest = base / "projects" / self.id
        dest.mkdir(parents=True)
        src = sqlite3.connect(f"file:{APP / 'projects' / project['memory'] / 'engram.sqlite'}?mode=ro", uri=True)
        out = sqlite3.connect(dest / "engram.sqlite")
        src.backup(out)
        src.close()
        out.close()
        (base / "projects.json").write_text(json.dumps([root]))
        settings = app_settings()
        if not with_model:
            settings["llm"] = {**settings["llm"], "provider": "none", "metaApiKey": None}
        settings["memory"] = {**settings.get("memory", {}), "extractEverySecs": 10}
        self.settings = base / "settings.json"
        self.settings.write_text(json.dumps(settings))
        self.settings.chmod(0o600)
        (base / "models").symlink_to(APP / "models")
        env = {"HOME": str(home), "ENGRAM_HOME": str(base), "PATH": os.environ["PATH"]}
        self.log = open(base / "daemon.log", "w")
        self.proc = subprocess.Popen([str(DAEMON), "daemon", "--port", "0"], env=env, stdout=subprocess.PIPE, stderr=self.log, text=True)
        ready = None
        deadline = time.time() + 90
        while time.time() < deadline:
            line = self.proc.stdout.readline()
            if not line:
                break
            if line.startswith("ENGRAM_READY "):
                ready = json.loads(line.split(" ", 1)[1])
                break
            self.log.write(line)
        if not ready:
            self.stop()
            raise RuntimeError(f"trial daemon did not start; see {base / 'daemon.log'}")
        threading.Thread(target=lambda: shutil.copyfileobj(self.proc.stdout, self.log), daemon=True).start()
        self.url, self._token = ready["url"], ready["adminToken"]

    def call(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.url + path, data, method=method,
                                     headers={"Authorization": "Bearer " + self._token, "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)

    def stats(self):
        return self.call("GET", f"/p/{self.id}/api/stats")

    def settle(self, limit=180):
        """Wait for the startup scan of the fresh clone to finish."""
        last, steady, deadline = None, 0, time.time() + limit
        while time.time() < deadline and steady < 3:
            now = self.stats()
            now = (now.get("documents"), now.get("chunks"))
            steady = steady + 1 if now == last else 0
            last = now
            time.sleep(2)

    def hide(self, prefixes):
        """Drop indexed copies of files the clone hides (the benchmark's own answers)."""
        removed = 0
        for doc in self.call("GET", f"/p/{self.id}/api/documents?limit=100000"):
            path = doc.get("path") or doc.get("title") or ""
            if any(path.startswith(p) for p in prefixes):
                self.call("POST", f"/p/{self.id}/api/documents/delete", {"id": doc["id"]})
                removed += 1
        return removed

    def brief_size(self, repo):
        """Characters of brief an agent working in the clone gets, asked the way an agent asks."""
        msg = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "brief", "arguments": {"cwd": str(repo)}}}
        out = self.call("POST", "/mcp", msg)["result"]
        return -1 if out.get("isError") else len(out["content"][0]["text"])

    def connect_muse(self):
        return self.call("POST", "/api/connect", {"agent": "muse", "apply": True})

    def wait_for_new_memory(self, before, limit=240):
        deadline = time.time() + limit
        while time.time() < deadline:
            if self.stats().get("memories", 0) > before:
                return True
            time.sleep(5)
        return False

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()
        # The trial's settings may hold the model key; it never outlives the trial.
        self.settings.unlink(missing_ok=True)


# ---------- one Muse session ----------

def muse(prompt, repo, home, out, args, key):
    prompt_file = out.with_suffix(".prompt.txt")
    prompt_file.write_text(prompt)
    cmd = [str(MUSE), "exec", "--json", "--prompt-file", str(prompt_file), "--workspace", str(repo),
           "--trust-workspace", "--disable-approval", "--disable-web-tools", "--disable-request-user-input"]
    if args.live:
        cmd += ["--provider", "meta", "--api-key-stdin"]
        if args.model:
            cmd += ["--model", args.model]
        if args.effort:
            cmd += ["--reasoning-effort", args.effort]
    else:
        cmd += ["--provider", "echo"]
    env = {"HOME": str(home), "PATH": os.environ["PATH"]}
    start = time.time()
    with open(out, "w") as stdout, open(out.with_suffix(".stderr.txt"), "w") as stderr:
        try:
            proc = subprocess.run(cmd, cwd=repo, env=env, input=(key or "") if args.live else None, text=True,
                                  stdout=stdout, stderr=stderr, timeout=args.timeout)
            code = proc.returncode
        except subprocess.TimeoutExpired:
            code = "timeout"
    return parse_session(out, time.time() - start, code)


def parse_session(path, seconds, code):
    """Usage, final reply and tool use from Muse's JSONL events."""
    usage = {"input": 0, "cached": 0, "output": 0, "reasoning": 0, "model_calls": 0, "models": []}
    kinds, final, terminal = {}, "", None
    for line in path.read_text().splitlines():
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        p, kind = rec.get("payload", {}), rec.get("payload_type", "")
        if kind == "run.usage":
            usage = {"input": 0, "cached": 0, "output": 0, "reasoning": 0, "model_calls": p.get("calls", 0), "models": []}
            for m in p.get("models", []):
                for k, field in (("input", "input_tokens"), ("cached", "cached_tokens"), ("output", "output_tokens"), ("reasoning", "reasoning_tokens")):
                    usage[k] += m.get(field) or 0
                usage["models"].append(m.get("model"))
        elif kind.startswith("run.terminal"):
            final, terminal = p.get("text") or "", p.get("terminal")
        elif kind == "task.lifecycle.proposed":
            task_kind = p.get("event", {}).get("task_kind", "?")
            kinds[task_kind] = kinds.get(task_kind, 0) + 1
    tools = {k: n for k, n in kinds.items() if not k.startswith(("model.", "reminder."))}
    return {
        "seconds": round(seconds, 1), "exit": code, "terminal": terminal, "final": final, "usage": usage,
        "tool_calls": sum(tools.values()),
        "memory_calls": sum(n for k, n in tools.items() if "memoryos" in k or "mcp" in k),
        "task_kinds": kinds,
    }


def mcp_status(home):
    """What Muse's own startup log says about the memoryos server: ready, failed, or absent."""
    logs = sorted((home / ".local/share/muse/local-tracing/bootstrap").glob("*.log"))
    seen = "absent"
    for log in logs:
        for line in log.read_text(errors="replace").splitlines():
            if 'event="mcp.handshake"' in line and 'server="memoryos"' in line:
                m = re.search(r'outcome="([^"]+)"', line)
                seen = m.group(1) if m else "unknown"
    return seen


def hook_brief(home):
    """Sessions whose log carries the brief the connector's SessionStart hook injected."""
    logs = (home / ".local/share/muse/sessions").glob("**/session.jsonl")
    return sum("Shared project memory from MemoryOS" in log.read_text(errors="replace") for log in logs)


# ---------- grading ----------

def matches(groups, text):
    return all(any(re.search(p, text, re.I | re.M) for p in group) for group in groups)


def grade(task, final, diff):
    added = "\n".join(l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++"))
    contradicted = [p for p in task.get("never", []) if re.search(p, final + "\n" + added, re.I | re.M)]
    ok = matches(task.get("answer", []), final) and matches(task.get("diff", []), added) and not contradicted
    return {"pass": ok, "contradicted": contradicted}


# ---------- a trial ----------

def trial(task, projects, condition, n, root, args, key):
    project = projects[task["project"]]
    dirname = f"{task['id']}-{condition}-{n}"
    work = root / dirname
    work.mkdir(parents=True)
    home, repo = work / "home", work / task["project"]
    (home / ".config/muse").mkdir(parents=True)
    (home / ".config/muse/settings.json").write_text(json.dumps({"schema_version": 1}))
    base = clone(project, repo)
    daemon = None
    row = {"task": task["id"], "kind": task["kind"], "project": task["project"], "condition": condition, "run": n, "dir": str(work)}
    try:
        if condition == "with":
            handoff = task["kind"] == "handoff"
            # The default data folder under the trial HOME: Muse runs hooks with HOME but not ENGRAM_HOME,
            # so this is how the connector's SessionStart hook finds the trial daemon.
            daemon = Daemon(project, repo, home, home / "Library/Application Support/Engram", with_model=handoff and args.live)
            daemon.settle()
            row["hidden_docs"] = daemon.hide(project.get("hide", []))
            row["brief_chars"] = daemon.brief_size(repo)
            daemon.connect_muse()
        sessions = task.get("sessions") or [task["prompt"]]
        row["sessions"] = []
        for i, prompt in enumerate(sessions):
            if i > 0 and daemon:
                # MemoryOS captures session 1 from Muse's log; session 2 should find it.
                row["captured"] = daemon.wait_for_new_memory(before_memories, limit=args.capture_wait if args.live else 5)
            before_memories = daemon.stats().get("memories", 0) if daemon else 0
            row["sessions"].append(muse(prompt, repo, home, work / f"session{i + 1}.jsonl", args, key))
        row["mcp"] = mcp_status(home)
        row["hook_brief"] = hook_brief(home)
        diff = changes(repo, base)
        (work / "changes.diff").write_text(diff)
        row.update(grade(task, row["sessions"][-1]["final"], diff))
    except Exception as err:  # a broken trial is recorded, not fatal to the whole run
        row["error"] = f"{type(err).__name__}: {err}"
        row["pass"] = False
    finally:
        if daemon:
            daemon.stop()
        if not args.keep:
            shutil.rmtree(repo, ignore_errors=True)
            shutil.rmtree(home / ".local/share/muse/sessions", ignore_errors=True)
    total = lambda s: s["usage"]["input"] + s["usage"]["output"]
    s = row.get("sessions", [])
    row["tokens"] = sum(total(x) for x in s)
    row["input_tokens"] = sum(x["usage"]["input"] for x in s)
    # What drives the bill: input the provider could not serve from its cache.
    row["uncached_tokens"] = sum(x["usage"]["input"] - x["usage"]["cached"] for x in s)
    row["tool_calls"] = sum(x["tool_calls"] for x in s)
    row["seconds"] = round(sum(x["seconds"] for x in s), 1)
    return row


def summarize(rows):
    med = lambda xs: round(statistics.median(xs)) if xs else None
    out = {}
    for key in sorted({(r["task"], r["condition"]) for r in rows}):
        rs = [r for r in rows if (r["task"], r["condition"]) == key]
        out[f"{key[0]} / {key[1]}"] = {
            "kind": rs[0]["kind"], "runs": len(rs), "passed": sum(r["pass"] for r in rs),
            "median_tokens": med([r["tokens"] for r in rs]), "median_input_tokens": med([r["input_tokens"] for r in rs]),
            "median_uncached_tokens": med([r["uncached_tokens"] for r in rs]),
            "median_tool_calls": med([r["tool_calls"] for r in rs]), "median_seconds": med([r["seconds"] for r in rs]),
            "contradicted": sum(bool(r.get("contradicted")) for r in rs), "errors": sum("error" in r for r in rs),
        }
    for cond in ("with", "without"):
        rs = [r for r in rows if r["condition"] == cond]
        if rs:
            out[f"ALL / {cond}"] = {"runs": len(rs), "passed": sum(r["pass"] for r in rs),
                                    "median_tokens": med([r["tokens"] for r in rs]),
                                    "median_uncached_tokens": med([r["uncached_tokens"] for r in rs]),
                                    "median_tool_calls": med([r["tool_calls"] for r in rs]),
                                    "contradicted": sum(bool(r.get("contradicted")) for r in rs)}
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--live", action="store_true", help="call Muse Spark for real (costs money)")
    ap.add_argument("--yes", action="store_true", help="with --live: do not ask before starting")
    ap.add_argument("--tasks", help="comma-separated task ids (default: all)")
    ap.add_argument("--conditions", default="with,without")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--model", help="Muse model id (default: Muse's own default)")
    ap.add_argument("--effort", help="Muse reasoning effort (default: Muse's own default)")
    ap.add_argument("--timeout", type=int, default=900, help="seconds per session")
    ap.add_argument("--capture-wait", type=int, default=240, help="handoff: seconds to wait for MemoryOS to capture session 1")
    ap.add_argument("--keep", action="store_true", help="keep each trial's clone")
    ap.add_argument("--work", default=os.path.join(tempfile.gettempdir(), "memoryos-bench"))
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()

    projects, tasks = load_tasks(args.tasks)
    conditions = args.conditions.split(",")
    for path in (MUSE, DAEMON):
        if not path.exists():
            sys.exit(f"missing {path}" + (" (run: cargo build -p engram-daemon)" if path == DAEMON else ""))
    stamp = time.strftime("%Y%m%d-%H%M%S")
    root = Path(os.path.realpath(args.work)) / stamp
    watched = [os.path.realpath(p) for p in json.loads((APP / "projects.json").read_text())]
    if any(str(root).startswith(w + os.sep) for w in watched):
        sys.exit(f"{root} is inside a watched project; pick another --work folder")

    sessions = sum(len(t.get("sessions") or [1]) for t in tasks) * len(conditions) * args.runs
    print(f"{len(tasks)} tasks x {len(conditions)} conditions x {args.runs} runs = {sessions} Muse sessions"
          f" ({'LIVE, Muse Spark' if args.live else 'dry run, echo provider, no model calls'})")
    key = None
    if args.live:
        key = muse_key()
        if not key:
            sys.exit("no Meta key: set MUSE_API_KEY, or choose Meta in the MemoryOS Settings")
        if not args.yes and input("Start? This spends Meta API credits. [y/N] ").strip().lower() != "y":
            sys.exit("not started")

    rng = random.Random(args.seed)
    rows = []
    for n in range(1, args.runs + 1):
        for task in tasks:
            order = conditions[:]
            rng.shuffle(order)  # neither condition always goes first
            for cond in order:
                row = trial(task, projects, cond, n, root, args, key)
                rows.append(row)
                status = "error: " + row["error"] if "error" in row else ("pass" if row["pass"] else "fail")
                print(f"{task['id']:<20} {cond:<8} run {n}  {status:<6} tokens {row['tokens']:>7} (uncached {row['uncached_tokens']:>6})  tools {row['tool_calls']:>3}"
                      f"  {row['seconds']:>6}s  mcp {row.get('mcp', '-')}  brief {row.get('hook_brief', '-')}"
                      f"  memory calls {sum(x['memory_calls'] for x in row.get('sessions', []))}")

    results = {"live": args.live, "started": stamp, "model": args.model, "effort": args.effort, "work": str(root),
               "summary": summarize(rows), "trials": rows}
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    out = out_dir / f"{'live' if args.live else 'dry'}-{stamp}.json"
    out.write_text(json.dumps(results, indent=2))
    print(json.dumps(results["summary"], indent=2))
    print(f"\nwrote {out}")


if __name__ == "__main__":
    main()
