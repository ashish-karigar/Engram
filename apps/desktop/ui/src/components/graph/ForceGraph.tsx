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
  /** Open or close a node of the zoomable map (double-click). */
  onExpand?(id: string): void;
  onZoom(k: number): void;
  commands: React.MutableRefObject<GraphCommands | null>;
}

type Drag =
  | { node: SimNode; moved: boolean }
  | { pan: { x: number; y: number; cx: number; cy: number }; moved: boolean }
  | { rotate: { x: number; y: number; yaw: number; pitch: number }; moved: boolean }
  | null;

/**
 * The project as one network: a live force simulation drawn on canvas, with drag, pan, zoom and hover focus.
 * In 3D the same layout runs on three axes and dragging empty space rotates it; drawing, labels and
 * highlighting are shared, so both modes look and behave the same.
 */
export function ForceGraph({ graph, settings, selectedId, onSelect, onExpand, onZoom, commands }: Props) {
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
  const live = useRef({ settings, selectedId, onSelect, onExpand, onZoom });
  live.current = { settings, selectedId, onSelect, onExpand, onZoom };
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
    // Double-click a node to open or close it; empty space fits the view.
    const dbl = (e: MouseEvent) => {
      const r = el.getBoundingClientRect();
      const node = hit(e.clientX - r.left, e.clientY - r.top);
      if (node && (node.children > 0 || node.expanded) && live.current.onExpand) live.current.onExpand(node.id);
      else fit();
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
    const dims = settings.mode === "3d" ? 3 : 2;
    const modeChanged = s.numDimensions() !== dims;
    const previous = new Map(scene.current.nodes.map((n) => [n.id, { x: n.x, y: n.y, z: n.z, vx: n.vx, vy: n.vy, vz: n.vz }]));
    const next = buildScene(graph, settings, selectedId, previous);
    if (modeChanged) toDimensions(next.nodes, dims, view);
    const structureChanged = next.nodes.length !== scene.current.nodes.length || next.links.length !== scene.current.links.length;
    scene.current = next;
    if (location.search.includes("debuggraph")) (window as unknown as { __graph: unknown }).__graph = next;
    s.numDimensions(dims);
    s.nodes(next.nodes);
    // Big nodes (parts, services) push harder, so a few landmarks spread out with their contents around them.
    const size = (d: SimNode) => Math.max(1, d.radius / 6);
    s.force("link", forceLink<SimNode, SimLink>(next.links).id((d) => d.id)
      .distance((l) => linkDistance(l, settings))
      .strength((l) => linkStrength(l, settings)));
    s.force("charge", forceManyBody<SimNode>().strength((d) => -settings.repel * 10 * size(d)).distanceMax(900));
    const layout = placeLandmarks(next.nodes, next.links);
    const pull = (d: SimNode) => (layout.spots.has(d.id) ? layout.strength(d) : d.area && layout.spots.has(d.area) ? 0.05 : settings.centerForce * 0.06);
    const at = (d: SimNode) => layout.spots.get(d.id) ?? (d.area ? layout.spots.get(d.area) : undefined);
    s.force("x", forceX<SimNode>((d) => at(d)?.x ?? 0).strength(pull));
    s.force("y", forceY<SimNode>((d) => at(d)?.y ?? 0).strength(pull));
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
      // Nodes appearing (something was opened) start beside their parent and need room to settle;
      // background refreshes that only add a node or two get a gentle nudge.
      const added = next.nodes.filter((n) => !previous.has(n.id)).length;
      s.alpha(added > 2 ? 0.6 : structureChanged ? 0.1 : 0.15).restart();
    }
    render();
  }, [graphKey, settings.mode, settings.search, settings.hidden, settings.showOrphans, settings.local, settings.localDepth, localKey,
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

const isApi = (n: SimNode) => n.kind === "api" || n.kind === "api_group" || n.kind === "endpoint";

/** Children hug what holds them, memories sit by what they explain, and links between parts stay loose. */
function linkStrength(l: SimLink, s: GraphSettings) {
  if (l.type === "contains") return 1;
  if (l.type === "about") return s.linkForce * 0.8;
  return s.linkForce * 0.15;
}

function linkDistance(l: SimLink, s: GraphSettings) {
  const r = l.source.radius + l.target.radius;
  if (l.type === "contains") return 30 + r * 1.6;
  if (l.type === "about") return 45 + r * 1.5;
  return s.linkDistance + r * 2.5;
}

/**
 * Fixed places for the landmarks only; everything else finds its place from its links.
 * Parts that call an API sit on the left, the parts that serve it on the right, and the API and the
 * external services both sides use sit in the channel between them. A service only one part uses
 * (a database) sits beyond that part. Parts with no API link are laid out in a grid below, and notes
 * that belong to nothing get their own corner. Spacing grows with what each part holds.
 */
function placeLandmarks(nodes: SimNode[], links: SimLink[]) {
  const spots = new Map<string, { x: number; y: number }>();
  const areas = nodes.filter((n) => n.kind === "area");
  const held = new Map<string, number>();
  for (const n of nodes) if (n.area) held.set(n.area, (held.get(n.area) ?? 0) + 1);
  const reach = (a: SimNode) => 110 + 36 * Math.sqrt(held.get(a.id) ?? 1);

  const callers = new Set<string>(), servers = new Set<string>();
  const hubCallers = new Map<string, Set<string>>();
  for (const l of links) {
    if (l.type !== "calls" || !isApi(l.target) || !l.source.area) continue;
    const hub = l.target.kind === "api" ? l.target : nodes.find((n) => n.id === l.target.hub);
    if (!hub) continue;
    if (l.source.area !== hub.serves) callers.add(l.source.area);
    if (!hubCallers.has(hub.id)) hubCallers.set(hub.id, new Set());
    hubCallers.get(hub.id)!.add(l.source.area);
  }
  for (const n of nodes) if (n.kind === "api" && n.serves) servers.add(n.serves);

  const bySize = (a: SimNode, b: SimNode) => (held.get(b.id) ?? 0) - (held.get(a.id) ?? 0);
  const left = areas.filter((a) => callers.has(a.id) && !servers.has(a.id)).sort(bySize);
  const right = areas.filter((a) => servers.has(a.id)).sort(bySize);
  const rest = areas.filter((a) => !callers.has(a.id) && !servers.has(a.id)).sort(bySize);

  // One row for the parts that talk over an API, with room for the channel between its two halves.
  const channel = left.length && right.length ? 420 : 0;
  const rowWidth = [...left, ...right].reduce((w, a) => w + reach(a) * 2 + 120, channel);
  let x = -rowWidth / 2;
  for (const half of [left, right]) {
    for (const a of half) {
      spots.set(a.id, { x: x + reach(a), y: 0 });
      x += reach(a) * 2 + 120;
    }
    x += channel;
  }
  const wired = left.length + right.length > 0;
  const rowReach = Math.max(0, ...[...left, ...right].map(reach));

  // Parts that only use each other in-process (libraries, crates): a grid below, biggest first.
  const cols = Math.max(1, Math.ceil(Math.sqrt(rest.length)));
  const cell = Math.max(0, ...rest.map(reach)) * 2 + 140;
  rest.forEach((a, i) => {
    const row = Math.floor(i / cols), n = Math.min(cols, rest.length - row * cols);
    spots.set(a.id, { x: ((i % cols) - (n - 1) / 2) * cell, y: (wired ? rowReach * 2 + 260 : 0) + row * cell });
  });

  const centre = (ids: Iterable<string>) => {
    const p = [...ids].map((id) => spots.get(id)).filter((q): q is { x: number; y: number } => !!q);
    return p.length ? { x: p.reduce((t, q) => t + q.x, 0) / p.length, y: p.reduce((t, q) => t + q.y, 0) / p.length } : null;
  };
  // Each API body between its callers and the part that serves it.
  nodes.filter((n) => n.kind === "api").forEach((h, i) => {
    const server = h.serves ? spots.get(h.serves) : undefined;
    const from = centre(hubCallers.get(h.id) ?? []);
    spots.set(h.id, server && from ? { x: (from.x + server.x) / 2, y: (from.y + server.y) / 2 } : server ? { x: server.x - 320, y: 0 } : from ? { x: from.x + 320, y: 0 } : { x: 0, y: 200 * (i + 1) });
  });

  // External services: between the parts that use them; beyond the part, when only one does.
  const middle = centre(wired ? [...left, ...right].map((a) => a.id) : []);
  const users = new Map<string, Set<string>>();
  for (const l of links) {
    const [svc, other] = l.source.kind === "service" ? [l.source, l.target] : l.target.kind === "service" ? [l.target, l.source] : [];
    if (svc && other?.area) users.set(svc.id, (users.get(svc.id) ?? new Set()).add(other.area));
  }
  const services = nodes.filter((n) => n.kind === "service");
  const shared = services.filter((n) => (users.get(n.id)?.size ?? 0) > 1);
  services.forEach((svc, i) => {
    const parts = users.get(svc.id) ?? new Set<string>();
    if (parts.size === 1) {
      const part = areas.find((a) => a.id === [...parts][0])!, at = spots.get(part.id)!;
      const side = at.x >= (middle?.x ?? 0) ? 1 : -1;
      spots.set(svc.id, { x: at.x + side * (reach(part) + 260), y: at.y + ((i % 3) - 1) * 150 });
    } else if (parts.size > 1) {
      const k = shared.indexOf(svc), c = centre(parts) ?? { x: 0, y: 0 };
      spots.set(svc.id, { x: c.x, y: (k % 2 ? 1 : -1) * (230 + 130 * Math.floor(k / 2)) + c.y });
    } else {
      spots.set(svc.id, { x: (middle?.x ?? 0) + 200 * (i + 1), y: -rowReach - 320 });
    }
  });

  // Notes about nothing in the map wait in a corner.
  for (const g of nodes.filter((n) => n.kind === "group")) {
    let far = 0;
    for (const p of spots.values()) far = Math.max(far, p.y);
    spots.set(g.id, { x: -rowWidth / 2 - 200, y: far / 2 + 320 });
  }
  const firm = (d: SimNode) => (d.kind === "area" ? 0.6 : d.kind === "api" ? 0.5 : d.kind === "service" ? 0.2 : 0.3);
  return { spots, strength: firm };
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
