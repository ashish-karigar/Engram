import type { ActivityItem } from "../../api/types.ts";
import type { Branch } from "./lanes.ts";

const LANE = 16;
const X0 = 11;
/** Where the row's dot sits, from the top of its content. */
export const DOT_Y = 16;

export const laneX = (lane: number) => X0 + lane * LANE;
export const gutterWidth = (lanes: number) => X0 * 2 + (lanes - 1) * LANE;

export const agentColor = (agent: string) => `var(--a-${agent}, var(--muted))`;

function dotStyle(item: ActivityItem): { color: string; shape: "ring" | "dot" | "diamond" | "small" } {
  switch (item.kind) {
    case "session": return { color: agentColor(item.agent), shape: "dot" };
    case "commit": return { color: "var(--text)", shape: "ring" };
    case "memory": return { color: `var(--t-${item.type}, var(--accent))`, shape: "diamond" };
    default: return { color: "var(--faint)", shape: "small" };
  }
}

interface Props {
  item: ActivityItem;
  lane: number;
  lanes: number;
  /** Extra space above the dot (the day header sits there). */
  offset: number;
  passing: Branch[];
  forking: Branch[];
  own?: Branch;
}

/**
 * The thread line beside one row: the main line, the lanes of sessions running past this row,
 * a curve where a session branches off (its first older row) or merges back (its own row), and the dot.
 */
export function Gutter({ item, lane, lanes, offset, passing, forking, own }: Props) {
  const y = offset + DOT_Y;
  const x0 = laneX(0);
  const { color, shape } = dotStyle(item);
  return (
    <div className="tl-gutter" style={{ width: gutterWidth(lanes) }}>
      <span className="tl-rail" style={{ left: x0 }} />
      {passing.map((b) => <span key={b.row} className="tl-rail lane" style={{ left: laneX(b.lane), background: agentColor(b.agent) }} />)}
      {own && <span className="tl-rail lane" style={{ left: laneX(own.lane), top: y, background: agentColor(own.agent) }} />}
      <svg className="tl-curves" width={gutterWidth(lanes)} height={y + 1}>
        {own && <path d={`M${x0} 0 C${x0} ${y * 0.6} ${laneX(own.lane)} ${y * 0.4} ${laneX(own.lane)} ${y}`} stroke={agentColor(own.agent)} />}
        {forking.map((b) => (
          <path key={b.row} d={`M${laneX(b.lane)} 0 C${laneX(b.lane)} ${y * 0.6} ${x0} ${y * 0.4} ${x0} ${y}`} stroke={agentColor(b.agent)} />
        ))}
      </svg>
      <span className={`tl-dot ${shape}`} style={{ left: laneX(lane), top: y, color }} />
    </div>
  );
}
