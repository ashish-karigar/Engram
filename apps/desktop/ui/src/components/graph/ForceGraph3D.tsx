import { useEffect, useMemo, useRef } from "react";
import ForceGraph3DFactory, { type ForceGraph3DInstance } from "3d-force-graph";
import SpriteText from "three-spritetext";
import { Color, Group, Mesh, MeshLambertMaterial, SphereGeometry } from "three";
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
  fx?: number;
  fy?: number;
  fz?: number;
}

interface Link3D {
  source: string | Node3D;
  target: string | Node3D;
  type: string;
  /** Set by three-forcegraph on the link it draws; we recolor it in place. */
  __lineObj?: { material?: { color: Color; opacity: number } };
}

/** What we keep per node so focus changes can recolor without rebuilding the scene. */
interface Handles {
  material: MeshLambertMaterial;
  label: SpriteText;
  color: Color;
  alwaysLabelled: boolean;
}

const endId = (end: string | Node3D) => (typeof end === "string" ? end : end.id);
const DIM = new Color("#3a4152");
const LINK = new Color("#8c97b3");
const LINK_HOT = new Color("#8fa6ff");
const LABELLED_KINDS = new Set(["decision", "rejected_option", "dead_end", "fact", "preference", "document", "module"]);
const HUB_LABELS = 18;

function shortLabel(n: Node3D): string {
  const text = n.kind === "file" ? n.label.slice(n.label.lastIndexOf("/") + 1) : n.label;
  return text.length > 34 ? `${text.slice(0, 32)}…` : text;
}

/**
 * 3D view of the same graph. The layout is computed before the first frame and then stops, so it
 * never keeps bouncing; dragged nodes stay where they are dropped. Hover and selection recolor the
 * existing objects in place instead of rebuilding them.
 */
export function ForceGraph3D({ graph, settings, selectedId, onSelect, onZoom, commands }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const fg = useRef<ForceGraph3DInstance | null>(null);
  const handles = useRef(new Map<string, Handles>());
  /** Nodes that keep a visible name; decided before the data is handed to the renderer. */
  const named = useRef(new Set<string>());
  const needsFit = useRef(true);
  const adjacency = useRef<Map<string, Set<string>>>(new Map());
  const selectedRef = useRef<string | null>(selectedId);
  selectedRef.current = selectedId;
  const live = useRef({ onSelect });
  live.current = { onSelect };

  const applyFocus = (focusId: string | null) => {
    const around = new Set(focusId ? [focusId, ...(adjacency.current.get(focusId) ?? [])] : []);
    for (const [id, h] of handles.current) {
      const lit = !focusId || around.has(id);
      h.material.color.copy(lit ? h.color : DIM);
      h.material.emissive.copy(lit ? h.color : DIM).multiplyScalar(id === focusId ? 0.55 : 0.18);
      h.label.visible = focusId ? around.has(id) : h.alwaysLabelled;
      h.label.color = id === focusId ? "#ffffff" : "#d6dcea";
    }
    for (const l of (fg.current?.graphData().links ?? []) as Link3D[]) {
      const m = l.__lineObj?.material;
      if (!m) continue;
      const hot = focusId !== null && (endId(l.source) === focusId || endId(l.target) === focusId);
      m.color.copy(hot ? LINK_HOT : LINK);
      m.opacity = hot ? 0.95 : focusId ? 0.05 : 0.32;
    }
  };

  const flyTo = (node: Node3D) => {
    const f = fg.current;
    if (!f || node.x === undefined) return;
    const distance = 120;
    const len = Math.max(1, Math.hypot(node.x, node.y ?? 0, node.z ?? 0));
    const ratio = 1 + distance / len;
    f.cameraPosition({ x: node.x * ratio, y: (node.y ?? 0) * ratio, z: (node.z ?? 0) * ratio }, { x: node.x, y: node.y ?? 0, z: node.z ?? 0 }, 900);
  };

  useEffect(() => {
    const el = host.current!;
    const f = new ForceGraph3DFactory(el, { controlType: "orbit", rendererConfig: { alpha: true, antialias: true } })
      .backgroundColor("rgba(0,0,0,0)")
      .showNavInfo(false)
      .nodeId("id")
      .nodeLabel(() => "")
      .nodeThreeObject((n) => {
        const node = n as Node3D;
        const color = new Color(node.color);
        const material = new MeshLambertMaterial({ color, emissive: color.clone().multiplyScalar(0.18), transparent: true, opacity: 0.96 });
        const r = 2.2 + node.radius * 0.55;
        const group = new Group();
        group.add(new Mesh(new SphereGeometry(r, 18, 14), material));
        const label = new SpriteText(shortLabel(node), node.kind === "file" ? 5 : 6.5, "#d6dcea");
        label.fontFace = "Inter, -apple-system, sans-serif";
        label.backgroundColor = "rgba(11,14,20,0.55)";
        label.padding = 1.2;
        label.borderRadius = 2;
        label.position.set(0, r + 5, 0);
        const alwaysLabelled = named.current.has(node.id);
        label.visible = alwaysLabelled;
        group.add(label);
        handles.current.set(node.id, { material, label, color, alwaysLabelled });
        return group;
      })
      .linkColor(() => "#8c97b3")
      .linkOpacity(0.32)
      .linkWidth(0)
      .warmupTicks(160)
      .cooldownTicks(80)
      .d3VelocityDecay(0.55)
      .onNodeHover((n) => {
        el.style.cursor = n ? "pointer" : "grab";
        applyFocus(n ? (n as Node3D).id : selectedRef.current);
      })
      .onNodeClick((n) => {
        live.current.onSelect((n as Node3D).id);
        flyTo(n as Node3D);
      })
      .onNodeDragEnd((n) => {
        // Pin where dropped, like Obsidian, so it does not spring back and set everything moving.
        const node = n as Node3D;
        node.fx = node.x;
        node.fy = node.y;
        node.fz = node.z;
      })
      .onBackgroundClick(() => live.current.onSelect(null))
      .onEngineStop(() => {
        // Fit once the layout has settled, not before it has spread out.
        if (needsFit.current) {
          needsFit.current = false;
          f.zoomToFit(700, 30);
        }
        applyFocus(selectedRef.current);
      });
    // Gentle pull toward the centre. Without it, unconnected nodes drift far out in 3D and the
    // fitted view becomes a speck.
    let pulled: Node3D[] = [];
    const gravity = Object.assign(
      (alpha: number) => {
        for (const n of pulled as (Node3D & { vx?: number; vy?: number; vz?: number })[]) {
          n.vx = (n.vx ?? 0) - (n.x ?? 0) * 0.03 * alpha;
          n.vy = (n.vy ?? 0) - (n.y ?? 0) * 0.03 * alpha;
          n.vz = (n.vz ?? 0) - (n.z ?? 0) * 0.03 * alpha;
        }
      },
      { initialize: (nodes: Node3D[]) => { pulled = nodes; } },
    );
    f.d3Force("gravity", gravity as never);
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

  const graphKey = useMemo(
    () => graph.nodes.map((n) => `${n.id}:${n.status ?? ""}`).join("|") + "#" + graph.edges.map((e) => e.id).join("|"),
    [graph],
  );

  // Data and filters: rebuild only when the visible graph really changes; keep positions.
  useEffect(() => {
    const f = fg.current;
    if (!f) return;
    const previous = new Map(((f.graphData().nodes as Node3D[]) ?? []).map((n) => [n.id, n]));
    const scene = buildScene(graph, settings, selectedId, new Map());
    adjacency.current = scene.adj;
    const nodes: Node3D[] = scene.nodes.map((n) => {
      const p = previous.get(n.id);
      return { id: n.id, label: n.label, kind: n.kind, status: n.status, degree: n.degree, radius: n.radius, color: n.color, x: p?.x, y: p?.y, z: p?.z, fx: p?.fx, fy: p?.fy, fz: p?.fz };
    });
    const links: Link3D[] = scene.links.map((l) => ({ source: l.source.id, target: l.target.id, type: l.type }));
    // Name the important nodes: memories, documents, folders, and the most-connected files.
    const hubs = new Set([...nodes].sort((a, b) => b.degree - a.degree).slice(0, HUB_LABELS).map((n) => n.id));
    named.current = new Set(nodes.filter((n) => LABELLED_KINDS.has(n.kind) || hubs.has(n.id)).map((n) => n.id));
    handles.current.clear();
    if (previous.size === 0) needsFit.current = true;
    f.graphData({ nodes, links });
  }, [graphKey, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, settings.local ? selectedId : null]);

  // Forces follow the same sliders as the 2D view; reheat briefly, then it stops again.
  useEffect(() => {
    const f = fg.current;
    if (!f) return;
    (f.d3Force("charge") as unknown as { strength(v: number): void } | undefined)?.strength(-settings.repel * 7);
    (f.d3Force("link") as unknown as { distance(v: number): void } | undefined)?.distance(settings.linkDistance * 0.7);
  }, [settings.repel, settings.linkDistance]);

  useEffect(() => applyFocus(selectedId), [selectedId]);

  commands.current = {
    fit: () => fg.current?.zoomToFit(600, 40),
    center: () => {
      const node = (fg.current?.graphData().nodes as Node3D[] | undefined)?.find((n) => n.id === selectedRef.current);
      if (node) flyTo(node);
      else fg.current?.zoomToFit(600, 40);
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
