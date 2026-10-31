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

export interface WeakMemory {
  memory: MemoryView;
  reason: "changelog" | "duplicate" | "identifier" | "low_confidence";
  detail: string;
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
  /** Areas: folder and file count. */
  path?: string;
  files?: number;
  /** Zoomable map: how many children it can open into, whether it is open, and its part. */
  children?: number;
  /** The API node: for each calling file, the files that serve what it calls. */
  routes?: Record<string, string[]>;
  expanded?: boolean;
  area?: string;
  serves?: string;
  hub?: string;
  endpoints?: number;
  /** Who wrote most of it, from provenance. */
  owner?: { who: string; share: number; lines: number };
}

export interface Provenance {
  lines: number;
  contributors: { who: string; lines: number; isAgent: boolean; evidence: string }[];
  created: { sha: string; ts: string | null; who: string; subject: string } | null;
  lastChanged: { sha: string; ts: string | null; who: string; subject: string } | null;
  commits: number;
  sessions: { agent: string; session: string; documentId: string | null }[];
}

export interface ImpactResult {
  target: { id: string; kind: string; name: string; path: string | null };
  affected: { file: string; area: string | null; depth: number; why: string }[];
  endpoints: string[];
  decisions: [string, string, string][];
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
  /** Zoomable map: how many fine-grained edges this one stands for. */
  count?: number;
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
  | { type: "entity"; entity: { id: string; kind: string; qualifiedName: string; path: string | null }; signature?: string | null; provenance?: Provenance }
  | { type: "document"; document: DocumentView; chunks: ChunkPreview[]; focus?: string }
  | null;

export interface SessionMessage {
  ts: string;
  role: "user" | "assistant" | "compacted";
  text?: string;
}

interface Span {
  start: string;
  end: string;
}

/** One entry of the activity timeline; see crates/serve/src/activity.rs. */
export type ActivityItem = Span & (
  | { kind: "session"; agent: string; session: string; title: string; messages: SessionMessage[]; userMessages: number; agentMessages: number; toolCalls: number; compactions: number; files: string[] }
  | { kind: "commit"; sha: string; subject: string; author: string | null; files: string[]; fileCount: number }
  | { kind: "edits"; files: string[] }
  | { kind: "memory"; id: string; type: string; title: string; status: string; author: string | null; replaces: string[] }
  | { kind: "system"; counts: Record<string, number>; titles: string[] }
);

export interface ActivityPage {
  items: ActivityItem[];
  from: string;
  to: string;
  hiddenSystem: number;
  earlier: boolean;
}

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

export interface AppSettings {
  llm: { provider: "none" | "meta" | "ollama"; metaBaseUrl: string; metaModel: string; metaApiKey: string | null; ollamaHost: string; ollamaModel: string };
  embeddings: { provider: "hash" | "local" | "ollama" | "openai"; localModel: string; ollamaHost: string; ollamaModel: string; openaiBaseUrl: string; openaiModel: string; openaiApiKey: string | null };
  indexing: { sessions: boolean; indexDocs: boolean; indexCode: boolean; maxFileKb: number; ignore: string[] };
  memory: { extractEverySecs: number; decayEveryHours: number; snapshotEverySecs: number };
  dataDir?: string;
}

export interface TestResult {
  ok: boolean;
  detail: string;
  latencyMs: number;
}
