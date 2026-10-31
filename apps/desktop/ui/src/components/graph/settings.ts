/** Graph view preferences. Stored per viewer in localStorage, like Obsidian's graph settings. */
export interface GraphSettings {
  mode: "2d" | "3d";
  search: string;
  hidden: string[];
  showOrphans: boolean;
  showArchived: boolean;
  local: boolean;
  localDepth: number;
  nodeSize: number;
  linkWidth: number;
  textFade: number;
  arrows: boolean;
  centerForce: number;
  repel: number;
  linkForce: number;
  linkDistance: number;
}

export const DEFAULTS: GraphSettings = {
  mode: "2d",
  search: "",
  hidden: [],
  showOrphans: true,
  showArchived: false,
  local: false,
  localDepth: 1,
  nodeSize: 1,
  linkWidth: 1,
  textFade: 0.9,
  arrows: false,
  centerForce: 0.2,
  repel: 32,
  linkForce: 0.6,
  linkDistance: 70,
};

/** Node groups, in legend order. Colors come from the `--t-<kind>` CSS tokens. */
export const GROUPS: { kind: string; label: string }[] = [
  { kind: "decision", label: "Decisions" },
  { kind: "rejected_option", label: "Rejected options" },
  { kind: "dead_end", label: "Dead ends" },
  { kind: "fact", label: "Facts" },
  { kind: "preference", label: "Preferences" },
  { kind: "episode", label: "Episodes" },
  { kind: "task", label: "Tasks" },
  { kind: "document", label: "Documents" },
  { kind: "module", label: "Modules" },
  { kind: "file", label: "Files" },
  { kind: "api", label: "API" },
];

const KEY = "engram.graph.settings.v4";

export function loadSettings(): GraphSettings {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return DEFAULTS;
  }
}

export function saveSettings(s: GraphSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* private window or blocked storage: settings just don't persist */
  }
}
