import { useEffect, useMemo, useRef } from "react";
import { forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, forceZ, type Simulation3D } from "d3-force-3d";
import type { Graph } from "../../api/types.ts";
import { type Camera, draw, fitCamera, radius } from "./draw.ts";
import { buildScene, type SimLink, type SimNode } from "./model.ts";
import { project, unproject, type View } from "./projection.ts";
import type { GraphSettings } from "./settings.ts";

export interface GraphCommands {
  fit(): void;
  center(): void;
  zoomBy(factor: number): void;
}

interface Props {
  graph: Graph;
  settings: GraphSettings;
  selectedId: string | null;
  onSelect(id: string | null): void;
  onZoom(k: number): void;
  commands: React.MutableRefObject<GraphCommands | null>;
}

type Drag =
  | { node: SimNode; moved: boolean }
  | { pan: { x: number; y: number; cx: number; cy: number }; moved: boolean }
  | { rotate: { x: number; y: number; yaw: number; pitch: number }; moved: boolean }
  | null;

/**
 * Obsidian-style graph: a live force simulation drawn on canvas, with drag, pan, zoom and hover focus.
 * In 3D the same layout runs on three axes and dragging empty space rotates it; drawing, labels and
 * highlighting are shared, so both modes look and behave the same.
 */
export function ForceGraph({ graph, settings, selectedId, onSelect, onZoom, commands }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const sim = useRef<Simulation3D<SimNode, SimLink> | null>(null);
  const scene = useRef<{ nodes: SimNode[]; links: SimLink[]; adj: Map<string, Set<string>> }>({ nodes: [], links: [], adj: new Map() });
  const cam = useRef<Camera>({ x: 0, y: 0, k: 1 });
  const view = useRef<View>({ yaw: 0, pitch: 0 });
  const size = useRef({ w: 0, h: 0 });
  const hover = useRef<string | null>(null);
  const drag = useRef<Drag>(null);
  /** Auto-fit stages: 0 = not yet, 1 = rough fit done, 2 = settled or the user took over. */
  const autoFit = useRef(0);
  const frame = useRef(0);
  const live = useRef({ settings, selectedId, onSelect, onZoom });
  live.current = { settings, selectedId, onSelect, onZoom };
  const flat = () => live.current.settings.mode !== "3d";

  /** Rotate and project every node; in 3D also order them far to near for painting. */
  const projected = () => {
    const nodes = scene.current.nodes;
    project(nodes, view.current, flat());
    return flat() ? nodes : [...nodes].sort((a, b) => b.depth! - a.depth!);
  };

  const render = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      const ctx = canvas.current?.getContext("2d");
      if (!ctx) return;
      const { links, adj } = scene.current;
      const nodes = projected();
      const focusId = hover.current ?? live.current.selectedId;
      const neighbours = new Set(focusId ? [focusId, ...(adj.get(focusId) ?? [])] : []);
      const dpr = window.devicePixelRatio || 1;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      draw(ctx, size.current.w, size.current.h, cam.current, nodes, links, live.current.settings, { id: focusId, neighbours, selected: live.current.selectedId });
    });
  };

  const fit = () => {
    // In 3D, unlinked nodes drift far out in depth; frame the connected part so it stays readable.
    const nodes = projected(), core = nodes.filter((n) => n.degree > 0);
    cam.current = fitCamera(!flat() && core.length > 3 ? core : nodes, size.current.w, size.current.h);
    live.current.onZoom(cam.current.k);
    render();
  };

  // Canvas, simulation and input handlers live for the component's lifetime.
  useEffect(() => {
    const el = canvas.current!;
    const resize = () => {
      const rect = el.parentElement!.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      size.current = { w: rect.width, h: rect.height };
      el.width = rect.width * dpr;
      el.height = rect.height * dpr;
      el.style.width = `${rect.width}px`;
      el.style.height = `${rect.height}px`;
      if (autoFit.current < 2) cam.current = { ...cam.current, x: rect.width / 2, y: rect.height / 2 };
      render();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el.parentElement!);
    resize();

    const s = forceSimulation<SimNode, SimLink>().numDimensions(2).alphaDecay(0.03).velocityDecay(0.35);
    s.on("tick", () => {
      if (autoFit.current === 0 && s.alpha() < 0.3) { autoFit.current = 1; fit(); }
      else if (autoFit.current === 1 && s.alpha() < 0.05) { autoFit.current = 2; fit(); }
      render();
    });
    sim.current = s;

    const point = (e: PointerEvent | WheelEvent) => {
      const r = el.getBoundingClientRect();
      return { px: e.clientX - r.left, py: e.clientY - r.top };
    };
    const world = (px: number, py: number) => ({ x: (px - cam.current.x) / cam.current.k, y: (py - cam.current.y) / cam.current.k });
    /** The nearest node under the pointer (projected positions are current from the last frame). */
    const hit = (px: number, py: number) => {
      const p = world(px, py);
      let best: SimNode | null = null;
      for (const n of scene.current.nodes) {
        if (n.sx === undefined) continue;
        const inside = Math.hypot(n.sx - p.x, n.sy! - p.y) <= radius(n, live.current.settings) + 4 / cam.current.k;
        if (inside && (!best || n.depth! < best.depth!)) best = n;
      }
      return best;
    };

    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId);
      const { px, py } = point(e);
      const node = hit(px, py);
      autoFit.current = 2;
      if (node) {
        node.fx = node.x;
        node.fy = node.y;
        if (!flat()) node.fz = node.z;
        s.alphaTarget(0.25).restart();
        drag.current = { node, moved: false };
      } else if (!flat() && !e.shiftKey) {
        drag.current = { rotate: { x: px, y: py, ...view.current }, moved: false };
      } else {
        drag.current = { pan: { x: px, y: py, cx: cam.current.x, cy: cam.current.y }, moved: false };
      }
    };
    const move = (e: PointerEvent) => {
      const { px, py } = point(e);
      const d = drag.current;
      if (d && "node" in d) {
        const p = world(px, py);
        if (flat()) {
          d.node.fx = p.x;
          d.node.fy = p.y;
        } else {
          const q = unproject(p.x, p.y, d.node.depth ?? 0, view.current);
          d.node.fx = q.x;
          d.node.fy = q.y;
          d.node.fz = q.z;
        }
        d.moved = true;
      } else if (d && "rotate" in d) {
        const r = d.rotate;
        const pitch = Math.max(-Math.PI / 2, Math.min(Math.PI / 2, r.pitch - (py - r.y) * 0.006));
        view.current = { yaw: r.yaw + (px - r.x) * 0.006, pitch };
        d.moved = d.moved || Math.hypot(px - r.x, py - r.y) > 3;
        render();
      } else if (d) {
        cam.current = { ...cam.current, x: d.pan.cx + px - d.pan.x, y: d.pan.cy + py - d.pan.y };
        d.moved = d.moved || Math.hypot(px - d.pan.x, py - d.pan.y) > 3;
        render();
      } else {
        const id = hit(px, py)?.id ?? null;
        if (id !== hover.current) {
          hover.current = id;
          el.style.cursor = id ? "pointer" : "grab";
          render();
        }
      }
    };
    const up = () => {
      const d = drag.current;
      drag.current = null;
      if (d && "node" in d) {
        d.node.fx = null;
        d.node.fy = null;
        d.node.fz = null;
        s.alphaTarget(0);
        if (!d.moved) live.current.onSelect(d.node.id);
      } else if (d && !d.moved) {
        live.current.onSelect(null);
      }
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const { px, py } = point(e);
      const c = cam.current;
      const k = Math.min(6, Math.max(0.1, c.k * Math.exp(-e.deltaY * 0.0015)));
      cam.current = { k, x: px - ((px - c.x) * k) / c.k, y: py - ((py - c.y) * k) / c.k };
      autoFit.current = 2;
      live.current.onZoom(k);
      render();
    };
    const leave = () => { hover.current = null; render(); };

    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointerleave", leave);
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("dblclick", fit);
    return () => {
      observer.disconnect();
      s.stop();
      cancelAnimationFrame(frame.current);
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointerleave", leave);
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("dblclick", fit);
    };
  }, []);

  // Rebuild the scene when data or filters change, keeping positions so nothing jumps.
  // Polling returns a fresh object every few seconds; only real changes count, whatever the order.
  const graphKey = useMemo(
    () => graph.nodes.map((n) => `${n.id}:${n.status ?? ""}:${n.label}`).sort().join("|") + "#" + graph.edges.map((e) => e.id).sort().join("|"),
    [graph],
  );
  const localKey = settings.local ? selectedId : null;
  useEffect(() => {
    const s = sim.current;
    if (!s) return;
    const dims = settings.mode === "3d" ? 3 : 2;
    const modeChanged = s.numDimensions() !== dims;
    const previous = new Map(scene.current.nodes.map((n) => [n.id, { x: n.x, y: n.y, z: n.z, vx: n.vx, vy: n.vy, vz: n.vz }]));
    const next = buildScene(graph, settings, selectedId, previous);
    if (modeChanged) toDimensions(next.nodes, dims, view);
    const structureChanged = next.nodes.length !== scene.current.nodes.length || next.links.length !== scene.current.links.length;
    scene.current = next;
    s.numDimensions(dims);
    s.nodes(next.nodes);
    // Big nodes (parts, services) push harder and sit further from what they link to, so a few
    // landmarks spread out with their endpoints and memories around them instead of piling up.
    const size = (d: SimNode) => Math.max(1, d.radius / 6);
    s.force("link", forceLink<SimNode, SimLink>(next.links).id((d) => d.id)
      .distance((l) => settings.linkDistance + (l.source.radius + l.target.radius) * 2.5).strength(settings.linkForce));
    s.force("charge", forceManyBody<SimNode>().strength((d) => -settings.repel * 10 * size(d)).distanceMax(900));
    s.force("x", flowColumns(next.links, settings));
    s.force("y", forceY<SimNode>(0).strength(settings.centerForce * 0.06));
    s.force("z", dims === 3 ? forceZ<SimNode>(0).strength(settings.centerForce * 0.06) : null);
    s.force("collide", forceCollide<SimNode>((d) => d.radius * settings.nodeSize + 10));
    if (structureChanged && localKey) autoFit.current = 0;
    if (modeChanged || (structureChanged && autoFit.current < 2)) {
      // Settle the layout up front (a few ms for hundreds of nodes), so the first frame is
      // already readable even where animation frames are throttled; then keep gentle motion.
      s.stop().alpha(1);
      s.tick(next.nodes.length > 1500 ? 80 : 220);
      autoFit.current = 2;
      fit();
      s.alpha(0.08).restart();
    } else {
      // New nodes arriving in the background start beside a neighbour; a small nudge lets them
      // find their place without shoving the rest of the layout around.
      s.alpha(structureChanged ? 0.1 : 0.15).restart();
    }
    render();
  }, [graphKey, settings.mode, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, localKey,
      settings.linkDistance, settings.linkForce, settings.repel, settings.centerForce, settings.nodeSize, settings.view]);

  // Display-only changes (selection, sizes, labels, arrows) just redraw.
  useEffect(render, [selectedId, settings.linkWidth, settings.textFade, settings.arrows]);

  // Keyboard: Z fits, C centers on the selection, like Supermemory's graph.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select") || e.metaKey || e.ctrlKey) return;
      if (e.key === "z") fit();
      if (e.key === "c") commands.current?.center();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  commands.current = {
    fit,
    center() {
      projected();
      const n = scene.current.nodes.find((x) => x.id === live.current.selectedId);
      const target = n ? { x: n.sx!, y: n.sy! } : { x: 0, y: 0 };
      cam.current = { ...cam.current, x: size.current.w / 2 - target.x * cam.current.k, y: size.current.h / 2 - target.y * cam.current.k };
      render();
    },
    zoomBy(factor) {
      const c = cam.current, k = Math.min(6, Math.max(0.1, c.k * factor));
      const { w, h } = size.current;
      cam.current = { k, x: w / 2 - ((w / 2 - c.x) * k) / c.k, y: h / 2 - ((h / 2 - c.y) * k) / c.k };
      live.current.onZoom(k);
      render();
    },
  };

  return <canvas ref={canvas} className="force-graph" />;
}

/** Horizontal distance between the flow columns in the architecture view. */
const COLUMN = 340;

/**
 * Horizontal pull. In the architecture view, nodes line up as a flow, left to right: parts that only
 * call, endpoints, parts that serve, external services; memories and unconnected parts float. In the
 * code view, a gentle pull to the centre as before.
 */
function flowColumns(links: SimLink[], settings: GraphSettings) {
  const centre = settings.centerForce * 0.06;
  if (settings.view !== "architecture") return forceX<SimNode>(0).strength(centre);
  const calls = new Set<string>(), serves = new Set<string>();
  for (const l of links) {
    if (l.type === "calls") calls.add(l.source.id);
    if (l.type === "served by") serves.add(l.target.id);
  }
  const column = (d: SimNode): number | null => {
    if (d.kind === "endpoint") return 0;
    if (d.kind === "service") return 2;
    if (d.kind !== "area") return null;
    if (serves.has(d.id)) return 1;
    return calls.has(d.id) ? -1 : null;
  };
  return forceX<SimNode>((d) => (column(d) ?? 0) * COLUMN).strength((d) => (column(d) === null ? centre : 0.35));
}

/**
 * Switch the layout between flat and spatial. Going to 3D lifts nodes off the plane by a random
 * depth proportional to the layout's size, so the simulation can spread them; going back flattens
 * them and resets the rotation.
 */
function toDimensions(nodes: SimNode[], dims: 2 | 3, view: React.MutableRefObject<View>) {
  view.current = { yaw: 0, pitch: 0 };
  let span = 1;
  for (const n of nodes) span = Math.max(span, Math.abs(n.x ?? 0), Math.abs(n.y ?? 0));
  for (const n of nodes) {
    n.z = dims === 3 ? (Math.random() - 0.5) * span : undefined;
    n.vz = 0;
    n.fz = null;
  }
}
