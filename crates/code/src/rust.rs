use tree_sitter::Node;

use crate::lang::{FileIndex, Import, Symbol, children, span, text};

pub(crate) fn visit(node: Node, src: &[u8], owner: Option<&str>, top: bool, out: &mut FileIndex) {
    let named = |kind: &'static str, out: &mut FileIndex| {
        if let Some(n) = node.child_by_field_name("name") {
            let (start_line, end_line) = span(node);
            let name = owner.map_or(text(n, src).to_string(), |o| format!("{o}::{}", text(n, src)));
            out.symbols.push(Symbol { name, kind, start_line, end_line, top_level: top, signature: String::new() });
        }
    };
    match node.kind() {
        "function_item" | "function_signature_item" => named(if owner.is_some() { "method" } else { "function" }, out),
        "struct_item" => named("struct", out),
        "enum_item" => named("enum", out),
        "union_item" => named("struct", out),
        "type_item" => named("type", out),
        "trait_item" => {
            named("trait", out);
            let trait_name = node.child_by_field_name("name").map(|n| text(n, src).to_string());
            if let Some(body) = node.child_by_field_name("body") {
                children(body).into_iter().for_each(|c| visit(c, src, trait_name.as_deref(), false, out));
            }
        }
        "impl_item" => {
            let ty = node.child_by_field_name("type").map(|t| text(t, src).to_string()).unwrap_or_default();
            let (start_line, end_line) = span(node);
            out.symbols.push(Symbol { name: format!("impl {ty}"), kind: "impl", start_line, end_line, top_level: top, signature: String::new() });
            let base = ty.split('<').next().unwrap_or(&ty).trim().to_string();
            if let Some(body) = node.child_by_field_name("body") {
                children(body).into_iter().for_each(|c| visit(c, src, Some(&base), false, out));
            }
        }
        "mod_item" => match node.child_by_field_name("body") {
            Some(body) => {
                named("module", out);
                children(body).into_iter().for_each(|c| visit(c, src, None, false, out));
            }
            None => {
                if let Some(n) = node.child_by_field_name("name") {
                    out.imports.push(Import::Rust { path: vec![text(n, src).to_string()], is_mod: true });
                }
            }
        },
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                for path in use_paths(arg, src, &[]) {
                    out.imports.push(Import::Rust { path, is_mod: false });
                }
            }
        }
        _ => children(node).into_iter().for_each(|c| visit(c, src, owner, top, out)),
    }
}

fn split(path: &str) -> Vec<String> {
    path.split("::").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Expand a `use` tree into full paths: `a::{b, c::D}` → `[a,b]`, `[a,c,D]`.
fn use_paths(node: Node, src: &[u8], prefix: &[String]) -> Vec<Vec<String>> {
    let join = |rest: Vec<String>| [prefix.to_vec(), rest].concat();
    match node.kind() {
        "scoped_use_list" => {
            let base = node.child_by_field_name("path").map(|p| split(text(p, src))).unwrap_or_default();
            let next = join(base);
            node.child_by_field_name("list").map(|l| use_paths(l, src, &next)).unwrap_or_default()
        }
        "use_list" => children(node).into_iter().flat_map(|c| use_paths(c, src, prefix)).collect(),
        "use_as_clause" => node.child_by_field_name("path").map(|p| vec![join(split(text(p, src)))]).unwrap_or_default(),
        "use_wildcard" => vec![join(split(text(node, src).trim_end_matches('*').trim_end_matches("::")))],
        "self" => vec![prefix.to_vec()],
        _ => vec![join(split(text(node, src)))],
    }
}
