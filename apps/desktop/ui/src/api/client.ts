import type {
  ActivityPage, AppSettings, PendingLink, ImpactResult, ConnectResult, Connection, TestResult, DocumentView, EventEntry, Graph, InferredMemory, MemoryView, WeakMemory, NodeDetail, OpResult, Project,
  RequestEntry, SearchMode, Stats,
} from "./types.ts";

async function call<T>({ url, token }: Connection, path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(`${url}${path}`, {
    ...init,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
  });
  const body = await res.json().catch(() => null);
  if (!res.ok) throw new Error(body?.error ?? `${res.status}`);
  return body as T;
}

const post = (body: unknown): RequestInit => ({ method: "POST", body: JSON.stringify(body) });

export const projectsApi = (c: Connection) => ({
  list: () => call<Project[]>(c, "/api/projects"),
  add: (root: string) => call<Project>(c, "/api/projects", post({ root })),
  remove: (id: string) => call<{ ok: boolean }>(c, `/api/projects/${id}`, { method: "DELETE" }),
  settings: () => call<AppSettings>(c, "/api/settings"),
  saveSettings: (s: AppSettings) => call<AppSettings>(c, "/api/settings", post(s)),
  testSettings: (target: "llm" | "embeddings", settings?: AppSettings) => call<TestResult>(c, "/api/settings/test", post({ target, settings })),
  /** Agents connect once, to every project. Without `apply` this only reads their status. */
  connect: (agent: string, apply: boolean) => call<ConnectResult>(c, "/api/connect", post({ agent, apply })),
});

/** Everything the UI reads or does inside one project. */
export const projectApi = (c: Connection, id: string) => {
  const base = `/p/${id}/api`;
  return {
    stats: () => call<Stats>(c, `${base}/stats`),
    graph: (archived: boolean) => call<Graph>(c, `${base}/graph?archived=${archived ? 1 : 0}`),
    architecture: () => call<Graph>(c, `${base}/architecture`),
    zoom: (expanded: string[]) => call<Graph>(c, `${base}/zoom?expanded=${expanded.map(encodeURIComponent).join(",")}`),
    map: (area?: string) => call<OpResult>(c, `${base}/map${area ? `?area=${encodeURIComponent(area)}` : ""}`),
    impact: (target: string) => call<{ text: string; data: ImpactResult }>(c, `${base}/impact`, post({ target, limit: 60 })),
    node: (nodeId: string) => call<NodeDetail>(c, `${base}/node?id=${encodeURIComponent(nodeId)}`),
    events: (limit = 80) => call<EventEntry[]>(c, `${base}/events?limit=${limit}`),
    activity: (opts: { before?: string; system?: boolean; days?: number } = {}) =>
      call<ActivityPage>(c, `${base}/activity?days=${opts.days ?? 3}&system=${opts.system ? 1 : 0}${opts.before ? `&before=${encodeURIComponent(opts.before)}` : ""}`),
    requests: (limit = 80) => call<RequestEntry[]>(c, `${base}/requests?limit=${limit}`),
    search: (query: string, mode: SearchMode = "hybrid") => call<OpResult>(c, `${base}/search`, post({ query, mode })),
    profile: () => call<OpResult>(c, `${base}/profile`),
    documents: (limit = 500) => call<DocumentView[]>(c, `${base}/documents?limit=${limit}`),
    addDocument: (title: string, content: string) => call<OpResult>(c, `${base}/documents`, post({ title, content })),
    deleteDocument: (id: string) => call<{ ok: boolean }>(c, `${base}/documents/delete`, post({ id })),
    review: () => call<InferredMemory[]>(c, `${base}/review`),
    weak: () => call<WeakMemory[]>(c, `${base}/weak`),
    links: () => call<PendingLink[]>(c, `${base}/links`),
    retiredLinks: () => call<PendingLink[]>(c, `${base}/links/retired`),
    decideLink: (id: string, action: "approve" | "decline" | "keep" | "undo", keep?: string) => call<{ ok: boolean }>(c, `${base}/links`, post({ id, action, keep })),
    archive: (ids: string[], restore = false) => call<{ changed: number }>(c, `${base}/memories/archive`, post({ ids, restore })),
    decide: (id: string, action: "approve" | "decline" | "undo") => call<MemoryView>(c, `${base}/review`, post({ id, action })),
  };
};

export type ProjectApi = ReturnType<typeof projectApi>;
