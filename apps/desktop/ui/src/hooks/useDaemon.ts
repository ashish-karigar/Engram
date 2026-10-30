import { useEffect, useState } from "react";
import type { Connection } from "../api/types.ts";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

/** Inside the desktop app the daemon runs in-process; in a browser, pass ?url=…&token=… */
async function discover(): Promise<Connection> {
  if ((window as TauriWindow).__TAURI_INTERNALS__) {
    const { invoke } = await import("@tauri-apps/api/core");
    const info = await invoke<{ url: string; adminToken: string }>("daemon_info");
    return { url: info.url, token: info.adminToken };
  }
  const q = new URLSearchParams(location.search);
  const url = q.get("url");
  const token = q.get("token");
  if (!url || !token) throw new Error("Open with ?url=<daemon>&token=<admin token>, or run the desktop app.");
  return { url, token };
}

export function useDaemon(): { connection: Connection | null; error: string | null } {
  const [connection, setConnection] = useState<Connection | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    discover().then(setConnection, (e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  return { connection, error };
}

/** Native folder picker in the desktop app; a text prompt in a browser. */
export async function pickFolder(): Promise<string | null> {
  if ((window as TauriWindow).__TAURI_INTERNALS__) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  }
  return window.prompt("Absolute path of the project folder:");
}
