use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Python,
    Rust,
    TypeScript,
    /// TSX grammar, also used for JavaScript and JSX.
    Tsx,
}

impl Lang {
    pub fn from_path(path: &str) -> Option<Lang> {
        match path.rsplit_once('.')?.1 {
            "py" => Some(Lang::Python),
            "rs" => Some(Lang::Rust),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Lang::Tsx),
            _ => None,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Qualified within the file: `Fusion.update` (Python/TS), `Store::open` (Rust).
    pub name: String,
    /// function, method, class, struct, enum, trait, interface, type, module, impl.
    pub kind: &'static str,
    /// 1-based, inclusive.
    pub start_line: usize,
    pub end_line: usize,
    /// Directly in the file, not nested in another symbol. Top-level symbols are chunk boundaries.
    pub top_level: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// `from ..pkg.mod import x` → module `pkg.mod`, level 2.
    Python { module: String, level: usize },
    /// `use crate::a::B` → `["crate","a","B"]`; `mod foo;` → `["foo"]` with `is_mod`.
    Rust { path: Vec<String>, is_mod: bool },
    /// `import x from "./y"` → `./y`.
    Js { specifier: String },
}

#[derive(Debug, Default)]
pub struct FileIndex {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
}

pub(crate) fn text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

pub(crate) fn span(node: Node) -> (usize, usize) {
    (node.start_position().row + 1, node.end_position().row + 1)
}

pub(crate) fn children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// Parse one file. Returns None for unsupported languages or unparseable input.
pub fn parse_file(lang: Lang, source: &str) -> Option<FileIndex> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let tree = parser.parse(source, None)?;
    let mut out = FileIndex::default();
    let src = source.as_bytes();
    match lang {
        Lang::Python => crate::python::visit(tree.root_node(), src, None, true, &mut out),
        Lang::Rust => crate::rust::visit(tree.root_node(), src, None, true, &mut out),
        Lang::TypeScript | Lang::Tsx => crate::ts::visit(tree.root_node(), src, None, true, &mut out),
    }
    Some(out)
}
