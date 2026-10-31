import { useEffect, useMemo, useRef } from "react";
import { forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type Simulation } from "d3-force";
import type { Graph } from "../../api/types.ts";
import { type Camera, draw, fitCamera } from "./draw.ts";
import { buildScene, type SimLink, type SimNode } from "./model.ts";
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

type Drag = { node: SimNode; moved: boolean } | { pan: { x: number; y: number; cx: number; cy: number }; moved: boolean } | null;

/** Obsidian-style graph: a live force simulation drawn on canvas, with drag, pan, zoom and hover focus. */
export function ForceGraph({ graph, settings, selectedId, onSelect, onZoom, commands }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const sim = useRef<Simulation<SimNode, SimLink> | null>(null);
  const scene = useRef<{ nodes: SimNode[]; links: SimLink[]; adj: Map<string, Set<string>> }>({ nodes: [], links: [], adj: new Map() });
  const cam = useRef<Camera>({ x: 0, y: 0, k: 1 });
  const size = useRef({ w: 0, h: 0 });
  const hover = useRef<string | null>(null);
  const drag = useRef<Drag>(null);
  /** Auto-fit stages: 0 = not yet, 1 = rough fit done, 2 = settled or the user took over. */
  const autoFit = useRef(0);
  const frame = useRef(0);
  const live = useRef({ settings, selectedId, onSelect, onZoom });
  live.current = { settings, selectedId, onSelect, onZoom };

  const render = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      const ctx = canvas.current?.getContext("2d");
      if (!ctx) return;
      const { nodes, links, adj } = scene.current;
      const focusId = hover.current ?? live.current.selectedId;
      const neighbours = new Set(focusId ? [focusId, ...(adj.get(focusId) ?? [])] : []);
      const dpr = window.devicePixelRatio || 1;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      draw(ctx, size.current.w, size.current.h, cam.current, nodes, links, live.current.settings, { id: focusId, neighbours, selected: live.current.selectedId });
    });
  };

  const fit = () => {
    cam.current = fitCamera(scene.current.nodes, size.current.w, size.current.h);
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

    const s = forceSimulation<SimNode, SimLink>().alphaDecay(0.03).velocityDecay(0.35);
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
    const hit = (px: number, py: number) => {
      const p = world(px, py);
      const nodes = scene.current.nodes;
      for (let i = nodes.length - 1; i >= 0; i--) {
        const n = nodes[i]!;
        if (Math.hypot(n.x! - p.x, n.y! - p.y) <= n.radius * live.current.settings.nodeSize + 4 / cam.current.k) return n;
      }
      return null;
    };

    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId);
      const { px, py } = point(e);
      const node = hit(px, py);
      autoFit.current = 2;
      if (node) {
        node.fx = node.x;
        node.fy = node.y;
        s.alphaTarget(0.25).restart();
        drag.current = { node, moved: false };
      } else {
        drag.current = { pan: { x: px, y: py, cx: cam.current.x, cy: cam.current.y }, moved: false };
      }
    };
    const move = (e: PointerEvent) => {
      const { px, py } = point(e);
      const d = drag.current;
      if (d && "node" in d) {
        const p = world(px, py);
        d.node.fx = p.x;
        d.node.fy = p.y;
        d.moved = true;
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
  // Polling returns a fresh object every few seconds; only real changes count.
  const graphKey = useMemo(
    () => graph.nodes.map((n) => `${n.id}:${n.status ?? ""}:${n.label}`).join("|") + "#" + graph.edges.map((e) => e.id).join("|"),
    [graph],
  );
  const localKey = settings.local ? selectedId : null;
  useEffect(() => {
    const s = sim.current;
    if (!s) return;
    const previous = new Map(scene.current.nodes.map((n) => [n.id, { x: n.x, y: n.y, vx: n.vx, vy: n.vy }]));
    const next = buildScene(graph, settings, selectedId, previous);
    const structureChanged = next.nodes.length !== scene.current.nodes.length || next.links.length !== scene.current.links.length;
    scene.current = next;
    s.nodes(next.nodes);
    s.force("link", forceLink<SimNode, SimLink>(next.links).id((d) => d.id).distance(settings.linkDistance).strength(settings.linkForce));
    s.force("charge", forceManyBody<SimNode>().strength(-settings.repel * 10).distanceMax(600));
    s.force("x", forceX<SimNode>(0).strength(settings.centerForce * 0.06));
    s.force("y", forceY<SimNode>(0).strength(settings.centerForce * 0.06));
    s.force("collide", forceCollide<SimNode>((d) => d.radius * settings.nodeSize + 10));
    if (structureChanged && localKey) autoFit.current = 0;
    if (structureChanged && autoFit.current < 2) {
      // Settle the layout up front (a few ms for hundreds of nodes), so the first frame is
      // already readable even where animation frames are throttled; then keep gentle motion.
      s.stop().alpha(1);
      s.tick(next.nodes.length > 1500 ? 80 : 220);
      autoFit.current = 2;
      fit();
      s.alpha(0.08).restart();
    } else {
      s.alpha(structureChanged ? 0.5 : 0.15).restart();
    }
    render();
  }, [graphKey, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, localKey,
      settings.linkDistance, settings.linkForce, settings.repel, settings.centerForce, settings.nodeSize]);

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
      const n = scene.current.nodes.find((x) => x.id === live.current.selectedId);
      const target = n ? { x: n.x!, y: n.y! } : { x: 0, y: 0 };
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
