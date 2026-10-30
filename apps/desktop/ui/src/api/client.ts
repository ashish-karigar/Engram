import type {
  ConnectResult, Connection, EventEntry, Graph, NodeDetail, OpResult, Project, RequestEntry, Stats,
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
});

/** Everything the UI reads or does inside one project. */
export const projectApi = (c: Connection, id: string) => {
  const base = `/p/${id}/api`;
  return {
    stats: () => call<Stats>(c, `${base}/stats`),
    graph: (archived: boolean) => call<Graph>(c, `${base}/graph?archived=${archived ? 1 : 0}`),
    node: (nodeId: string) => call<NodeDetail>(c, `${base}/node?id=${encodeURIComponent(nodeId)}`),
    events: (limit = 80) => call<EventEntry[]>(c, `${base}/events?limit=${limit}`),
    requests: (limit = 80) => call<RequestEntry[]>(c, `${base}/requests?limit=${limit}`),
    search: (query: string) => call<OpResult>(c, `${base}/search`, post({ query })),
    connect: (agent: string, apply: boolean) => call<ConnectResult>(c, `${base}/connect`, post({ agent, apply })),
  };
};

export type ProjectApi = ReturnType<typeof projectApi>;
