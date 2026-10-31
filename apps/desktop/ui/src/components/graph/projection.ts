import type { SimNode } from "./model.ts";

/** Rotation of the layout: yaw turns around the vertical axis, pitch tilts toward the viewer. */
export interface View {
  yaw: number;
  pitch: number;
}

/** Distance from the eye to the layout's centre, in layout units. Larger means flatter perspective. */
const EYE = 1800;

/**
 * Rotate each node, then apply perspective, writing screen-plane coordinates (`sx`, `sy`),
 * depth (larger is farther), size `scale` and a depth `fade`. In 2D everything is identity,
 * so the flat graph draws exactly as before.
 */
export function project(nodes: SimNode[], v: View, flat: boolean): void {
  const cy = Math.cos(v.yaw), sy = Math.sin(v.yaw), cp = Math.cos(v.pitch), sp = Math.sin(v.pitch);
  for (const n of nodes) {
    if (flat) {
      n.sx = n.x!; n.sy = n.y!; n.depth = 0; n.scale = 1; n.fade = 1;
      continue;
    }
    const x = n.x!, y = n.y!, z = n.z ?? 0;
    const x1 = x * cy + z * sy, z1 = -x * sy + z * cy;
    const y2 = y * cp - z1 * sp, z2 = y * sp + z1 * cp;
    const f = EYE / Math.max(EYE * 0.25, EYE + z2);
    n.sx = x1 * f; n.sy = y2 * f; n.depth = z2; n.scale = f;
    n.fade = Math.min(1, Math.max(0.35, 0.35 + (f - 0.7) * 1.6));
  }
}

/** Layout position under a screen-plane point, at the given depth (used to drag nodes in 3D). */
export function unproject(sx: number, sy: number, depth: number, v: View): { x: number; y: number; z: number } {
  const f = EYE / Math.max(EYE * 0.25, EYE + depth);
  const x1 = sx / f, y2 = sy / f;
  const cy = Math.cos(v.yaw), syaw = Math.sin(v.yaw), cp = Math.cos(v.pitch), sp = Math.sin(v.pitch);
  const y = y2 * cp + depth * sp, z1 = -y2 * sp + depth * cp;
  return { x: x1 * cy - z1 * syaw, y, z: x1 * syaw + z1 * cy };
}
