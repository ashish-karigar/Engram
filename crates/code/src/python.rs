use tree_sitter::Node;

use crate::lang::{FileIndex, Import, Symbol, children, span, text};

pub(crate) fn visit(node: Node, src: &[u8], class: Option<&str>, top: bool, out: &mut FileIndex) {
    match node.kind() {
        "decorated_definition" => {
            if let Some(def) = node.child_by_field_name("definition") {
                define(def, node, src, class, top, out);
            }
        }
        "class_definition" | "function_definition" => define(node, node, src, class, top, out),
        "import_statement" => {
            for child in children(node) {
                let target = if child.kind() == "aliased_import" { child.child_by_field_name("name") } else { Some(child) };
                if let Some(t) = target.filter(|t| t.kind() == "dotted_name") {
                    out.imports.push(Import::Python { module: text(t, src).into(), level: 0 });
                }
            }
        }
        "import_from_statement" => {
            if let Some(m) = node.child_by_field_name("module_name") {
                let raw = text(m, src);
                let module = raw.trim_start_matches('.');
                out.imports.push(Import::Python { module: module.into(), level: raw.len() - module.len() });
            }
        }
        _ => children(node).into_iter().for_each(|c| visit(c, src, class, top, out)),
    }
}

/// `def`/`class`; `outer` is the decorated wrapper when present, so decorators stay in the span.
fn define(def: Node, outer: Node, src: &[u8], class: Option<&str>, top: bool, out: &mut FileIndex) {
    let Some(name) = def.child_by_field_name("name").map(|n| text(n, src)) else { return };
    let is_class = def.kind() == "class_definition";
    let kind = if is_class { "class" } else if class.is_some() { "method" } else { "function" };
    let full = class.map_or(name.to_string(), |c| format!("{c}.{name}"));
    let (start_line, end_line) = span(outer);
    out.symbols.push(Symbol { name: full.clone(), kind, start_line, end_line, top_level: top });
    if let Some(body) = def.child_by_field_name("body").filter(|_| is_class) {
        children(body).into_iter().for_each(|c| visit(c, src, Some(&full), false, out));
    }
}
