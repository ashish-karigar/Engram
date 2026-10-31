use tree_sitter::Parser;

use crate::extract::DocKind;

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Section path (markdown headings) or symbol name (code).
    pub heading: Option<String>,
    pub content: String,
    /// 1-based, inclusive.
    pub start_line: usize,
    pub end_line: usize,
}

/// Target chunk size. Small enough to pack several into a token budget, large enough to carry meaning.
pub const MAX_CHARS: usize = 1600;
const CODE_WINDOW: usize = 60;
const CODE_OVERLAP: usize = 8;

pub fn chunk(text: &str, kind: &DocKind) -> Vec<Chunk> {
    let chunks = match kind {
        DocKind::Markdown => chunk_markdown(text),
        DocKind::Code(lang) if lang == "python" => chunk_python(text).unwrap_or_else(|| chunk_lines(text, None)),
        DocKind::Code(_) => chunk_lines(text, None),
        DocKind::Text => chunk_paragraphs(text, None, 1),
    };
    chunks.into_iter().filter(|c| !c.content.trim().is_empty()).collect()
}

/// Split at headings, keeping the heading path ("Setup > Sensors") as context for each section.
fn chunk_markdown(text: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    let mut path: Vec<(usize, String)> = vec![];
    let mut out = vec![];
    let mut start = 0;
    let mut heading: Option<String> = None;
    let mut in_fence = false;
    let flush = |out: &mut Vec<Chunk>, from: usize, to: usize, heading: &Option<String>| {
        if from < to {
            out.extend(chunk_paragraphs(&lines[from..to].join("\n"), heading.clone(), from + 1));
        }
    };
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if in_fence || level == 0 || level > 6 || !line[level..].starts_with(' ') {
            continue;
        }
        flush(&mut out, start, i, &heading);
        path.retain(|(l, _)| *l < level);
        path.push((level, line[level..].trim().to_string()));
        heading = Some(path.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" > "));
        start = i;
    }
    flush(&mut out, start, lines.len(), &heading);
    out
}

/// Pack paragraphs up to MAX_CHARS; a single oversized paragraph is split by lines.
fn chunk_paragraphs(text: &str, heading: Option<String>, first_line: usize) -> Vec<Chunk> {
    let mut out = vec![];
    let (mut buf, mut buf_start, mut line_no) = (String::new(), first_line, first_line);
    for line in text.lines() {
        if buf.len() + line.len() > MAX_CHARS && !buf.is_empty() && (line.trim().is_empty() || buf.len() > MAX_CHARS) {
            out.push(Chunk { heading: heading.clone(), content: buf.trim().to_string(), start_line: buf_start, end_line: line_no - 1 });
            buf.clear();
            buf_start = line_no;
        }
        buf.push_str(line);
        buf.push('\n');
        line_no += 1;
    }
    if !buf.trim().is_empty() {
        out.push(Chunk { heading, content: buf.trim().to_string(), start_line: buf_start, end_line: line_no.saturating_sub(1).max(buf_start) });
    }
    out
}

/// Fixed line windows with overlap, for languages without a structural chunker.
fn chunk_lines(text: &str, heading: Option<String>) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = vec![];
    let mut start = 0;
    while start < lines.len() {
        let end = (start + CODE_WINDOW).min(lines.len());
        out.push(Chunk { heading: heading.clone(), content: lines[start..end].join("\n"), start_line: start + 1, end_line: end });
        if end == lines.len() {
            break;
        }
        start = end - CODE_OVERLAP;
    }
    out
}

/// AST-aware: each top-level function or class is one chunk (split further if huge);
/// the code between them (imports, constants) is grouped into its own chunks.
fn chunk_python(text: &str) -> Option<Vec<Chunk>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).ok()?;
    let tree = parser.parse(text, None)?;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = vec![];
    let mut loose_start = 0;
    let mut cursor = tree.root_node().walk();
    for node in tree.root_node().named_children(&mut cursor) {
        let def = match node.kind() {
            "decorated_definition" => node.child_by_field_name("definition"),
            "function_definition" | "class_definition" => Some(node),
            _ => None,
        };
        let Some(def) = def else { continue };
        let (start, end) = (node.start_position().row, node.end_position().row + 1);
        if start > loose_start {
            out.extend(chunk_lines(&lines[loose_start..start].join("\n"), None).into_iter().map(|c| shift(c, loose_start)));
        }
        let name = def.child_by_field_name("name").and_then(|n| n.utf8_text(text.as_bytes()).ok()).unwrap_or("?");
        let body = lines[start..end.min(lines.len())].join("\n");
        let parts = if body.len() > MAX_CHARS * 2 { chunk_lines(&body, None) } else { vec![Chunk { heading: None, content: body, start_line: 1, end_line: end - start }] };
        out.extend(parts.into_iter().map(|c| Chunk { heading: Some(name.to_string()), ..shift(c, start) }));
        loose_start = end;
    }
    if loose_start < lines.len() {
        out.extend(chunk_lines(&lines[loose_start..].join("\n"), None).into_iter().map(|c| shift(c, loose_start)));
    }
    Some(out)
}

fn shift(c: Chunk, offset: usize) -> Chunk {
    Chunk { start_line: c.start_line + offset, end_line: c.end_line + offset, ..c }
}
