import { createRequire } from "node:module";
import { Language, Parser, type Node } from "web-tree-sitter";

export interface PySymbol {
  name: string;
  kind: "class" | "function" | "method";
  line: number;
}

export interface PyImport {
  /** Dotted module path without leading dots, e.g. "perception.lidar". */
  module: string;
  /** Number of leading dots for relative imports. */
  level: number;
}

export interface PyFileIndex {
  symbols: PySymbol[];
  imports: PyImport[];
}

let parser: Parser | undefined;

async function getParser(): Promise<Parser> {
  if (parser) return parser;
  await Parser.init();
  const wasm = createRequire(import.meta.url).resolve("tree-sitter-python/tree-sitter-python.wasm");
  parser = new Parser();
  parser.setLanguage(await Language.load(wasm));
  return parser;
}

export async function parsePython(source: string): Promise<PyFileIndex> {
  const tree = (await getParser()).parse(source);
  const result: PyFileIndex = { symbols: [], imports: [] };
  if (tree) visit(tree.rootNode, result, false);
  return result;
}

function visit(node: Node, out: PyFileIndex, inClass: boolean): void {
  switch (node.type) {
    case "class_definition":
    case "function_definition": {
      const name = node.childForFieldName("name")?.text;
      if (name) {
        const isClass = node.type === "class_definition";
        out.symbols.push({ name, kind: isClass ? "class" : inClass ? "method" : "function", line: node.startPosition.row + 1 });
        const body = node.childForFieldName("body");
        if (body && isClass) visitChildren(body, out, true);
      }
      return;
    }
    case "import_statement":
      for (const child of node.namedChildren) {
        const target = child.type === "aliased_import" ? child.childForFieldName("name") : child;
        if (target?.type === "dotted_name") out.imports.push({ module: target.text, level: 0 });
      }
      return;
    case "import_from_statement": {
      const m = node.childForFieldName("module_name");
      if (m) out.imports.push(parseModuleName(m.text));
      return;
    }
    default:
      visitChildren(node, out, inClass);
  }
}

function visitChildren(node: Node, out: PyFileIndex, inClass: boolean): void {
  for (const child of node.namedChildren) visit(child, out, inClass);
}

function parseModuleName(text: string): PyImport {
  const level = text.length - text.replace(/^\.+/, "").length;
  return { module: text.slice(level), level };
}

/** Map an import to a project file, or null for third-party/stdlib modules. */
export function resolveImport(imp: PyImport, fromPath: string, files: ReadonlySet<string>): string | null {
  const parts = imp.module ? imp.module.split(".") : [];
  const base = imp.level > 0 ? fromPath.split("/").slice(0, -imp.level) : [];
  const stem = [...base, ...parts].join("/");
  for (const candidate of [`${stem}.py`, `${stem}/__init__.py`]) {
    if (files.has(candidate)) return candidate;
  }
  return null;
}
