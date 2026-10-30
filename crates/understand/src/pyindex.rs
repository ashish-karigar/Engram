use std::collections::HashSet;

use engram_core::{Error, Result};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PySymbol {
    pub name: String,
    /// class, function or method.
    pub kind: &'static str,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyImport {
    /// Dotted module path without leading dots, e.g. "perception.lidar".
    pub module: String,
    /// Number of leading dots for relative imports.
    pub level: usize,
}

#[derive(Debug, Default)]
pub struct PyFileIndex {
    pub symbols: Vec<PySymbol>,
    pub imports: Vec<PyImport>,
}

pub fn parse_python(source: &str) -> Result<PyFileIndex> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).map_err(|e| Error::msg(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| Error::msg("python parse failed"))?;
    let mut out = PyFileIndex::default();
    visit(tree.root_node(), source.as_bytes(), false, &mut out);
    Ok(out)
}

fn text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn visit(node: Node, src: &[u8], in_class: bool, out: &mut PyFileIndex) {
    match node.kind() {
        "class_definition" | "function_definition" => {
            let Some(name) = node.child_by_field_name("name") else { return };
            let is_class = node.kind() == "class_definition";
            let kind = if is_class { "class" } else if in_class { "method" } else { "function" };
            out.symbols.push(PySymbol { name: text(name, src).into(), kind, line: node.start_position().row + 1 });
            if let Some(body) = node.child_by_field_name("body").filter(|_| is_class) {
                visit_children(body, src, true, out);
            }
        }
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let target = if child.kind() == "aliased_import" { child.child_by_field_name("name") } else { Some(child) };
                if let Some(t) = target.filter(|t| t.kind() == "dotted_name") {
                    out.imports.push(PyImport { module: text(t, src).into(), level: 0 });
                }
            }
        }
        "import_from_statement" => {
            if let Some(m) = node.child_by_field_name("module_name") {
                out.imports.push(parse_module_name(text(m, src)));
            }
        }
        _ => visit_children(node, src, in_class, out),
    }
}

fn visit_children(node: Node, src: &[u8], in_class: bool, out: &mut PyFileIndex) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, src, in_class, out);
    }
}

fn parse_module_name(text: &str) -> PyImport {
    let module = text.trim_start_matches('.');
    PyImport { module: module.into(), level: text.len() - module.len() }
}

/// Map an import to a project file, or None for third-party and stdlib modules.
pub fn resolve_import(imp: &PyImport, from_path: &str, files: &HashSet<String>) -> Option<String> {
    let mut parts: Vec<&str> = if imp.level > 0 {
        let dir: Vec<&str> = from_path.split('/').collect();
        dir[..dir.len().saturating_sub(imp.level)].to_vec()
    } else {
        vec![]
    };
    parts.extend(imp.module.split('.').filter(|p| !p.is_empty()));
    let stem = parts.join("/");
    [format!("{stem}.py"), format!("{stem}/__init__.py")].into_iter().find(|c| files.contains(c))
}
