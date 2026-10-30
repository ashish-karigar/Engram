import type { Db } from "./db.ts";

export function getMeta(db: Db, key: string): string | undefined {
  const r = db.prepare("SELECT value FROM meta WHERE key = ?").get(key) as { value: string } | undefined;
  return r?.value;
}

export function setMeta(db: Db, key: string, value: string): void {
  db.prepare("INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(
    key,
    value,
  );
}
