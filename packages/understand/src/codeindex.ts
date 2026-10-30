import { readFile } from "node:fs/promises";
import { join, posix } from "node:path";
import { addEdge, upsertEntity, type Db } from "@engram/core";
import { parsePython, resolveImport } from "./pyindex.ts";

export interface IndexResult {
  files: number;
  symbols: number;
  importEdges: number;
}

const moduleOf = (path: string) => posix.dirname(path) === "." ? "(root)" : posix.dirname(path);

/**
 * Turn Python files into entities and edges:
 *   module ─contains→ file ─contains→ symbol,   file ─imports→ file
 * Re-running is safe: entities are upserted and edges are unique.
 */
export async function indexPython(db: Db, root: string, paths: string[], allFiles: ReadonlySet<string>, commitSha?: string): Promise<IndexResult> {
  const result: IndexResult = { files: 0, symbols: 0, importEdges: 0 };
  for (const path of paths.filter((p) => p.endsWith(".py"))) {
    const source = await readFile(join(root, path), "utf8").catch(() => null);
    if (source === null) continue;
    const parsed = await parsePython(source);
    const file = upsertEntity(db, { kind: "file", qualifiedName: path, path, commitSha });
    const mod = upsertEntity(db, { kind: "module", qualifiedName: moduleOf(path), path: moduleOf(path), commitSha });
    addEdge(db, mod.id, file.id, "contains");
    result.files++;

    for (const s of parsed.symbols) {
      const sym = upsertEntity(db, { kind: s.kind, qualifiedName: `${path}::${s.name}`, path, commitSha });
      addEdge(db, file.id, sym.id, "contains");
      result.symbols++;
    }
    for (const imp of parsed.imports) {
      const target = resolveImport(imp, path, allFiles);
      if (!target || target === path) continue;
      const dst = upsertEntity(db, { kind: "file", qualifiedName: target, path: target, commitSha });
      addEdge(db, file.id, dst.id, "imports", { weight: 0.8 });
      result.importEdges++;
    }
  }
  return result;
}
