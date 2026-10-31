import type { SimLink, SimNode } from "./model.ts";
import type { GraphSettings } from "./settings.ts";

export interface Camera {
  x: number;
  y: number;
  k: number;
}

export interface Focus {
  /** Node under the pointer or selected; its neighbourhood stays bright, the rest dims. */
  id: string | null;
  neighbours: Set<string>;
  selected: string | null;
}

const ACCENT = "#7c97ff";

/** What a line means, by colour (see the legend): rgb, used with an alpha. */
export const EDGE_COLORS: Record<string, [string, string]> = {
  imports: ["109,141,255", "imports"],
  invokes: ["71,195,148", "function calls"],
  calls: ["240,163,94", "HTTP calls"],
  serves: ["240,163,94", "serves an API"],
  "served by": ["240,163,94", "serves an API"],
  uses: ["209,107,165", "uses a service"],
  about: ["177,132,255", "a memory about it"],
  touches: ["177,132,255", "a memory about it"],
};

/**
 * Draw one frame from projected positions (`sx`, `sy`; see projection.ts). Nodes arrive far to near,
 * so nearer ones paint on top. Sizes are divided by zoom where they should stay constant on screen.
 */
export function draw(ctx: CanvasRenderingContext2D, w: number, h: number, cam: Camera, nodes: SimNode[], links: SimLink[], s: GraphSettings, focus: Focus) {
  ctx.save();
  ctx.clearRect(0, 0, w, h);
  ctx.translate(cam.x, cam.y);
  ctx.scale(cam.k, cam.k);
  const lit = (id: string) => !focus.id || focus.neighbours.has(id);

  for (const l of bundle(nodes, links, focus)) {
    const hot = focus.id && (l.source.id === focus.id || l.target.id === focus.id);
    const contains = l.type === "contains";
    const rgb = EDGE_COLORS[l.type]?.[0];
    ctx.strokeStyle = focus.id && !hot ? "rgba(255,255,255,0.04)"
      : rgb ? `rgba(${rgb},${hot ? 0.9 : l.bundled ? 0.3 : 0.42})`
      : hot ? ACCENT : l.type === "supersedes" ? "rgba(177,132,255,0.35)" : contains ? "rgba(200,210,235,0.09)" : "rgba(200,210,235,0.16)";
    // A summed edge (34 calls between two folders) is drawn thicker than a single call.
    // Bundles are capped so a part everything depends on (core) doesn't drown the view.
    const weight = 1 + Math.log2(Math.max(1, l.count));
    ctx.lineWidth = (hot ? 1.6 : 1) * (l.bundled ? Math.min(weight, 4) : weight) * s.linkWidth / cam.k;
    ctx.globalAlpha = Math.min(l.source.fade!, l.target.fade!);
    ctx.setLineDash(l.type === "supersedes" || l.type === "derives" || contains ? [4 / cam.k, 3 / cam.k] : []);
    ctx.beginPath();
    ctx.moveTo(l.source.sx!, l.source.sy!);
    ctx.lineTo(l.target.sx!, l.target.sy!);
    ctx.stroke();
    if (s.arrows) arrow(ctx, l, cam.k, s);
    if (l.bundled && hot) bundleLabel(ctx, l, cam.k, focus.id);
  }
  ctx.setLineDash([]);

  for (const n of nodes) {
    const r = radius(n, s);
    ctx.globalAlpha = (lit(n.id) ? 1 : 0.18) * n.fade!;
    ctx.beginPath();
    if (n.kind === "document" || n.kind === "module" || n.kind === "api" || n.kind === "api_group") roundedSquare(ctx, n.sx!, n.sy!, r * (n.kind === "module" ? 1.25 : 1));
    else ctx.arc(n.sx!, n.sy!, r, 0, Math.PI * 2);
    if (n.expanded) {
      // An opened node is a hub for its children: hollow, so they read as its contents.
      ctx.strokeStyle = n.color;
      ctx.lineWidth = 2.2 / cam.k;
      ctx.stroke();
      ctx.globalAlpha *= 0.25;
      ctx.fillStyle = n.color;
      ctx.fill();
    } else if (n.status === "superseded") {
      ctx.strokeStyle = n.color;
      ctx.lineWidth = 1.5 / cam.k;
      ctx.stroke();
    } else {
      ctx.fillStyle = n.color;
      if (n.id === focus.selected) { ctx.shadowColor = n.color; ctx.shadowBlur = 18; }
      ctx.fill();
      ctx.shadowBlur = 0;
    }
    if (n.children > 0 && !n.expanded) {
      // Can be opened (double-click): a thin outer ring.
      ctx.beginPath();
      ctx.arc(n.sx!, n.sy!, r + 3 / cam.k, 0, Math.PI * 2);
      ctx.strokeStyle = n.color;
      ctx.lineWidth = 1 / cam.k;
      ctx.stroke();
    }
    if (n.id === focus.selected) {
      ctx.beginPath();
      ctx.arc(n.sx!, n.sy!, r, 0, Math.PI * 2);
      ctx.strokeStyle = "#fff";
      ctx.lineWidth = 2 / cam.k;
      ctx.stroke();
    }
  }
  ctx.globalAlpha = 1;
  labels(ctx, cam, nodes, s, focus);
  ctx.restore();
}

type Drawn = SimLink & { bundled?: boolean; calls?: number; imports?: number };

/**
 * Lines between two opened parts are bundled: every import and call between two parts (either way) is
 * drawn as one line between the parts, as thick as how many it stands for: green when code calls
 * across, blue when it only imports. Hovering a part labels its bundles with the counts.
 * Lines inside a part, and the lines of the node under focus, are drawn as they are.
 */
function bundle(nodes: SimNode[], links: SimLink[], focus: Focus): Drawn[] {
  const areas = new Map(nodes.filter((n) => n.kind === "area").map((n) => [n.id, n]));
  const out: Drawn[] = [];
  const bundles = new Map<string, Drawn>();
  for (const l of links) {
    const [a, b] = [l.source.area, l.target.area];
    const between = (l.type === "imports" || l.type === "invokes") && a && b && a !== b && areas.has(a) && areas.has(b) && (l.source.kind !== "area" || l.target.kind !== "area");
    if (!between || l.source.id === focus.id || l.target.id === focus.id) {
      out.push(l);
      continue;
    }
    const key = a < b ? `${a}|${b}` : `${b}|${a}`;
    let found = bundles.get(key);
    if (!found) {
      found = { id: key, type: "imports", count: 0, calls: 0, imports: 0, source: areas.get(a)!, target: areas.get(b)!, bundled: true };
      bundles.set(key, found);
    }
    found.count += l.count;
    if (l.type === "invokes") { found.calls! += l.count; found.type = "invokes"; } else found.imports! += l.count;
  }
  return [...out, ...bundles.values()];
}

function bundleLabel(ctx: CanvasRenderingContext2D, l: Drawn, k: number, from: string | null) {
  // Two thirds of the way out from the hovered part, so its labels fan out instead of piling up.
  const [a, b] = l.source.id === from ? [l.source, l.target] : [l.target, l.source];
  const x = a.sx! + (b.sx! - a.sx!) * 0.65, y = a.sy! + (b.sy! - a.sy!) * 0.65;
  const text = [l.calls ? `${l.calls} calls` : "", l.imports ? `${l.imports} imports` : ""].filter(Boolean).join(" · ");
  ctx.save();
  ctx.font = `${10.5 / k}px Inter, -apple-system, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  const w = ctx.measureText(text).width + 8 / k, h = 15 / k;
  ctx.setLineDash([]);
  ctx.fillStyle = "rgba(14,17,26,0.85)";
  ctx.beginPath();
  ctx.roundRect(x - w / 2, y - h / 2, w, h, h / 2);
  ctx.fill();
  ctx.fillStyle = `rgb(${EDGE_COLORS[l.type]?.[0] ?? "200,210,235"})`;
  ctx.fillText(text, x, y);
  ctx.restore();
}

/** Radius on screen: the node's size times perspective (1 in 2D). */
export const radius = (n: SimNode, s: GraphSettings) => n.radius * s.nodeSize * (n.scale ?? 1);

/** Short label: file nodes show their file name; the full path is in the drawer. */
function shortLabel(n: SimNode): string {
  if ((n.kind === "api" || n.kind === "api_group") && n.children > 0 && !n.expanded) return `${n.label} · ${n.children}`;
  const text = n.kind === "file" ? n.label.slice(n.label.lastIndexOf("/") + 1) : n.label;
  return text.length > 40 ? `${text.slice(0, 38)}…` : text;
}

/**
 * Labels fade in with zoom (Obsidian's "text fade threshold"). Focused labels always show.
 * Overlapping labels are culled greedily, most-connected first, so the view stays readable.
 */
function labels(ctx: CanvasRenderingContext2D, cam: Camera, nodes: SimNode[], s: GraphSettings, focus: Focus) {
  const size = 11.5 / cam.k;
  ctx.font = `${size}px Inter, -apple-system, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  const zoomAlpha = Math.min(1, Math.max(0, (cam.k - s.textFade * 0.6) / (s.textFade * 0.4)));
  const placed: [number, number, number, number][] = [];
  const order = [...nodes].sort((a, b) => Number(focus.neighbours.has(b.id)) - Number(focus.neighbours.has(a.id)) || b.degree - a.degree);
  for (const n of order) {
    const focused = focus.id !== null && focus.neighbours.has(n.id);
    // Parts and services are the landmarks: always labelled.
    const hub = (n.degree >= 8 && cam.k > s.textFade * 0.35) || ["area", "service", "api", "api_group"].includes(n.kind);
    const alpha = (focused ? 1 : focus.id ? 0 : hub ? Math.max(zoomAlpha, 0.85) : zoomAlpha) * (focused ? 1 : n.fade!);
    if (alpha <= 0.02) continue;
    const text = shortLabel(n);
    const w = ctx.measureText(text).width, h = size * 1.2;
    const x = n.sx! - w / 2, y = n.sy! + radius(n, s) + 3 / cam.k;
    if (!focused && placed.some(([px, py, pw, ph]) => x < px + pw && px < x + w && y < py + ph && py < y + h)) continue;
    placed.push([x, y, w, h]);
    ctx.globalAlpha = alpha;
    ctx.fillStyle = n.id === focus.selected ? "#ffffff" : "#c9d1e3";
    ctx.fillText(text, n.sx!, y);
  }
  ctx.globalAlpha = 1;
}

function roundedSquare(ctx: CanvasRenderingContext2D, x: number, y: number, r: number) {
  const s = r * 1.7;
  ctx.roundRect(x - s / 2, y - s / 2, s, s, s * 0.28);
}

function arrow(ctx: CanvasRenderingContext2D, l: SimLink, k: number, s: GraphSettings) {
  const dx = l.target.sx! - l.source.sx!, dy = l.target.sy! - l.source.sy!;
  const len = Math.hypot(dx, dy) || 1;
  const tip = radius(l.target, s) + 2 / k;
  const x = l.target.sx! - (dx / len) * tip, y = l.target.sy! - (dy / len) * tip;
  const a = Math.atan2(dy, dx), size = 6 / k;
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(x - size * Math.cos(a - 0.45), y - size * Math.sin(a - 0.45));
  ctx.lineTo(x - size * Math.cos(a + 0.45), y - size * Math.sin(a + 0.45));
  ctx.closePath();
  ctx.fillStyle = ctx.strokeStyle;
  ctx.fill();
}

/** Camera that fits all nodes in a w×h viewport with padding. */
export function fitCamera(nodes: SimNode[], w: number, h: number, pad = 60): Camera {
  if (nodes.length === 0) return { x: w / 2, y: h / 2, k: 1 };
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const n of nodes) {
    x0 = Math.min(x0, n.sx!); y0 = Math.min(y0, n.sy!); x1 = Math.max(x1, n.sx!); y1 = Math.max(y1, n.sy!);
  }
  const k = Math.min(1.6, Math.max(0.15, Math.min((w - pad * 2) / Math.max(1, x1 - x0), (h - pad * 2) / Math.max(1, y1 - y0))));
  return { k, x: w / 2 - ((x0 + x1) / 2) * k, y: h / 2 - ((y0 + y1) / 2) * k };
}
