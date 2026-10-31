use tree_sitter::Node;

use crate::lang::{FileIndex, Import, Symbol, children, span, text};

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string()
}

fn push(out: &mut FileIndex, name: String, kind: &'static str, at: Node, top: bool) {
    let (start_line, end_line) = span(at);
    out.symbols.push(Symbol { name, kind, start_line, end_line, top_level: top, signature: String::new() });
}

/// `outer` spans are used for top-level symbols so `export` stays inside the chunk.
pub(crate) fn visit(node: Node, src: &[u8], class: Option<&str>, top: bool, out: &mut FileIndex) {
    visit_with(node, node, src, class, top, out)
}

fn visit_with(node: Node, outer: Node, src: &[u8], class: Option<&str>, top: bool, out: &mut FileIndex) {
    let name = || node.child_by_field_name("name").map(|n| text(n, src).to_string());
    match node.kind() {
        "import_statement" => {
            if let Some(s) = node.child_by_field_name("source") {
                out.imports.push(Import::Js { specifier: unquote(text(s, src)) });
            }
        }
        "export_statement" => {
            if let Some(s) = node.child_by_field_name("source") {
                out.imports.push(Import::Js { specifier: unquote(text(s, src)) });
            }
            for child in children(node) {
                visit_with(child, node, src, class, top, out);
            }
        }
        "call_expression" => {
            let callee = node.child_by_field_name("function").map(|f| text(f, src)).unwrap_or("");
            if callee == "require" || callee == "import" {
                if let Some(arg) = node.child_by_field_name("arguments").and_then(|a| children(a).into_iter().next()).filter(|a| a.kind() == "string") {
                    out.imports.push(Import::Js { specifier: unquote(text(arg, src)) });
                }
            }
            children(node).into_iter().for_each(|c| visit(c, src, class, false, out));
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(n) = name() {
                push(out, n, "function", outer, top);
            }
        }
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(n) = name() {
                push(out, n.clone(), "class", outer, top);
                if let Some(body) = node.child_by_field_name("body") {
                    children(body).into_iter().for_each(|c| visit(c, src, Some(&n), false, out));
                }
            }
        }
        "method_definition" => {
            if let (Some(n), Some(c)) = (name(), class) {
                push(out, format!("{c}.{n}"), "method", node, false);
            }
        }
        "interface_declaration" => name().into_iter().for_each(|n| push(out, n, "interface", outer, top)),
        "type_alias_declaration" => name().into_iter().for_each(|n| push(out, n, "type", outer, top)),
        "enum_declaration" => name().into_iter().for_each(|n| push(out, n, "enum", outer, top)),
        "lexical_declaration" | "variable_declaration" if top => {
            // `const Foo = (...) => ...` and `const Foo = function ...` are functions (React components too).
            for decl in children(node).into_iter().filter(|d| d.kind() == "variable_declarator") {
                let is_fn = decl.child_by_field_name("value").is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression" | "function"));
                if let (true, Some(n)) = (is_fn, decl.child_by_field_name("name")) {
                    push(out, text(n, src).to_string(), "function", outer, top);
                }
                children(decl).into_iter().for_each(|c| visit(c, src, class, false, out));
            }
        }
        _ => children(node).into_iter().for_each(|c| visit(c, src, class, top && node.kind() == "program", out)),
    }
}
