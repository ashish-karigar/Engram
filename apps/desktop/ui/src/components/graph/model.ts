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
  const edges = graph.edges.filter((e) => kept.has(e.source) && kept.has(e.target));
  const degree = new Map<string, number>();
  for (const e of edges) {
    degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
    degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
  }
  if (!s.showOrphans) keep = keep.filter((n) => (degree.get(n.id) ?? 0) > 0 || n.id === selected);

  const nodes: SimNode[] = keep.map((n) => {
    const d = degree.get(n.id) ?? 0;
    const prior = previous.get(n.id) ?? placeNear(n.id, adj, previous);
    const children = n.children ?? 0;
    const color = s.colorBy === "author" ? authorColor(n.owner?.who) : cssColor(n.kind);
    return {
      id: n.id, label: n.label, kind: n.kind, status: n.status ?? "", degree: d, radius: radiusFor(n.kind, d, children), color,
      children, expanded: n.expanded ?? false, area: n.area, owner: n.owner?.who, ...prior,
    };
  });
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const links: SimLink[] = edges
    .filter((e) => byId.has(e.source) && byId.has(e.target))
    .map((e) => ({ id: e.id, type: e.type, count: e.count ?? 1, source: byId.get(e.source)!, target: byId.get(e.target)! }));
  return { nodes, links, adj };
}

function placeNear(id: string, adj: Map<string, Set<string>>, previous: Map<string, Position>): Position {
  for (const n of adj.get(id) ?? []) {
    const p = previous.get(n);
    if (p?.x !== undefined && p.y !== undefined) {
      const jitter = () => (Math.random() - 0.5) * 30;
      return { x: p.x + jitter(), y: p.y + jitter(), ...(p.z !== undefined && { z: p.z + jitter() }) };
    }
  }
  return {};
}

/** Memories are the point of the graph, so they stay prominent; code nodes stay small. */
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

function radiusFor(kind: string, degree: number, children = 0): number {
  if (kind === "area") return 14 + Math.sqrt(degree + children) * 2.2;
  if (kind === "dir") return 7 + Math.sqrt(children) * 1.6;
  if (kind === "class" || kind === "struct" || kind === "trait") return 5 + Math.sqrt(children) * 1.2;
  if (kind === "function" || kind === "method") return 3.2 + Math.sqrt(degree) * 0.5;
  if (kind === "service") return 7 + Math.sqrt(degree);
  if (kind === "endpoint") return 3.5 + Math.sqrt(degree) * 0.6;
  if (kind === "file") return children ? 4 + Math.sqrt(children) * 0.9 : 2.5 + Math.sqrt(degree) * 0.9;
  if (kind === "module") return 4.5 + Math.sqrt(degree) * 1.1;
  return 5 + Math.sqrt(degree) * 1.8;
}
