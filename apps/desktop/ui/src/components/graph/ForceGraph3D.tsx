import { useEffect, useMemo, useRef } from "react";
import ForceGraph3DFactory, { type ForceGraph3DInstance } from "3d-force-graph";
import SpriteText from "three-spritetext";
import { BufferAttribute, BufferGeometry, Color, Group, LineBasicMaterial, LineSegments, Mesh, MeshLambertMaterial, SphereGeometry } from "three";
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
  node: Node3D;
  group: Group;
  mesh: Mesh;
  material: MeshLambertMaterial;
  /** Created only when the node needs a name; most nodes never do. */
  label: SpriteText | null;
  color: Color;
  alwaysLabelled: boolean;
}

/** One low-poly sphere shared by every node; each node only scales it. */
const SPHERE = new SphereGeometry(1, 12, 9);

const endId = (end: string | Node3D) => (typeof end === "string" ? end : end.id);
const DIM = new Color("#3a4152");
const LINK = new Color("#8c97b3");
const LINK_HOT = new Color("#8fa6ff");
const DIM_LINK = new Color("#1d2230");
const LINK_SUPERSEDES = new Color("#a98ee8");
const linkOpacity = (width: number) => Math.min(0.9, 0.22 + width * 0.14);
const LABELLED_KINDS = new Set(["decision", "rejected_option", "dead_end", "fact", "preference", "document", "module"]);

/** Text fade threshold in 2D maps to how many hub files are named in 3D: lower fade, more names. */
const hubLabels = (textFade: number) => Math.round(Math.min(60, Math.max(4, 18 / textFade)));
const nodeRadius = (n: Node3D, size: number) => (2.2 + n.radius * 0.55) * size;

function makeLabel(n: Node3D, r: number): SpriteText {
  const label = new SpriteText(shortLabel(n), n.kind === "file" ? 5 : 6.5, "#d6dcea");
  label.fontFace = "Inter, -apple-system, sans-serif";
  label.backgroundColor = "rgba(11,14,20,0.55)";
  label.padding = 1.2;
  label.borderRadius = 2;
  label.position.set(0, r + 5, 0);
  return label;
}

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
  /** All links drawn as ONE line object: one GPU draw call instead of one per link. */
  const linkLines = useRef<LineSegments | null>(null);
  /** Nodes that keep a visible name; decided before the data is handed to the renderer. */
  const named = useRef(new Set<string>());
  const needsFit = useRef(true);
  const adjacency = useRef<Map<string, Set<string>>>(new Map());
  const selectedRef = useRef<string | null>(selectedId);
  selectedRef.current = selectedId;
  const live = useRef({ onSelect });
  live.current = { onSelect };

  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const centreRef = useRef(settings.centerForce);
  centreRef.current = settings.centerForce;
  const sizeRef = useRef(settings.nodeSize);
  sizeRef.current = settings.nodeSize;

  const showLabel = (h: Handles, visible: boolean) => {
    if (visible && !h.label) {
      h.label = makeLabel(h.node, nodeRadius(h.node, sizeRef.current));
      h.group.add(h.label);
    }
    if (h.label) h.label.visible = visible;
  };

  const applyFocus = (focusId: string | null) => {
    const around = new Set(focusId ? [focusId, ...(adjacency.current.get(focusId) ?? [])] : []);
    for (const [id, h] of handles.current) {
      const lit = !focusId || around.has(id);
      h.material.color.copy(lit ? h.color : DIM);
      h.material.emissive.copy(lit ? h.color : DIM).multiplyScalar(id === focusId ? 0.55 : 0.18);
      showLabel(h, focusId ? around.has(id) : h.alwaysLabelled);
      if (h.label) h.label.color = id === focusId ? "#ffffff" : "#d6dcea";
    }
    paintLinks(focusId);
  };

  const linkList = () => (fg.current?.graphData().links ?? []) as Link3D[];

  /** Colour per link end: highlighted, dimmed, or normal. Written into one shared buffer. */
  const paintLinks = (focusId: string | null) => {
    const lines = linkLines.current;
    if (!lines) return;
    const colors = lines.geometry.getAttribute("color") as BufferAttribute;
    linkList().forEach((l, i) => {
      const hot = focusId !== null && (endId(l.source) === focusId || endId(l.target) === focusId);
      const c = hot ? LINK_HOT : focusId ? DIM_LINK : l.type === "supersedes" ? LINK_SUPERSEDES : LINK;
      colors.setXYZ(i * 2, c.r, c.g, c.b);
      colors.setXYZ(i * 2 + 1, c.r, c.g, c.b);
    });
    colors.needsUpdate = true;
  };

  /** Follow the simulation: copy node positions into the line buffer. */
  const placeLinks = () => {
    const lines = linkLines.current;
    if (!lines) return;
    const pos = lines.geometry.getAttribute("position") as BufferAttribute;
    linkList().forEach((l, i) => {
      const s = l.source as Node3D, d = l.target as Node3D;
      pos.setXYZ(i * 2, s.x ?? 0, s.y ?? 0, s.z ?? 0);
      pos.setXYZ(i * 2 + 1, d.x ?? 0, d.y ?? 0, d.z ?? 0);
    });
    pos.needsUpdate = true;
    lines.geometry.computeBoundingSphere();
  };

  const rebuildLinkBuffer = () => {
    const f = fg.current;
    if (!f) return;
    if (linkLines.current) {
      f.scene().remove(linkLines.current);
      linkLines.current.geometry.dispose();
    }
    const n = linkList().length;
    const geometry = new BufferGeometry();
    geometry.setAttribute("position", new BufferAttribute(new Float32Array(n * 6), 3));
    geometry.setAttribute("color", new BufferAttribute(new Float32Array(n * 6), 3));
    const material = new LineBasicMaterial({ vertexColors: true, transparent: true, opacity: linkOpacity(settingsRef.current.linkWidth) });
    linkLines.current = new LineSegments(geometry, material);
    linkLines.current.visible = !settingsRef.current.arrows;
    f.scene().add(linkLines.current);
    placeLinks();
  };

  /** Frame the connected part of the graph; loose, unlinked nodes at the edges do not shrink the view. */
  const fitCore = (ms: number) => {
    const f = fg.current;
    if (!f) return;
    const connected = (f.graphData().nodes as Node3D[]).filter((n) => n.degree > 0).length;
    f.zoomToFit(ms, 30, connected > 3 ? (n) => (n as Node3D).degree > 0 : undefined);
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
        const superseded = node.status === "superseded";
        const material = new MeshLambertMaterial({ color, emissive: color.clone().multiplyScalar(0.18), transparent: true, opacity: superseded ? 0.3 : 0.96 });
        const mesh = new Mesh(SPHERE, material);
        mesh.scale.setScalar(nodeRadius(node, sizeRef.current));
        const group = new Group();
        group.add(mesh);
        const h: Handles = { node, group, mesh, material, label: null, color, alwaysLabelled: named.current.has(node.id) };
        handles.current.set(node.id, h);
        showLabel(h, h.alwaysLabelled);
        return group;
      })
      .linkColor(() => "#8c97b3")
      .linkOpacity(0.32)
      .linkWidth(0)
      .linkVisibility(() => settingsRef.current.arrows)
      .onEngineTick(() => placeLinks())
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
        placeLinks();
        // Fit once the layout has settled, not before it has spread out.
        if (needsFit.current) {
          needsFit.current = false;
          fitCore(700);
        }
        applyFocus(selectedRef.current);
      });
    // Gentle pull toward the centre. Without it, unconnected nodes drift far out in 3D and the
    // fitted view becomes a speck.
    let pulled: Node3D[] = [];
    const pull = () => 0.01 + centreRef.current * 0.07;
    const gravity = Object.assign(
      (alpha: number) => {
        for (const n of pulled as (Node3D & { vx?: number; vy?: number; vz?: number })[]) {
          const k = pull() * alpha;
          n.vx = (n.vx ?? 0) - (n.x ?? 0) * k;
          n.vy = (n.vy ?? 0) - (n.y ?? 0) * k;
          n.vz = (n.vz ?? 0) - (n.z ?? 0) * k;
        }
      },
      { initialize: (nodes: Node3D[]) => { pulled = nodes; } },
    );
    f.d3Force("gravity", gravity as never);
    // Retina at full resolution quadruples the pixels to shade for little visible gain.
    f.renderer().setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
    fg.current = f;
    if (location.search.includes("debug3d")) (window as unknown as { __engram3d?: unknown }).__engram3d = f;

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
    const hubs = new Set([...nodes].sort((a, b) => b.degree - a.degree).slice(0, hubLabels(settings.textFade)).map((n) => n.id));
    named.current = new Set(nodes.filter((n) => LABELLED_KINDS.has(n.kind) || hubs.has(n.id)).map((n) => n.id));
    handles.current.clear();
    if (previous.size === 0) needsFit.current = true;
    f.graphData({ nodes, links });
    rebuildLinkBuffer();
  }, [graphKey, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, settings.local ? selectedId : null]);

  // Forces follow the same sliders as the 2D view; reheat briefly, then it stops again.
  useEffect(() => {
    const f = fg.current;
    if (!f) return;
    (f.d3Force("charge") as unknown as { strength(v: number): void } | undefined)?.strength(-settings.repel * 7);
    const link = f.d3Force("link") as unknown as { distance(v: number): unknown; strength(v: number): unknown } | undefined;
    link?.distance(settings.linkDistance * 0.7);
    link?.strength(settings.linkForce);
    f.d3ReheatSimulation();
  }, [settings.repel, settings.linkDistance, settings.linkForce, settings.centerForce]);

  // Display settings change objects in place; nothing is rebuilt and the layout does not move.
  useEffect(() => {
    for (const h of handles.current.values()) {
      const r = nodeRadius(h.node, settings.nodeSize);
      h.mesh.scale.setScalar(r);
      h.label?.position.set(0, r + 5, 0);
    }
  }, [settings.nodeSize]);

  useEffect(() => {
    const nodes = [...handles.current.values()].map((h) => h.node);
    const hubs = new Set(nodes.sort((a, b) => b.degree - a.degree).slice(0, hubLabels(settings.textFade)).map((n) => n.id));
    named.current = new Set(nodes.filter((n) => LABELLED_KINDS.has(n.kind) || hubs.has(n.id)).map((n) => n.id));
    for (const h of handles.current.values()) h.alwaysLabelled = named.current.has(h.node.id);
    applyFocus(selectedRef.current);
  }, [settings.textFade]);

  useEffect(() => {
    // Arrows need the library's per-link objects; without arrows the single batched buffer is used.
    const f = fg.current;
    if (!f) return;
    f.linkDirectionalArrowLength(settings.arrows ? 3.5 : 0).linkDirectionalArrowRelPos(1).linkVisibility(() => settings.arrows);
    if (linkLines.current) linkLines.current.visible = !settings.arrows;
  }, [settings.arrows]);

  useEffect(() => {
    const m = linkLines.current?.material as LineBasicMaterial | undefined;
    if (m) m.opacity = linkOpacity(settings.linkWidth);
    fg.current?.linkOpacity(linkOpacity(settings.linkWidth));
  }, [settings.linkWidth]);

  useEffect(() => applyFocus(selectedId), [selectedId]);

  commands.current = {
    fit: () => fitCore(600),
    center: () => {
      const node = (fg.current?.graphData().nodes as Node3D[] | undefined)?.find((n) => n.id === selectedRef.current);
      if (node) flyTo(node);
      else fitCore(600);
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
