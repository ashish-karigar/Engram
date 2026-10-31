import type { SimulationLinkDatum, SimulationNodeDatum } from "d3-force";
import type { Graph } from "../../api/types.ts";

export interface Node extends SimulationNodeDatum {
  id: string;
  label: string;
  kind: string;
  superseded: boolean;
  degree: number;
  r: number;
}

export interface Link extends SimulationLinkDatum<Node> {
  source: Node;
  target: Node;
}

export interface Scene {
  nodes: Node[];
  links: Link[];
  neighbours: Map<string, Set<string>>;
}

const CODE = new Set(["file", "module", "function", "method", "class", "struct", "trait"]);

/** Three quiet greys: memories lightest, documents in between, code darkest. */
export function fill(n: Node): string {
  if (n.kind === "document") return "#8a909b";
  if (CODE.has(n.kind)) return "#5c626d";
  return "#b8bdc6";
}

/** Short label: a file or folder shows its name, not its path. */
export function label(n: Node): string {
  const path = CODE.has(n.kind) || (n.label.includes("/") && !n.label.includes(" "));
  const text = path ? n.label.replace(/\/$/, "").slice(n.label.replace(/\/$/, "").lastIndexOf("/") + 1) : n.label;
  return text.length > 42 ? `${text.slice(0, 40)}…` : text;
}

/**
 * Turn the server's graph into simulation objects. Nodes that survive a refresh keep their
 * positions, and new ones start next to a placed neighbour, so polling never reshuffles the view.
 */
export function buildScene(graph: Graph, query: string, orphans: boolean, previous: Map<string, Node>): Scene {
  const q = query.trim().toLowerCase();
  let kept = graph.nodes.filter((n) => !q || n.label.toLowerCase().includes(q));
  const ids = new Set(kept.map((n) => n.id));
  const edges = graph.edges.filter((e) => e.source !== e.target && ids.has(e.source) && ids.has(e.target));

  const neighbours = new Map<string, Set<string>>();
  for (const e of edges) {
    if (!neighbours.has(e.source)) neighbours.set(e.source, new Set());
    if (!neighbours.has(e.target)) neighbours.set(e.target, new Set());
    neighbours.get(e.source)!.add(e.target);
    neighbours.get(e.target)!.add(e.source);
  }
  if (!orphans) kept = kept.filter((n) => neighbours.has(n.id));

  const nodes: Node[] = kept.map((n) => {
    const degree = neighbours.get(n.id)?.size ?? 0;
    const old = previous.get(n.id);
    const near = old ? undefined : placeNear(n.id, neighbours, previous);
    return {
      id: n.id, label: n.label, kind: n.kind, superseded: n.status === "superseded", degree,
      r: Math.min(14, 3 + Math.sqrt(degree) * 1.5),
      x: old?.x ?? near?.x, y: old?.y ?? near?.y, vx: old?.vx, vy: old?.vy,
    };
  });
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const links: Link[] = [];
  const seen = new Set<string>();
  for (const e of edges) {
    const s = byId.get(e.source), t = byId.get(e.target);
    const key = e.source < e.target ? `${e.source}|${e.target}` : `${e.target}|${e.source}`;
    if (s && t && !seen.has(key)) { seen.add(key); links.push({ source: s, target: t }); }
  }
  return { nodes, links, neighbours };
}

function placeNear(id: string, neighbours: Map<string, Set<string>>, previous: Map<string, Node>) {
  for (const n of neighbours.get(id) ?? []) {
    const p = previous.get(n);
    if (p?.x !== undefined && p.y !== undefined) return { x: p.x + (Math.random() - 0.5) * 20, y: p.y + (Math.random() - 0.5) * 20 };
  }
  return undefined;
}
