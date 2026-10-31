import type { SimulationLinkDatum, SimulationNodeDatum } from "d3-force";
import type { Graph } from "../../api/types.ts";
import type { GraphSettings } from "./settings.ts";

export interface SimNode extends SimulationNodeDatum {
  id: string;
  label: string;
  kind: string;
  status: string;
  degree: number;
  radius: number;
  color: string;
  /** Third axis, used only while the graph is rotatable. */
  z?: number;
  vz?: number;
  fz?: number | null;
  /** Zoomable map: children it can open into, open or not, its part, who wrote most of it. */
  children: number;
  expanded: boolean;
  area?: string;
  owner?: string;
  /** API bodies: the part that serves them; groups and endpoints: their API body. */
  serves?: string;
  hub?: string;
  /** Screen-plane position after rotation and perspective (see projection.ts). */
  sx?: number;
  sy?: number;
  depth?: number;
  scale?: number;
  fade?: number;
}

export interface SimLink extends SimulationLinkDatum<SimNode> {
  id: string;
  type: string;
  /** How many fine-grained edges it stands for (zoomable map), 1 otherwise. */
  count: number;
  source: SimNode;
  target: SimNode;
}

const cssColor = (kind: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(`--t-${kind}`).trim() || "#6b7489";

/** Undirected adjacency lists, used for orphans, local graphs and hover highlighting. */
export function adjacency(graph: Graph): Map<string, Set<string>> {
  const adj = new Map<string, Set<string>>(graph.nodes.map((n) => [n.id, new Set<string>()]));
  for (const e of graph.edges) {
    adj.get(e.source)?.add(e.target);
    adj.get(e.target)?.add(e.source);
  }
  return adj;
}

/** Breadth-first neighbourhood of `root` up to `depth` hops. */
export function neighbourhood(adj: Map<string, Set<string>>, root: string, depth: number): Set<string> {
  const seen = new Set([root]);
  let frontier = [root];
  for (let d = 0; d < depth; d++) {
    const next: string[] = [];
    for (const id of frontier) for (const n of adj.get(id) ?? []) if (!seen.has(n)) { seen.add(n); next.push(n); }
    frontier = next;
  }
  return seen;
}

type Position = Pick<SimNode, "x" | "y" | "z" | "vx" | "vy" | "vz">;

/**
 * Apply filters and build simulation objects. Positions of nodes that survive a refresh are kept,
 * and new nodes start next to an already placed neighbour, so the layout never jumps.
 */
export function buildScene(graph: Graph, s: GraphSettings, selected: string | null, previous: Map<string, Position>) {
  const adj = adjacency(graph);
  const query = s.search.trim().toLowerCase();
  const local = s.local && selected && adj.has(selected) ? neighbourhood(adj, selected, s.localDepth) : null;

  let keep = graph.nodes.filter((n) =>
    !s.hidden.includes(n.kind) &&
    (!query || n.label.toLowerCase().includes(query)) &&
    (!local || local.has(n.id)));
  const kept = new Set(keep.map((n) => n.id));

  // Containment: sun (part) → planets (files) → moons (functions, classes) → their methods.
  const parents = new Map(graph.edges.filter((e) => e.type === "contains").map((e) => [e.target, e.source]));
  // A hidden node hands its links to its nearest visible parent, so hiding functions turns
  // function → endpoint calls into file → endpoint calls instead of cutting the network apart.
  const lift = (id: string): string | undefined => {
    for (let cur: string | undefined = id; cur; cur = parents.get(cur)) if (kept.has(cur)) return cur;
    return undefined;
  };
  const merged = new Map<string, { id: string; source: string; target: string; type: string; count: number }>();
  for (const e of graph.edges) {
    const [a, b] = e.type === "contains" ? [lift(e.source), kept.has(e.target) ? e.target : undefined] : [lift(e.source), lift(e.target)];
    if (!a || !b || a === b) continue;
    const id = `${a}>${b}:${e.type}`;
    const found = merged.get(id);
    if (found) found.count += e.count ?? 1;
    else merged.set(id, { id, source: a, target: b, type: e.type, count: e.count ?? 1 });
  }
  const edges = [...merged.values()];
  const degree = new Map<string, number>();
  for (const e of edges) {
    degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
    degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
  }
  if (!s.showOrphans) keep = keep.filter((n) => (degree.get(n.id) ?? 0) > 0 || n.id === selected);

  // Everything a node holds (a part: its files and functions), so suns outweigh their planets.
  const inside = new Map<string, number>();
  for (const child of parents.keys()) {
    if (!kept.has(child)) continue;
    for (let p = parents.get(child); p; p = parents.get(p)) inside.set(p, (inside.get(p) ?? 0) + 1);
  }
  const nodes: SimNode[] = keep.map((n) => {
    const d = degree.get(n.id) ?? 0;
    const prior = previous.get(n.id) ?? placeNear(n.id, adj, previous, parents.get(n.id));
    const children = n.children ?? 0;
    const color = s.colorBy === "author" ? authorColor(n.owner?.who) : cssColor(n.kind);
    return {
      id: n.id, label: n.label, kind: n.kind, status: n.status ?? "", degree: d, radius: radiusFor(d + (inside.get(n.id) ?? 0)), color,
      children, expanded: n.expanded ?? false, area: n.area, owner: n.owner?.who, serves: n.serves, hub: n.hub, ...prior,
    };
  });
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const links: SimLink[] = edges
    .filter((e) => byId.has(e.source) && byId.has(e.target))
    .map((e) => ({ id: e.id, type: e.type, count: e.count, source: byId.get(e.source)!, target: byId.get(e.target)! }));
  return { nodes, links, adj };
}

function placeNear(id: string, adj: Map<string, Set<string>>, previous: Map<string, Position>, parent?: string): Position {
  for (const n of [...(parent ? [parent] : []), ...(adj.get(id) ?? [])]) {
    const p = previous.get(n);
    if (p?.x !== undefined && p.y !== undefined) {
      const jitter = () => (Math.random() - 0.5) * 30;
      return { x: p.x + jitter(), y: p.y + jitter(), ...(p.z !== undefined && { z: p.z + jitter() }) };
    }
  }
  return {};
}

/** Agents keep their colours everywhere; people get stable colours from a hash of their name. */
const PEOPLE = ["#e8cf5c", "#b184ff", "#3fb6c9", "#ef6a62", "#f0a35e", "#9fb4ff"];
export function authorColor(who?: string): string {
  if (!who) return "#3a4252";
  const agent = { "Claude Code": "--a-claude-code", Codex: "--a-codex", "Muse Code": "--a-muse-code" }[who];
  if (agent) return getComputedStyle(document.documentElement).getPropertyValue(agent).trim();
  if (who === "uncommitted") return "#5b6478";
  let h = 0;
  for (const c of who) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PEOPLE[h % PEOPLE.length]!;
}

/** Obsidian-style: a node grows with its links and what it holds, so parts become the hubs. */
function radiusFor(weight: number): number {
  return Math.min(36, 2.5 + Math.sqrt(weight) * 1.3);
}
