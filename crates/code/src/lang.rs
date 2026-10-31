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
    /// The declaration line, e.g. `def update(self, job_id: int) -> JobOut:` (filled after parsing).
    pub signature: String,
}

/// A call inside a function: `this.jobs.updateStatus(id)` → callee `updateStatus`, receiver `this.jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub callee: String,
    /// What it is called on: `self`, `this.jobs`, `JobsApi`, `crate::db` (a path for Rust); None for a bare call.
    pub receiver: Option<String>,
    pub line: usize,
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
    pub calls: Vec<CallSite>,
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
    let lines: Vec<&str> = source.lines().collect();
    for s in &mut out.symbols {
        s.signature = signature(&lines, s.start_line, s.end_line);
    }
    collect_calls(lang, tree.root_node(), src, &mut out.calls);
    Some(out)
}

/// The first line of a symbol that is not a decorator, attribute, doc comment or blank.
fn signature(lines: &[&str], start: usize, end: usize) -> String {
    let skip = |l: &str| {
        let t = l.trim_start();
        t.is_empty() || t.starts_with('@') || t.starts_with("#[") || t.starts_with("//") || t.starts_with("///") || t.starts_with('#')
    };
    let line = (start..=end.min(lines.len())).filter_map(|n| lines.get(n - 1)).find(|l| !skip(l)).copied().unwrap_or("");
    let t = line.trim();
    if t.chars().count() > 200 { format!("{}…", t.chars().take(200).collect::<String>()) } else { t.to_string() }
}

/// Every function call in the file, with its callee name and what it is called on.
fn collect_calls(lang: Lang, node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let kind = node.kind();
    let call = match lang {
        Lang::Python => kind == "call",
        _ => kind == "call_expression",
    };
    if call {
        if let Some(f) = node.child_by_field_name("function") {
            let line = node.start_position().row + 1;
            let field = |n: Node, name: &str| n.child_by_field_name(name).map(|x| text(x, src).to_string());
            let site = match (lang, f.kind()) {
                (_, "identifier") => Some((text(f, src).to_string(), None)),
                (Lang::Python, "attribute") => field(f, "attribute").map(|a| (a, field(f, "object"))),
                (Lang::TypeScript | Lang::Tsx, "member_expression") => field(f, "property").map(|p| (p, field(f, "object"))),
                (Lang::Rust, "field_expression") => field(f, "field").map(|p| (p, field(f, "value"))),
                (Lang::Rust, "scoped_identifier") => field(f, "name").map(|n| (n, field(f, "path"))),
                _ => None,
            };
            if let Some((callee, receiver)) = site {
                let receiver = receiver.map(|r| r.chars().take(80).collect::<String>());
                out.push(CallSite { callee, receiver, line });
            }
        }
    }
    for c in children(node) {
        collect_calls(lang, c, src, out);
    }
}
