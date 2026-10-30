import { useEffect, useRef } from "react";
import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import type { Graph } from "../types.ts";

const COLORS: Record<string, string> = {
  decision: "#4f8cff",
  rejected_option: "#e5584f",
  dead_end: "#e5984f",
  fact: "#3fb68b",
  preference: "#a96bff",
  episode: "#8b93a1",
  task: "#e5c84f",
  module: "#2b3445",
  file: "#4a5568",
};

const toElements = (g: Graph): ElementDefinition[] => [
  ...g.nodes.map((n) => ({ data: { id: n.id, label: n.label, kind: n.kind, status: n.status ?? "" } })),
  ...g.edges.map((e) => ({ data: { id: e.id, source: e.source, target: e.target, label: e.type } })),
];

const STYLE: cytoscape.StylesheetJson = [
  {
    selector: "node",
    style: {
      label: "data(label)",
      "font-size": 10,
      color: "#cfd6e4",
      "text-valign": "bottom",
      "text-margin-y": 4,
      "text-wrap": "ellipsis",
      "text-max-width": "110px",
      width: 22,
      height: 22,
      "background-color": (n: cytoscape.NodeSingular) => COLORS[n.data("kind")] ?? "#556",
    },
  },
  { selector: 'node[kind = "module"]', style: { shape: "round-rectangle", width: 34, height: 26 } },
  { selector: 'node[kind = "file"]', style: { width: 12, height: 12 } },
  { selector: 'node[status = "superseded"]', style: { opacity: 0.4, "border-style": "dashed", "border-width": 2, "border-color": "#8b93a1" } },
  { selector: "edge", style: { width: 1, "line-color": "#3a4357", "curve-style": "bezier", "target-arrow-shape": "triangle", "target-arrow-color": "#3a4357", "arrow-scale": 0.6 } },
  { selector: 'edge[label = "supersedes"]', style: { "line-style": "dashed", "line-color": "#8b93a1" } },
  { selector: ":selected", style: { "border-width": 3, "border-color": "#ffffff" } },
];

interface Props {
  graph: Graph;
  onSelect(id: string | null): void;
}

export function GraphView({ graph, onSelect }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | undefined>(undefined);
  const shape = useRef("");

  useEffect(() => {
    const core = cytoscape({ container: host.current, style: STYLE, wheelSensitivity: 0.3 });
    core.on("tap", "node", (e) => onSelect(e.target.id()));
    core.on("tap", (e) => e.target === core && onSelect(null));
    cy.current = core;
    return () => core.destroy();
  }, [onSelect]);

  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    const signature = graph.nodes.map((n) => n.id).sort().join() + graph.edges.length;
    core.json({ elements: toElements(graph) });
    if (signature !== shape.current) {
      shape.current = signature;
      core.resize();
      core.layout({ name: "cose", animate: false, fit: true, padding: 40, nodeRepulsion: () => 9000, idealEdgeLength: () => 70 }).run();
    }
  }, [graph]);

  return <div ref={host} className="graph" />;
}

export function Legend() {
  return (
    <div className="legend">
      {Object.entries(COLORS).map(([kind, color]) => (
        <span key={kind}>
          <i style={{ background: color }} />
          {kind.replace("_", " ")}
        </span>
      ))}
    </div>
  );
}
