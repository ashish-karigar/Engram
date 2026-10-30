import type { StoredEvent } from "@engram/core";

export interface Task {
  events: StoredEvent[];
  startedAt: string;
  endedAt: string;
}

const MS = (iso: string) => Date.parse(iso);

/**
 * Sessionize time-ordered events: start a new task after an idle gap.
 * Single pass, O(n). Events must be sorted by seq, which the log guarantees.
 */
export function groupTasks(events: readonly StoredEvent[], idleGapMs = 10 * 60_000): Task[] {
  const tasks: Task[] = [];
  let current: StoredEvent[] = [];
  const close = () => {
    const first = current[0];
    const last = current.at(-1);
    if (first && last) tasks.push({ events: current, startedAt: first.ts, endedAt: last.ts });
    current = [];
  };
  for (const e of events) {
    const prev = current.at(-1);
    if (prev && MS(e.ts) - MS(prev.ts) > idleGapMs) close();
    current.push(e);
  }
  close();
  return tasks;
}
