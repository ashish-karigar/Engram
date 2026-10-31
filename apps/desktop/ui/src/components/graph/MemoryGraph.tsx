import { useEffect, useMemo, useRef, useState } from "react";
import { forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type Simulation } from "d3-force";
import type { Graph } from "../../api/types.ts";
import { buildScene, fill, label, type Link, type Node, type Scene } from "./scene.ts";

interface Props {
  graph: Graph;
  selectedId: string | null;
  onSelect(id: string | null): void;
}

interface Camera { x: number; y: number; k: number }
type Drag = { node: Node; moved: boolean } | { from: { x: number; y: number }; cam: Camera; moved: boolean } | null;

const LINE = "rgba(150, 158, 172, 0.22)";
const LINE_LIT = "rgba(222, 226, 233, 0.75)";
const TEXT = "#9ba1ab";
const TEXT_LIT = "#e3e6eb";

/**
 * Obsidian-style network: one force simulation drawn on a canvas. Everything is grey; hovering or
 * selecting a node brightens it and its neighbours and fades the rest. Labels appear as you zoom in.
 */
export function MemoryGraph({ graph, selectedId, onSelect }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const sim = useRef<Simulation<Node, Link> | null>(null);
  const scene = useRef<Scene>({ nodes: [], links: [], neighbours: new Map() });
  const cam = useRef<Camera>({ x: 0, y: 0, k: 1 });
  const size = useRef({ w: 0, h: 0 });
  const hover = useRef<string | null>(null);
  const drag = useRef<Drag>(null);
  const fitted = useRef(false);
  const frame = useRef(0);
  const selected = useRef(selectedId);
  selected.current = selectedId;
  const select = useRef(onSelect);
  select.current = onSelect;

  const [query, setQuery] = useState("");
  const [orphans, setOrphans] = useState(true);
  const [zoom, setZoom] = useState(1);

  const render = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      const ctx = canvas.current?.getContext("2d");
      if (ctx) draw(ctx, size.current, cam.current, scene.current, hover.current ?? selected.current, selected.current);
    });
  };

  const fit = () => {
    const { w, h } = size.current;
    const nodes = scene.current.nodes;
    if (!w || !h || !nodes.length) return false;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of nodes) { x0 = Math.min(x0, n.x!); y0 = Math.min(y0, n.y!); x1 = Math.max(x1, n.x!); y1 = Math.max(y1, n.y!); }
    const k = Math.min(2, Math.max(0.05, Math.min((w - 120) / Math.max(1, x1 - x0), (h - 120) / Math.max(1, y1 - y0))));
    cam.current = { k, x: w / 2 - ((x0 + x1) / 2) * k, y: h / 2 - ((y0 + y1) / 2) * k };
    setZoom(k);
    render();
    return true;
  };

  const zoomAt = (px: number, py: number, factor: number) => {
    const c = cam.current;
    const k = Math.min(6, Math.max(0.05, c.k * factor));
    cam.current = { k, x: px - ((px - c.x) * k) / c.k, y: py - ((py - c.y) * k) / c.k };
    setZoom(k);
    render();
  };

  // Canvas, simulation and input live as long as the component.
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
      if (!fitted.current) fitted.current = fit();
      render();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el.parentElement!);

    const s = forceSimulation<Node, Link>().alphaDecay(0.02).velocityDecay(0.4).stop();
    s.on("tick", render);
    sim.current = s;

    const point = (e: PointerEvent | WheelEvent) => {
      const r = el.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    const toWorld = (p: { x: number; y: number }) => ({ x: (p.x - cam.current.x) / cam.current.k, y: (p.y - cam.current.y) / cam.current.k });
    const hit = (p: { x: number; y: number }) => {
      const w = toWorld(p);
      const nodes = scene.current.nodes;
      for (let i = nodes.length - 1; i >= 0; i--) {
        const n = nodes[i]!;
        if (Math.hypot(n.x! - w.x, n.y! - w.y) <= n.r + 4 / cam.current.k) return n;
      }
      return null;
    };

    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId);
      const p = point(e);
      const node = hit(p);
      if (node) {
        node.fx = node.x;
        node.fy = node.y;
        drag.current = { node, moved: false };
      } else {
        drag.current = { from: p, cam: cam.current, moved: false };
      }
    };
    const move = (e: PointerEvent) => {
      const p = point(e);
      const d = drag.current;
      if (d && "node" in d) {
        const w = toWorld(p);
        if (!d.moved) s.alphaTarget(0.2).restart();
        d.node.fx = w.x;
        d.node.fy = w.y;
        d.moved = true;
      } else if (d) {
        cam.current = { ...d.cam, x: d.cam.x + p.x - d.from.x, y: d.cam.y + p.y - d.from.y };
        d.moved ||= Math.hypot(p.x - d.from.x, p.y - d.from.y) > 3;
        render();
      } else {
        const id = hit(p)?.id ?? null;
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
        if (!d.moved) select.current(d.node.id);
      } else if (d && !d.moved) {
        select.current(null);
      }
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const p = point(e);
      zoomAt(p.x, p.y, Math.exp(-e.deltaY * 0.0015));
      hover.current = hit(p)?.id ?? null;
    };
    const leave = () => { hover.current = null; render(); };
    const dblclick = (e: MouseEvent) => { if (!hit(point(e as PointerEvent))) fit(); };
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest("input, textarea, select") || e.metaKey || e.ctrlKey) return;
      if (e.key === "z") fit();
    };

    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointerleave", leave);
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("dblclick", dblclick);
    window.addEventListener("keydown", key);
    return () => {
      observer.disconnect();
      s.stop();
      cancelAnimationFrame(frame.current);
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointerleave", leave);
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("dblclick", dblclick);
      window.removeEventListener("keydown", key);
    };
  }, []);

  // Rebuild only when the graph really changes: polling hands back a fresh object every few seconds.
  const graphKey = useMemo(
    () => graph.nodes.map((n) => `${n.id}:${n.status ?? ""}:${n.label}`).join("|") + "#" + graph.edges.map((e) => e.id).join("|"),
    [graph],
  );
  useEffect(() => {
    const s = sim.current;
    if (!s) return;
    const previous = new Map(scene.current.nodes.map((n) => [n.id, n]));
    const next = buildScene(graph, query, orphans, previous);
    const first = scene.current.nodes.length === 0;
    const changed = first || next.nodes.length !== scene.current.nodes.length || next.links.length !== scene.current.links.length;
    scene.current = next;
    s.nodes(next.nodes)
      .force("link", forceLink<Node, Link>(next.links).distance(36).strength(0.7))
      .force("charge", forceManyBody<Node>().strength(-70).distanceMax(500))
      .force("x", forceX<Node>(0).strength(0.045))
      .force("y", forceY<Node>(0).strength(0.045))
      .force("collide", forceCollide<Node>((n) => n.r + 2));
    if (first) {
      // Settle before the first frame so the graph opens calm and framed, then drift to rest.
      s.alpha(1);
      s.tick(300);
      fitted.current = fit();
      s.alpha(0.05).restart();
    } else if (changed) {
      s.alpha(0.3).restart();
    }
    render();
  }, [graphKey, query, orphans]);

  useEffect(render, [selectedId]);

  return (
    <>
      <canvas ref={canvas} className="force-graph" />
      <div className="graph-search">
        <input placeholder="Filter nodes…" value={query} onChange={(e) => setQuery(e.target.value)} />
        <label className="local-toggle">
          <input type="checkbox" checked={orphans} onChange={(e) => setOrphans(e.target.checked)} /> Orphans
        </label>
      </div>
      <div className="graph-camera">
        <button onClick={fit}>Fit <kbd>Z</kbd></button>
        <span className="zoom">
          <button onClick={() => zoomAt(size.current.w / 2, size.current.h / 2, 1 / 1.25)}>−</button>
          {Math.round(zoom * 100)}%
          <button onClick={() => zoomAt(size.current.w / 2, size.current.h / 2, 1.25)}>+</button>
        </span>
      </div>
    </>
  );
}

function draw(ctx: CanvasRenderingContext2D, size: { w: number; h: number }, cam: Camera, scene: Scene, focus: string | null, selected: string | null) {
  const dpr = window.devicePixelRatio || 1;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, size.w, size.h);
  ctx.translate(cam.x, cam.y);
  ctx.scale(cam.k, cam.k);

  const lit = focus ? new Set([focus, ...(scene.neighbours.get(focus) ?? [])]) : null;

  // Lines: one faint grey; the focused node's lines brighten, everything else nearly disappears.
  ctx.lineWidth = 0.8 / cam.k;
  ctx.strokeStyle = lit ? "rgba(150, 158, 172, 0.06)" : LINE;
  ctx.beginPath();
  for (const l of scene.links) {
    if (lit && (l.source.id === focus || l.target.id === focus)) continue;
    ctx.moveTo(l.source.x!, l.source.y!);
    ctx.lineTo(l.target.x!, l.target.y!);
  }
  ctx.stroke();
  if (lit) {
    ctx.strokeStyle = LINE_LIT;
    ctx.lineWidth = 1.1 / cam.k;
    ctx.beginPath();
    for (const l of scene.links) {
      if (l.source.id !== focus && l.target.id !== focus) continue;
      ctx.moveTo(l.source.x!, l.source.y!);
      ctx.lineTo(l.target.x!, l.target.y!);
    }
    ctx.stroke();
  }

  for (const n of scene.nodes) {
    ctx.globalAlpha = !lit || lit.has(n.id) ? 1 : 0.15;
    ctx.beginPath();
    ctx.arc(n.x!, n.y!, n.r, 0, Math.PI * 2);
    if (n.superseded) {
      ctx.strokeStyle = fill(n);
      ctx.lineWidth = 1.2 / cam.k;
      ctx.stroke();
    } else {
      ctx.fillStyle = lit?.has(n.id) && n.id === focus ? "#e3e6eb" : fill(n);
      ctx.fill();
    }
    if (n.id === selected) {
      ctx.strokeStyle = "#e3e6eb";
      ctx.lineWidth = 1.5 / cam.k;
      ctx.beginPath();
      ctx.arc(n.x!, n.y!, n.r + 3 / cam.k, 0, Math.PI * 2);
      ctx.stroke();
    }
  }

  // Labels fade in between 70% and 120% zoom; the focused neighbourhood is always labelled.
  const fade = Math.min(1, Math.max(0, (cam.k - 0.7) / 0.5));
  const px = 11 / cam.k;
  ctx.font = `${px}px Inter, -apple-system, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  const placed: [number, number, number, number][] = [];
  const rank = (n: Node) => (n.id === focus ? 2 : lit?.has(n.id) ? 1 : 0);
  const order = [...scene.nodes].sort((a, b) => rank(b) - rank(a) || b.degree - a.degree);
  for (const n of order) {
    const on = lit?.has(n.id) ?? false;
    const alpha = on ? 1 : lit ? 0 : fade;
    if (alpha < 0.03) continue;
    const text = label(n);
    const w = ctx.measureText(text).width, h = px * 1.2;
    const x = n.x! - w / 2, y = n.y! + n.r + 3 / cam.k;
    if (n.id !== focus && placed.some(([a, b, c, d]) => x < a + c && a < x + w && y < b + d && b < y + h)) continue;
    placed.push([x, y, w, h]);
    ctx.globalAlpha = alpha;
    ctx.fillStyle = on ? TEXT_LIT : TEXT;
    ctx.fillText(text, n.x!, y);
  }
  ctx.globalAlpha = 1;
}
