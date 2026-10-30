import { useEffect, useRef } from "react";
import cytoscape, { type Core, type ElementDefinition, type StylesheetJson } from "cytoscape";
import type { Graph } from "../api/types.ts";

const color = (kind: string) => getComputedStyle(document.documentElement).getPropertyValue(`--t-${kind}`).trim() || "#566178";

const toElements = (g: Graph): ElementDefinition[] => [
  ...g.nodes.map((n) => ({ data: { id: n.id, label: n.label, kind: n.kind, status: n.status ?? "", color: color(n.kind) } })),
  ...g.edges.map((e) => ({ data: { id: e.id, source: e.source, target: e.target, label: e.type } })),
];

const STYLE: StylesheetJson = [
  {
    selector: "node",
    style: {
      label: "data(label)", "font-size": 10, color: "#aab3c5", "text-valign": "bottom", "text-margin-y": 5,
      "text-wrap": "ellipsis", "text-max-width": "120px", "min-zoomed-font-size": 8,
      width: 18, height: 18, "background-color": "data(color)", "border-width": 0,
      "overlay-opacity": 0, "transition-property": "opacity, width, height", "transition-duration": 150,
    } as never,
  },
  { selector: 'node[kind = "module"]', style: { shape: "round-rectangle", width: 38, height: 26, "font-size": 11, color: "#cdd5e6", "background-opacity": 0.9, "border-width": 1, "border-color": "#3b4660" } },
  { selector: 'node[kind = "file"]', style: { width: 9, height: 9, "font-size": 8, label: "" } },
  { selector: 'node[status = "superseded"]', style: { opacity: 0.45, "border-width": 2, "border-style": "dashed", "border-color": "#8a93a6" } },
  { selector: "edge", style: { width: 1, "line-color": "#2a3347", "curve-style": "bezier", "target-arrow-shape": "triangle", "target-arrow-color": "#2a3347", "arrow-scale": 0.6, opacity: 0.8 } },
  { selector: 'edge[label = "supersedes"]', style: { "line-style": "dashed", "line-color": "#6b7388" } },
  { selector: ".faded", style: { opacity: 0.12 } },
  { selector: ".hot", style: { opacity: 1, label: "data(label)", "z-index": 10 } },
  { selector: "edge.hot", style: { "line-color": "#6d8dff", "target-arrow-color": "#6d8dff", width: 1.6, "font-size": 9, color: "#8a93a6" } },
  { selector: ":selected", style: { "border-width": 3, "border-color": "#ffffff", label: "data(label)" } },
];

interface Props {
  graph: Graph;
  selectedId: string | null;
  onSelect(id: string | null): void;
}

export function GraphView({ graph, selectedId, onSelect }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | undefined>(undefined);
  const shape = useRef("");

  useEffect(() => {
    const core = cytoscape({ container: host.current, style: STYLE, wheelSensitivity: 0.25, minZoom: 0.2, maxZoom: 3 });
    core.on("tap", "node", (e) => onSelect(e.target.id()));
    core.on("tap", (e) => e.target === core && onSelect(null));
    core.on("mouseover", "node", (e) => e.target.addClass("hot"));
    core.on("mouseout", "node", (e) => e.target.removeClass("hot"));
    const observer = new ResizeObserver(() => { core.resize(); core.fit(undefined, 70); });
    if (host.current) observer.observe(host.current);
    cy.current = core;
    return () => { observer.disconnect(); core.destroy(); };
  }, [onSelect]);

  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    const signature = graph.nodes.map((n) => n.id).sort().join() + graph.edges.length;
    core.json({ elements: toElements(graph) });
    if (signature !== shape.current) {
      shape.current = signature;
      core.resize();
      core.layout({ name: "cose", animate: false, nodeRepulsion: () => 12000, idealEdgeLength: () => 80 }).run();
      core.fit(undefined, 70);
    }
  }, [graph]);

  useEffect(() => {
    const core = cy.current;
    if (!core) return;
    core.elements().removeClass("faded hot").unselect();
    const node = selectedId ? core.getElementById(selectedId) : null;
    if (!node || node.empty()) return;
    const around = node.closedNeighborhood();
    core.elements().not(around).addClass("faded");
    around.addClass("hot");
    node.select();
    core.animate({ center: { eles: node }, duration: 250 });
  }, [selectedId, graph]);

  return <div ref={host} className="graph" />;
}

const KINDS = ["decision", "rejected_option", "dead_end", "fact", "preference", "module", "file"];

export function Legend() {
  return (
    <div className="legend">
      {KINDS.map((k) => (
        <span key={k}><i style={{ background: color(k) }} />{k.replace("_", " ")}</span>
      ))}
    </div>
  );
}
