export interface Connection {
  url: string;
  token: string;
  root?: string;
}

export interface GraphNode {
  id: string;
  label: string;
  kind: string;
  status?: string;
  isHub?: boolean;
}
export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
}
export interface Graph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface Stats {
  events: number;
  memories: number;
  archived: number;
  entities: number;
  requests: number;
}

export interface MemoryView {
  id: string;
  type: string;
  title: string;
  claim: string;
  rationale: string | null;
  status: string;
  version: number;
  validFrom: string;
  validTo: string | null;
  commitSha: string | null;
  confidence: number;
}

export interface EvidenceEvent {
  seq: number;
  ts: string;
  kind: string;
  payload: unknown;
}

export type NodeDetail =
  | { type: "memory"; memory: MemoryView; history: MemoryView[]; evidence: EvidenceEvent[] }
  | { type: "entity"; entity: { id: string; kind: string; qualifiedName: string; path: string | null } }
  | null;

export interface EventEntry {
  seq: number;
  ts: string;
  kind: string;
  payload: unknown;
}

export interface RequestEntry {
  id: number;
  ts: string;
  actor: string | null;
  tool: string;
  request: unknown;
  returned: { id: string; score?: number; reason?: string }[];
  tokensOut: number | null;
  latencyMs: number | null;
  error: string | null;
}

export interface ConnectResult {
  agent: string;
  preview: string;
  result: string | null;
}

declare global {
  interface Window {
    engram?: { pickProject(): Promise<Connection | null> };
  }
}
