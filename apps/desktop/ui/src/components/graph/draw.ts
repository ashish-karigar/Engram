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

/** Draw one frame. Sizes are divided by zoom where they should stay constant on screen. */
export function draw(ctx: CanvasRenderingContext2D, w: number, h: number, cam: Camera, nodes: SimNode[], links: SimLink[], s: GraphSettings, focus: Focus) {
  ctx.save();
  ctx.clearRect(0, 0, w, h);
  ctx.translate(cam.x, cam.y);
  ctx.scale(cam.k, cam.k);
  const lit = (id: string) => !focus.id || focus.neighbours.has(id);

  for (const l of links) {
    const hot = focus.id && (l.source.id === focus.id || l.target.id === focus.id);
    ctx.strokeStyle = hot ? ACCENT : focus.id ? "rgba(255,255,255,0.04)" : l.type === "supersedes" ? "rgba(177,132,255,0.35)" : "rgba(200,210,235,0.16)";
    ctx.lineWidth = (hot ? 1.6 : 1) * s.linkWidth / cam.k;
    ctx.setLineDash(l.type === "supersedes" || l.type === "derives" ? [4 / cam.k, 3 / cam.k] : []);
    ctx.beginPath();
    ctx.moveTo(l.source.x!, l.source.y!);
    ctx.lineTo(l.target.x!, l.target.y!);
    ctx.stroke();
    if (s.arrows) arrow(ctx, l, cam.k, s);
  }
  ctx.setLineDash([]);

  for (const n of nodes) {
    const r = n.radius * s.nodeSize;
    ctx.globalAlpha = lit(n.id) ? 1 : 0.18;
    ctx.beginPath();
    if (n.kind === "document" || n.kind === "module") roundedSquare(ctx, n.x!, n.y!, r * (n.kind === "module" ? 1.25 : 1));
    else ctx.arc(n.x!, n.y!, r, 0, Math.PI * 2);
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

/** Labels fade in with zoom (Obsidian's "text fade threshold"); focused ones always show. */
function labels(ctx: CanvasRenderingContext2D, cam: Camera, nodes: SimNode[], s: GraphSettings, focus: Focus) {
  const size = 11.5 / cam.k;
  ctx.font = `${size}px Inter, -apple-system, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "top";
  for (const n of nodes) {
    const focused = focus.id !== null && focus.neighbours.has(n.id);
    const zoomAlpha = Math.min(1, Math.max(0, (cam.k - s.textFade * 0.6) / (s.textFade * 0.4)));
    const alpha = focused ? 1 : focus.id ? 0 : n.degree >= 6 ? Math.max(zoomAlpha, 0.85) : zoomAlpha;
    if (alpha <= 0.02) continue;
    ctx.globalAlpha = alpha;
    ctx.fillStyle = n.id === focus.selected ? "#ffffff" : "#c9d1e3";
    const text = n.label.length > 42 ? `${n.label.slice(0, 40)}…` : n.label;
    ctx.fillText(text, n.x!, n.y! + n.radius * s.nodeSize + 3 / cam.k);
  }
  ctx.globalAlpha = 1;
}

function roundedSquare(ctx: CanvasRenderingContext2D, x: number, y: number, r: number) {
  const s = r * 1.7;
  ctx.roundRect(x - s / 2, y - s / 2, s, s, s * 0.28);
}

function arrow(ctx: CanvasRenderingContext2D, l: SimLink, k: number, s: GraphSettings) {
  const dx = l.target.x! - l.source.x!, dy = l.target.y! - l.source.y!;
  const len = Math.hypot(dx, dy) || 1;
  const tip = l.target.radius * s.nodeSize + 2 / k;
  const x = l.target.x! - (dx / len) * tip, y = l.target.y! - (dy / len) * tip;
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
    x0 = Math.min(x0, n.x!); y0 = Math.min(y0, n.y!); x1 = Math.max(x1, n.x!); y1 = Math.max(y1, n.y!);
  }
  const k = Math.min(1.6, Math.max(0.15, Math.min((w - pad * 2) / Math.max(1, x1 - x0), (h - pad * 2) / Math.max(1, y1 - y0))));
  return { k, x: w / 2 - ((x0 + x1) / 2) * k, y: h / 2 - ((y0 + y1) / 2) * k };
}
