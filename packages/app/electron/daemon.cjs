const { spawn } = require("node:child_process");
const { existsSync } = require("node:fs");
const path = require("node:path");

const CLI = path.resolve(__dirname, "../../daemon/src/cli.ts");
const NODE_CANDIDATES = [process.env.ENGRAM_NODE, "/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"];

/** GUI apps on macOS do not inherit the shell PATH, so look for Node in the usual places. */
function findNode() {
  const found = NODE_CANDIDATES.find((p) => p && existsSync(p));
  if (!found) throw new Error("Node.js not found. Install Node 24+ or set ENGRAM_NODE.");
  return found;
}

const running = new Map();

/** Start the daemon for a folder (once) and resolve with its url and UI token. */
function startDaemon(root) {
  if (running.has(root)) return running.get(root).ready;
  const child = spawn(findNode(), [CLI, "start", root], { stdio: ["ignore", "pipe", "pipe"] });
  const ready = new Promise((resolve, reject) => {
    let buffer = "";
    child.stdout.on("data", (chunk) => {
      buffer += chunk;
      const line = buffer.split("\n").find((l) => l.startsWith("ENGRAM_READY "));
      if (line) resolve(JSON.parse(line.slice("ENGRAM_READY ".length)));
    });
    child.stderr.on("data", (chunk) => (buffer += chunk));
    child.on("exit", (code) => reject(new Error(`daemon exited (${code}): ${buffer.slice(-400)}`)));
  });
  running.set(root, { child, ready });
  return ready;
}

function stopAll() {
  for (const { child } of running.values()) child.kill("SIGTERM");
  running.clear();
}

module.exports = { startDaemon, stopAll };
