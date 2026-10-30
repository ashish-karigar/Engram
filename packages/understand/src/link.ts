import { upsertEntity, type Db } from "@engram/core";

/** Resolve file paths mentioned by a memory to file entities. Unknown paths are ignored unless allowed. */
export function linkFiles(db: Db, files: readonly string[], knownPaths: ReadonlySet<string>): string[] {
  const ids: string[] = [];
  for (const path of new Set(files.map((f) => f.replace(/^\.\//, "")))) {
    const exists = db.prepare("SELECT id FROM entities WHERE kind = 'file' AND qualified_name = ?").get(path) as
      | { id: string }
      | undefined;
    if (exists) ids.push(exists.id);
    else if (knownPaths.has(path)) ids.push(upsertEntity(db, { kind: "file", qualifiedName: path, path }).id);
  }
  return ids;
}
