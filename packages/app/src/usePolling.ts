import { useCallback, useEffect, useRef, useState } from "react";

/** Fetch now (and whenever `deps` change), then every `ms`. Keeps the previous value on error so the UI does not flicker. */
export function usePolling<T>(load: () => Promise<T>, ms: number, deps: readonly unknown[] = []): { data: T | undefined; error: string | null; reload(): void } {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string | null>(null);
  const loadRef = useRef(load);
  loadRef.current = load;

  const reload = useCallback(() => {
    loadRef.current().then(
      (value) => {
        setData(value);
        setError(null);
      },
      (err: unknown) => setError(err instanceof Error ? err.message : String(err)),
    );
  }, []);

  useEffect(() => {
    reload();
    const timer = setInterval(reload, ms);
    return () => clearInterval(timer);
  }, [reload, ms, ...deps]);

  return { data, error, reload };
}
