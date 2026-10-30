import type { Connection } from "./types.ts";

export interface Api {
  get<T>(path: string): Promise<T>;
  post<T>(path: string, body: unknown): Promise<T>;
}

export function createApi({ url, token }: Connection): Api {
  const call = async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const res = await fetch(`${url}${path}`, {
      ...init,
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    });
    if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
    return res.json() as Promise<T>;
  };
  return {
    get: (path) => call(path),
    post: (path, body) => call(path, { method: "POST", body: JSON.stringify(body) }),
  };
}

/** Browser dev mode: `?url=http://127.0.0.1:PORT&token=egm_...` skips the Electron picker. */
export function connectionFromQuery(): Connection | null {
  const q = new URLSearchParams(location.search);
  const url = q.get("url");
  const token = q.get("token");
  return url && token ? { url, token } : null;
}
