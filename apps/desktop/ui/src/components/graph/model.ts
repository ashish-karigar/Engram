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
  /** The top-level folder this node sits in (`frontend`, `backend`); itself for that folder. */
  part?: string;
  /** Where its cluster sits across the screen: -1 left (calls the API), 1 right (serves it), 0 middle. */
  side?: number;
  /** The API node: for each calling file, the files that serve what it calls. */
  routes?: Record<string, string[]>;
  /** Third axis, used only while the graph is rotatable. */
  z?: number;
  vz?: number;
  fz?: number | null;
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
  source: SimNode;
  target: SimNode;
}

/** Each kind keeps only a faint tint of its colour over a quiet grey, so the graph stays calm. */
const cssColor = (kind: string) => {
  const hex = getComputedStyle(document.documentElement).getPropertyValue(`--t-${kind}`).trim() || "#8a909b";
  const c = parseInt(hex.slice(1, 7), 16);
  const mix = (v: number, grey: number) => Math.round(v * 0.3 + grey * 0.7);
  return `rgb(${mix(c >> 16, 150)}, ${mix((c >> 8) & 255, 155)}, ${mix(c & 255, 165)})`;
};

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

  const inside = descendants(graph);
  const parts = topFolders(graph);
  const sides = partSides(graph, parts);
  const nodes: SimNode[] = keep.map((n) => {
    const d = degree.get(n.id) ?? 0;
    const prior = previous.get(n.id) ?? placeNear(n.id, adj, previous);
    const part = parts.get(n.id);
    return {
      id: n.id, label: n.label, kind: n.kind, status: n.status ?? "", degree: d, part,
      side: n.kind === "api" ? 0 : n.kind === "service" ? serviceSide(graph, n.id, parts, sides) : part !== undefined ? sides.get(part) : undefined,
      routes: n.routes,
      radius: radiusFor(n.kind, d, inside.get(n.id) ?? n.children ?? 0, n.label.split("/").length - 1), color: cssColor(n.kind), ...prior,
    };
  });
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const links: SimLink[] = edges
    .filter((e) => byId.has(e.source) && byId.has(e.target))
    .map((e) => ({ id: e.id, type: e.type, source: byId.get(e.source)!, target: byId.get(e.target)! }));
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

export interface Trace {
  /** Every node on the path, the start included. */
  nodes: Set<string>;
  /** Each highlighted line and the step it was reached at (0 = touching the start). */
  links: Map<string, number>;
  /** How many steps from the start each node is (the start is 0, its neighbours 1). */
  depth: Map<string, number>;
}

/**
 * Follow the flow out of a node instead of stopping at its neighbours: down into what a folder
 * contains, along the imports of its files, out through the API calls they make, and across the
 * API to the files that serve them, where it stops. A top-level part (`frontend`) stops at the
 * API, its boundary. The start's own neighbours are always shown, like a hover.
 * The result is a tree: each node lights up along the one line that first reached it, so a web
 * of imports shows as clean branches rather than every crossing line at once.
 */
export function trace(nodes: SimNode[], links: SimLink[], start: string): Trace {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const from = new Map<string, SimLink[]>();
  for (const l of links) for (const id of [l.source.id, l.target.id]) from.set(id, [...(from.get(id) ?? []), l]);
  const origin = byId.get(start);
  const result: Trace = { nodes: new Set([start]), links: new Map(), depth: new Map([[start, 0]]) };
  if (!origin) return result;
  const crosses = origin.part !== origin.id;
  // Flow stays on its own side of the API, except for API calls and the external services it uses.
  const forward = (u: SimNode, l: SimLink) =>
    l.source.id === u.id && (l.type === "contains" || l.type === "imports" || l.type === "calls" || l.type === "uses") &&
    (l.type === "calls" || l.type === "uses" || l.target.part === undefined || l.target.part === u.part);
  // A service has no side of its own: its trace starts from every file that uses it, and from
  // there only follows imports into the API clients, so it ends at the backend it reaches.
  const fromService = origin.kind === "service";
  const apiClients = new Set(links.filter((l) => l.type === "calls" && l.target.kind === "api").map((l) => l.source.id));

  let frontier = [origin];
  let api: { node: SimNode; step: number } | null = null;
  for (let step = 0; frontier.length && step < 40; step++) {
    const next: SimNode[] = [];
    for (const u of frontier) {
      for (const l of from.get(u.id) ?? []) {
        const v = l.source.id === u.id ? l.target : l.source;
        const flows = forward(u, l) && !(fromService && l.type !== "calls" && !apiClients.has(v.id));
        if ((!flows && step > 0) || result.nodes.has(v.id)) continue;
        result.links.set(l.id, step);
        result.nodes.add(v.id);
        result.depth.set(v.id, step + 1);
        if (v.kind === "api") api ??= { node: v, step: step + 1 };
        else if (v.kind !== "service" && (flows || (step === 0 && fromService))) next.push(v);
      }
    }
    frontier = next;
  }

  // Across the API, only to the files that serve what the traced callers call.
  if (api && crosses && origin.kind !== "api") {
    const reached = new Set<string>();
    for (const caller of result.nodes) for (const s of api.node.routes?.[caller] ?? []) reached.add(s);
    for (const l of from.get(api.node.id) ?? []) {
      const v = l.source.id === api.node.id ? l.target : l.source;
      if (l.type !== "serves" || !reached.has(v.id) || result.nodes.has(v.id)) continue;
      result.links.set(l.id, api.step);
      result.nodes.add(v.id);
      result.depth.set(v.id, api.step + 1);
    }
  }
  return result;
}

/** The top-level folder above each node, following `contains` edges up; only folders that hold something. */
function topFolders(graph: Graph): Map<string, string> {
  const parent = new Map<string, string>();
  for (const e of graph.edges) if (e.type === "contains") parent.set(e.target, e.source);
  const top = new Map<string, string>();
  for (const id of new Set([...parent.keys(), ...parent.values()])) {
    let root = id;
    for (let hops = 0; parent.has(root) && hops < 50; hops++) root = parent.get(root)!;
    top.set(id, root);
  }
  return top;
}

/**
 * Which side of the screen each top-level folder belongs on: the code that calls the API on the
 * left, the code that serves it on the right. A folder that does both, or neither, gets no side.
 */
function partSides(graph: Graph, parts: Map<string, string>): Map<string, number> {
  const calls = new Set<string>(), serves = new Set<string>();
  for (const e of graph.edges) {
    const part = parts.get(e.source);
    if (!part || !e.target.startsWith("api:")) continue;
    if (e.type === "calls") calls.add(part);
    if (e.type === "serves") serves.add(part);
  }
  const sides = new Map<string, number>();
  for (const p of calls) if (!serves.has(p)) sides.set(p, -1);
  for (const p of serves) if (!calls.has(p)) sides.set(p, 1);
  return sides;
}

/**
 * Where an external service sits: used from both sides (Auth0), in the middle with the API; used
 * from one side only (an LLM server the backend calls), on that side's outer edge.
 */
function serviceSide(graph: Graph, id: string, parts: Map<string, string>, sides: Map<string, number>): number | undefined {
  const used = new Set<number>();
  for (const e of graph.edges) {
    if (e.target !== id) continue;
    const side = sides.get(parts.get(e.source) ?? "");
    if (side !== undefined) used.add(side);
  }
  if (used.size === 0) return undefined;
  return used.size > 1 ? 0 : [...used][0]! * 1.5;
}

/** How many files and folders sit anywhere inside each folder, following `contains` edges. */
function descendants(graph: Graph): Map<string, number> {
  const children = new Map<string, string[]>();
  for (const e of graph.edges) if (e.type === "contains") children.set(e.source, [...(children.get(e.source) ?? []), e.target]);
  const count = new Map<string, number>();
  const walk = (id: string, seen: Set<string>): number => {
    if (count.has(id)) return count.get(id)!;
    let total = 0;
    for (const c of children.get(id) ?? []) if (!seen.has(c)) { seen.add(c); total += 1 + walk(c, seen); }
    count.set(id, total);
    return total;
  };
  for (const id of children.keys()) walk(id, new Set([id]));
  return count;
}

/**
 * Memories stay prominent. A folder grows with everything inside it and shrinks with each level
 * down, so top-level parts like `frontend` and `backend` clearly dominate their subfolders.
 */
function radiusFor(kind: string, degree: number, inside: number, depth: number): number {
  if (kind === "file") return 2.5 + Math.sqrt(degree) * 0.9;
  if (kind === "module") return Math.max(4, (4 + Math.sqrt(inside) * 1.3) * (depth === 0 ? 1.3 : 0.75 ** depth));
  if (kind === "api") return 6 + Math.sqrt(inside) * 1.6;
  if (kind === "service") return 5 + Math.sqrt(degree) * 1.3;
  return 5 + Math.sqrt(degree) * 1.8;
}
