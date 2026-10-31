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
 * Obsidian-style 3D graph: a live force simulation on three axes, drawn on canvas with perspective.
 * Drag empty space to rotate, Shift-drag to pan, scroll to zoom, drag a node to move it. Plain
 * physics: parts, APIs and busy files become hubs because of their links, nothing is placed by hand.
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

  /** Rotate and project every node, ordered far to near for painting. */
  const projected = () => {
    const nodes = scene.current.nodes;
    project(nodes, view.current, false);
    return [...nodes].sort((a, b) => b.depth! - a.depth!);
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
      draw(ctx, size.current.w, size.current.h, cam.current, nodes, links, live.current.settings, { id: focusId, neighbours, selected: live.current.selectedId, t: performance.now() });
      // Keep the dots on the focused node's lines moving.
      if (focusId) render();
    });
  };

  /** Turn the camera so the two biggest suns lie across the screen, not one behind the other. */
  const face = () => {
    const [a, b] = scene.current.nodes.filter((n) => n.kind === "area").sort((x, y) => y.radius - x.radius);
    if (!a || !b) return;
    view.current = { yaw: Math.atan2(b.z! - a.z!, b.x! - a.x!), pitch: 0 };
  };

  const fit = () => {
    // Unlinked nodes drift far out in depth; frame the connected part so it stays readable.
    const nodes = projected(), core = nodes.filter((n) => n.degree > 0);
    cam.current = fitCamera(core.length > 3 ? core : nodes, size.current.w, size.current.h);
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

    const s = forceSimulation<SimNode, SimLink>().numDimensions(3).alphaDecay(0.008).velocityDecay(0.35);
    s.on("tick", () => {
      if (autoFit.current === 0 && s.alpha() < 0.3) { autoFit.current = 1; face(); fit(); }
      else if (autoFit.current === 1 && s.alpha() < 0.05) { autoFit.current = 2; face(); fit(); }
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
        node.fz = node.z;
        s.alphaTarget(0.25).restart();
        drag.current = { node, moved: false };
      } else if (!e.shiftKey) {
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
        const q = unproject(p.x, p.y, d.node.depth ?? 0, view.current);
        d.node.fx = q.x;
        d.node.fy = q.y;
        d.node.fz = q.z;
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
    // Double-click empty space to fit the view.
    const dbl = (e: MouseEvent) => {
      const r = el.getBoundingClientRect();
      if (!hit(e.clientX - r.left, e.clientY - r.top)) fit();
    };
    el.addEventListener("dblclick", dbl);
    return () => {
      observer.disconnect();
      s.stop();
      cancelAnimationFrame(frame.current);
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointerleave", leave);
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("dblclick", dbl);
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
    const previous = new Map(scene.current.nodes.map((n) => [n.id, { x: n.x, y: n.y, z: n.z, vx: n.vx, vy: n.vy, vz: n.vz }]));
    const next = buildScene(graph, settings, selectedId, previous);
    const structureChanged = next.nodes.length !== scene.current.nodes.length || next.links.length !== scene.current.links.length;
    scene.current = next;
    if (location.search.includes("debuggraph")) Object.assign(window, { __graph: next, __sim: s });
    s.nodes(next.nodes);
    // A solar system per part: the part is the sun, its files the planets, their functions and
    // classes the moons. Containment holds firm at full strength, whatever a sun's size, and each
    // orbit sits just outside its parent's body. APIs (rockets) are pulled by both the code that calls
    // them and the code that serves them, so they hang between the two suns. Services (spaceships)
    // follow the code that uses them. Code links inside a part tighten it; code links across parts,
    // documents and memories pull softly, so they bridge systems without merging them.
    const links = new Map<string, number>();
    for (const l of next.links) for (const id of [l.source.id, l.target.id]) links.set(id, (links.get(id) ?? 0) + 1);
    const busy = (l: SimLink) => Math.min(links.get(l.source.id) ?? 1, links.get(l.target.id) ?? 1);
    const strength = (l: SimLink) => {
      if (l.type === "contains") return 0.9;
      if (l.type === "calls" || l.type === "serves") return 0.6 / Math.sqrt(busy(l));
      if (l.type === "uses") return 0.3 / Math.sqrt(busy(l));
      const inside = l.source.area !== undefined && l.source.area === l.target.area;
      return (inside ? 0.6 : 0.02) / busy(l);
    };
    const reach = (l: SimLink) => (l.type === "contains" ? 1 : 2.5);
    s.force("link", forceLink<SimNode, SimLink>(next.links).id((d) => d.id)
      .distance((l) => (settings.linkDistance + l.source.radius + l.target.radius) * reach(l))
      .strength((l) => settings.linkForce * strength(l)));
    // Every body repels about the same (Obsidian); suns a little more, so systems keep their distance
    // without the sun pushing its own planets away.
    s.force("charge", forceManyBody<SimNode>().strength((d) => -settings.repel * 6 * (d.kind === "area" ? 3 : 1)).theta(1.1).distanceMax(1500));
    const centre = settings.centerForce * 0.02;
    s.force("x", forceX<SimNode>(0).strength(centre));
    s.force("y", forceY<SimNode>(0).strength(centre));
    s.force("z", forceZ<SimNode>(0).strength(centre));
    s.force("collide", forceCollide<SimNode>((d) => d.radius * settings.nodeSize + 2));
    if (structureChanged && localKey) autoFit.current = 0;
    if (structureChanged && autoFit.current < 2) {
      // A short burst up front so the first frame is already shaped, then the galaxies drift apart
      // on screen and the view refits as they settle.
      s.stop().alpha(1);
      s.tick(next.nodes.length > 1500 ? 60 : 120);
      autoFit.current = 1;
      face();
      fit();
      s.alpha(0.7).restart();
    } else {
      // New nodes arriving in the background start beside a neighbour; a small nudge lets them
      // find their place without shoving the rest of the layout around.
      s.alpha(structureChanged ? 0.1 : 0.05).restart();
    }
    render();
  }, [graphKey, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, localKey,
      settings.linkDistance, settings.linkForce, settings.repel, settings.centerForce, settings.nodeSize, settings.colorBy]);

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
