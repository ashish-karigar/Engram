export interface Connection {
  url: string;
  token: string;
}

export interface Stats {
  events: number;
  memories: number;
  archived: number;
  entities: number;
  requests: number;
  documents: number;
  chunks: number;
  inferred: number;
}

export interface DocumentView {
  id: string;
  externalId: string;
  title: string;
  source: string;
  path: string | null;
  mime: string;
  status: string;
  error: string | null;
  charCount: number;
  chunkCount: number;
  version: number;
  updatedAt: string;
}

export interface ChunkPreview {
  id: string;
  heading: string | null;
  startLine: number | null;
  endLine: number | null;
  preview: string;
}

export interface InferredMemory {
  memory: MemoryView;
  parentCount: number;
}

export type SearchMode = "hybrid" | "memories" | "documents";

export interface Project {
  id: string;
  name: string;
  root: string;
  stats: Stats | null;
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
  isInference: boolean;
  reinforcement: number;
  expiresAt: string | null;
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
  | { type: "document"; document: DocumentView; chunks: ChunkPreview[]; focus?: string }
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

export interface OpResult {
  text: string;
  data: unknown;
}

export interface ConnectResult {
  agent: string;
  preview: string;
  result: string | null;
}
