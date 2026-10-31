import { useEffect, useMemo, useRef } from "react";
import ForceGraph3DFactory, { type ForceGraph3DInstance } from "3d-force-graph";
import type { Graph } from "../../api/types.ts";
import type { GraphCommands } from "./ForceGraph.tsx";
import { buildScene } from "./model.ts";
import type { GraphSettings } from "./settings.ts";

interface Props {
  graph: Graph;
  settings: GraphSettings;
  selectedId: string | null;
  onSelect(id: string | null): void;
  onZoom(k: number): void;
  commands: React.MutableRefObject<GraphCommands | null>;
}

interface Node3D {
  id: string;
  label: string;
  kind: string;
  status: string;
  degree: number;
  radius: number;
  color: string;
  x?: number;
  y?: number;
  z?: number;
}

interface Link3D {
  source: string | Node3D;
  target: string | Node3D;
  type: string;
}

const id = (end: string | Node3D) => (typeof end === "string" ? end : end.id);
const DIM = "rgba(120,130,150,0.18)";

/** 3D view of the same graph: orbit with the mouse, scroll to zoom, click a node to fly to it. */
export function ForceGraph3D({ graph, settings, selectedId, onSelect, onZoom, commands }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const fg = useRef<ForceGraph3DInstance | null>(null);
  const focus = useRef<{ id: string | null; around: Set<string> }>({ id: null, around: new Set() });
  const adjacency = useRef<Map<string, Set<string>>>(new Map());
  const live = useRef({ onSelect, onZoom });
  live.current = { onSelect, onZoom };

  const refreshColors = () => {
    const f = fg.current;
    if (!f) return;
    f.nodeColor(f.nodeColor()).linkColor(f.linkColor()).linkWidth(f.linkWidth());
  };

  const setFocus = (nodeId: string | null) => {
    focus.current = { id: nodeId, around: new Set(nodeId ? [nodeId, ...(adjacency.current.get(nodeId) ?? [])] : []) };
    refreshColors();
  };

  useEffect(() => {
    const el = host.current!;
    const f = new ForceGraph3DFactory(el, { controlType: "orbit", rendererConfig: { alpha: true, antialias: true } })
      .backgroundColor("rgba(0,0,0,0)")
      .showNavInfo(false)
      .nodeId("id")
      .nodeLabel((n) => `<div class="g3-tip"><b>${(n as Node3D).label}</b><span>${(n as Node3D).kind.replace("_", " ")}</span></div>`)
      .nodeVal((n) => Math.max(1, (n as Node3D).radius ** 1.6 / 6))
      .nodeOpacity(0.95)
      .nodeResolution(14)
      .nodeColor((n) => {
        const node = n as Node3D;
        const { id: f, around } = focus.current;
        return f && !around.has(node.id) ? DIM : node.color;
      })
      .linkColor((l) => {
        const link = l as Link3D;
        const f = focus.current.id;
        if (f && (id(link.source) === f || id(link.target) === f)) return "#7c97ff";
        if (f) return "rgba(120,130,150,0.06)";
        return link.type === "supersedes" ? "rgba(177,132,255,0.6)" : "rgba(190,200,225,0.28)";
      })
      .linkWidth((l) => {
        const f = focus.current.id;
        const link = l as Link3D;
        return f && (id(link.source) === f || id(link.target) === f) ? 1.2 : 0;
      })
      .linkOpacity(0.55)
      .onNodeHover((n) => {
        el.style.cursor = n ? "pointer" : "grab";
        if (!n) setFocus(selectedRef.current);
        else setFocus((n as Node3D).id);
      })
      .onNodeClick((n) => {
        const node = n as Node3D;
        live.current.onSelect(node.id);
        flyTo(node);
      })
      .onBackgroundClick(() => live.current.onSelect(null));
    fg.current = f;

    const resize = () => {
      const r = el.getBoundingClientRect();
      f.width(r.width).height(r.height);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el);
    resize();
    return () => {
      observer.disconnect();
      f._destructor();
      el.replaceChildren();
    };
  }, []);

  const selectedRef = useRef<string | null>(selectedId);
  selectedRef.current = selectedId;

  const flyTo = (node: Node3D) => {
    const f = fg.current;
    if (!f || node.x === undefined) return;
    const distance = 140;
    const ratio = 1 + distance / Math.max(1, Math.hypot(node.x, node.y ?? 0, node.z ?? 0));
    f.cameraPosition({ x: node.x * ratio, y: (node.y ?? 0) * ratio, z: (node.z ?? 0) * ratio }, { x: node.x, y: node.y ?? 0, z: node.z ?? 0 }, 900);
  };

  const graphKey = useMemo(
    () => graph.nodes.map((n) => `${n.id}:${n.status ?? ""}`).join("|") + "#" + graph.edges.map((e) => e.id).join("|"),
    [graph],
  );

  // Data and filters. Positions survive refreshes so the layout does not restart.
  useEffect(() => {
    const f = fg.current;
    if (!f) return;
    const previous = new Map(((f.graphData().nodes as Node3D[]) ?? []).map((n) => [n.id, n]));
    const scene = buildScene(graph, settings, selectedId, new Map());
    adjacency.current = scene.adj;
    const nodes: Node3D[] = scene.nodes.map((n) => {
      const p = previous.get(n.id);
      return { id: n.id, label: n.label, kind: n.kind, status: n.status, degree: n.degree, radius: n.radius, color: n.color, x: p?.x, y: p?.y, z: p?.z };
    });
    const links: Link3D[] = scene.links.map((l) => ({ source: l.source.id, target: l.target.id, type: l.type }));
    const fresh = previous.size === 0;
    f.graphData({ nodes, links });
    if (fresh) setTimeout(() => f.zoomToFit(700, 60), 1200);
  }, [graphKey, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, settings.local ? selectedId : null]);

  // Forces follow the same sliders as the 2D view.
  useEffect(() => {
    const f = fg.current;
    if (!f) return;
    (f.d3Force("charge") as unknown as { strength(v: number): void } | undefined)?.strength(-settings.repel * 6);
    (f.d3Force("link") as unknown as { distance(v: number): void } | undefined)?.distance(settings.linkDistance * 0.8);
    f.d3ReheatSimulation();
  }, [settings.repel, settings.linkDistance]);

  useEffect(() => setFocus(selectedId), [selectedId, graphKey]);

  commands.current = {
    fit: () => fg.current?.zoomToFit(600, 60),
    center: () => {
      const node = (fg.current?.graphData().nodes as Node3D[] | undefined)?.find((n) => n.id === selectedRef.current);
      if (node) flyTo(node);
      else fg.current?.zoomToFit(600, 60);
    },
    zoomBy: (factor) => {
      const f = fg.current;
      if (!f) return;
      const p = f.cameraPosition();
      f.cameraPosition({ x: p.x / factor, y: p.y / factor, z: p.z / factor }, undefined, 300);
    },
  };

  useEffect(() => onZoom(1), []);
  return <div ref={host} className="force-graph-3d" />;
}
