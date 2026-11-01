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
  /** A clicked node's traced flow: each line and the step it was reached at. */
  path?: Map<string, number>;
  /** And how many steps from the clicked node each traced node is. */
  depth?: Map<string, number>;
}

const ACCENT = "#7c97ff";
/** Traced steps: the familiar blue first, then shifting one step at a time toward amber. */
const STEPS = ["#7c97ff", "#9a8cff", "#b784f2", "#cf7fd6", "#e27fb4", "#ec8f8f", "#f0a35e"];
const stepColor = (step: number) => STEPS[Math.min(step, STEPS.length - 1)]!;

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

  // Lit lines last and the nearest steps on top, so a traced path is never painted over by a dimmer line.
  const step = (l: SimLink) => (focus.path ? focus.path.get(l.id) : focus.id && (l.source.id === focus.id || l.target.id === focus.id) ? 0 : undefined);
  const rank = (l: SimLink) => { const at = step(l); return at === undefined ? -1 : 1000 - at; };
  const ordered = focus.id ? [...links].sort((a, b) => rank(a) - rank(b)) : links;
  for (const l of ordered) {
    const at = step(l);
    const hot = at !== undefined;
    ctx.strokeStyle = hot ? (focus.path ? stepColor(at) : ACCENT) : focus.id ? "rgba(255,255,255,0.04)" : l.type === "supersedes" ? "rgba(177,132,255,0.35)" : "rgba(200,210,235,0.16)";
    ctx.lineWidth = (hot ? 1.6 : 1) * s.linkWidth / cam.k;
    // Lines to the first neighbours are full strength; each step further along the trace is dimmer.
    ctx.globalAlpha = Math.min(l.source.fade!, l.target.fade!) * (hot ? Math.max(0.12, 0.6 ** at) : 1);
    ctx.setLineDash(l.type === "supersedes" || l.type === "derives" || l.type === "mentions" ? [4 / cam.k, 3 / cam.k] : []);
    ctx.beginPath();
    ctx.moveTo(l.source.sx!, l.source.sy!);
    ctx.lineTo(l.target.sx!, l.target.sy!);
    ctx.stroke();
    if (s.arrows) arrow(ctx, l, cam.k, s);
  }
  ctx.setLineDash([]);

  for (const n of nodes) {
    const r = radius(n, s);
    ctx.globalAlpha = (lit(n.id) ? 1 : 0.18) * n.fade!;
    ctx.beginPath();
    if (n.kind === "document" || n.kind === "module") roundedSquare(ctx, n.sx!, n.sy!, r * (n.kind === "module" ? 1.25 : 1));
    else ctx.arc(n.sx!, n.sy!, r, 0, Math.PI * 2);
    if (n.status === "superseded") {
      ctx.strokeStyle = n.color;
      ctx.lineWidth = 1.5 / cam.k;
      ctx.stroke();
    } else {
      ctx.fillStyle = n.color;
      if (n.id === focus.selected) { ctx.shadowColor = n.color; ctx.shadowBlur = 18; }
      ctx.fill();
      ctx.shadowBlur = 0;
    }
    if (n.id === focus.selected) {
      ctx.strokeStyle = "#fff";
      ctx.lineWidth = 2 / cam.k;
      ctx.stroke();
    }
  }
  ctx.globalAlpha = 1;
  labels(ctx, cam, nodes, s, focus);
  ctx.restore();
}

/** Radius on screen: the node's size times perspective (1 in 2D). */
export const radius = (n: SimNode, s: GraphSettings) => n.radius * s.nodeSize * (n.scale ?? 1);

/** Short label: file nodes show their file name; the full path is in the drawer. */
function shortLabel(n: SimNode): string {
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
    // On a traced path, only the clicked node and its first neighbours are labelled outright;
    // deeper nodes are labelled as you zoom in, like the rest of the graph.
    const focused = focus.id !== null && focus.neighbours.has(n.id) && (focus.depth?.get(n.id) ?? 0) <= 1;
    const forced = focused && (!focus.path || n.id === focus.id);
    const hub = n.degree >= 8 && cam.k > s.textFade * 0.35;
    const deeper = !focused && focus.path !== undefined && focus.neighbours.has(n.id);
    const alpha = (focused ? 1 : deeper ? zoomAlpha : focus.id ? 0 : hub ? Math.max(zoomAlpha, 0.85) : zoomAlpha) * (focused ? 1 : n.fade!);
    if (alpha <= 0.02) continue;
    const text = shortLabel(n);
    const w = ctx.measureText(text).width, h = size * 1.2;
    const x = n.sx! - w / 2, y = n.sy! + radius(n, s) + 3 / cam.k;
    if (!forced && placed.some(([px, py, pw, ph]) => x < px + pw && px < x + w && y < py + ph && py < y + h)) continue;
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
