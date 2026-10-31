import type { ActivityItem } from "../../api/types.ts";

/**
 * Git-graph layout for the timeline. Lane 0 is the project's main line (commits, edits, memories).
 * Each agent session branches into its own lane for as long as it runs; sessions that overlap in
 * time (agents working in parallel) get different lanes. Rows are newest first, so a session's
 * row (at its end) is where it merges back, and the first older row is where it branched off.
 */
export interface Row {
  key: string;
  item: ActivityItem;
  lane: number;
  /** Row time: when the item ended. */
  t: number;
}

export interface Branch {
  lane: number;
  agent: string;
  start: number;
  /** Index of the session's own row (the merge point). */
  row: number;
}

export interface Layout {
  rows: Row[];
  branches: Branch[];
  lanes: number;
}

const ms = (iso: string) => new Date(iso).getTime();

export function itemKey(i: ActivityItem): string {
  switch (i.kind) {
    case "session": return `s:${i.session}:${i.start}`;
    case "commit": return `c:${i.sha}`;
    case "memory": return `m:${i.id}`;
    default: return `${i.kind}:${i.start}`;
  }
}

/** Interval colouring: each session takes the lowest lane that is free when it starts. */
export function layout(items: ActivityItem[]): Layout {
  const sorted = [...items].sort((a, b) => ms(b.end) - ms(a.end));
  const laneOf = new Map<string, number>();
  const busyUntil: number[] = [];
  for (const s of [...sorted].filter((i) => i.kind === "session").sort((a, b) => ms(a.start) - ms(b.start))) {
    let lane = busyUntil.findIndex((end) => end < ms(s.start));
    if (lane === -1) lane = busyUntil.length;
    busyUntil[lane] = ms(s.end);
    laneOf.set(itemKey(s), lane + 1);
  }
  const rows = sorted.map((item) => ({ key: itemKey(item), item, lane: laneOf.get(itemKey(item)) ?? 0, t: ms(item.end) }));
  const branches = rows.flatMap((r, row) =>
    r.item.kind === "session" ? [{ lane: r.lane, agent: r.item.agent, start: ms(r.item.start), row }] : []);
  return { rows, branches, lanes: busyUntil.length + 1 };
}

/** What the gutter draws for row `i`: lanes passing through, and a branch that forks off at its top. */
export function gutterAt(l: Layout, i: number) {
  const t = l.rows[i]!.t;
  const passing = l.branches.filter((b) => b.row < i && b.start < t);
  const forking = l.branches.filter((b) => b.row < i && b.start >= t && !l.rows.slice(b.row + 1, i).some((r) => r.t <= b.start));
  const own = l.branches.find((b) => b.row === i);
  return { passing, forking, own };
}
