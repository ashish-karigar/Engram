use std::sync::LazyLock;

use engram_core::{Error, Result};
use regex::Regex;

/// How a document should be chunked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocKind {
    Markdown,
    /// Source code, tagged with the file extension so the chunker can pick a parser.
    Code(String),
    Text,
}

pub struct Extracted {
    pub text: String,
    pub mime: String,
    pub kind: DocKind,
}

const CODE: &[(&str, &str)] = &[
    ("py", "python"), ("rs", "rust"), ("ts", "typescript"), ("tsx", "typescript"), ("mts", "typescript"), ("js", "javascript"),
    ("jsx", "javascript"), ("mjs", "javascript"), ("cjs", "javascript"), ("go", "go"), ("java", "java"), ("c", "c"), ("h", "c"), ("cpp", "cpp"),
    ("hpp", "cpp"), ("cs", "csharp"), ("rb", "ruby"), ("swift", "swift"), ("kt", "kotlin"), ("sh", "shell"),
    ("sql", "sql"),
];
const TEXT: &[&str] = &["txt", "rst", "adoc", "json", "yaml", "yml", "toml", "ini", "cfg", "csv", "xml"];
const MARKDOWN: &[&str] = &["md", "markdown", "mdx"];

pub fn extension(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

pub fn is_code(path: &str) -> bool {
    let ext = extension(path);
    CODE.iter().any(|(e, _)| *e == ext)
}

/// Whether Engram knows how to read this file.
pub fn is_supported(path: &str) -> bool {
    let ext = extension(path);
    MARKDOWN.contains(&ext.as_str())
        || TEXT.contains(&ext.as_str())
        || CODE.iter().any(|(e, _)| *e == ext)
        || matches!(ext.as_str(), "html" | "htm" | "pdf")
}

static HIDDEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>|<noscript[^>]*>.*?</noscript>").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static BLOCKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?(p|div|br|li|tr|h[1-6]|section|article)[^>]*>").unwrap());
static HEADINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>").unwrap());

/// HTML to markdown-ish text: headings become `#` lines so the markdown chunker can use them.
pub fn html_to_text(html: &str) -> String {
    let visible = HIDDEN.replace_all(html, "");
    let with_headings = HEADINGS.replace_all(&visible, |c: &regex::Captures| format!("\n{} {}\n", "#".repeat(c[1].parse().unwrap_or(1)), &c[2]));
    let with_breaks = BLOCKS.replace_all(&with_headings, "\n");
    let text = TAGS.replace_all(&with_breaks, "");
    let decoded = text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    decoded.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").split("\n\n\n").collect::<Vec<_>>().join("\n\n")
}

/// Turn raw bytes into text plus a hint for the chunker. `path` only supplies the extension.
pub fn extract(path: &str, bytes: &[u8]) -> Result<Extracted> {
    let ext = extension(path);
    if ext == "pdf" {
        let text = pdf_extract::extract_text_from_mem(bytes).map_err(|e| Error::msg(format!("pdf: {e}")))?;
        return Ok(Extracted { text, mime: "application/pdf".into(), kind: DocKind::Text });
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| Error::msg(format!("{path} is not UTF-8 text")))?;
    if matches!(ext.as_str(), "html" | "htm") {
        return Ok(Extracted { text: html_to_text(&text), mime: "text/html".into(), kind: DocKind::Markdown });
    }
    if MARKDOWN.contains(&ext.as_str()) {
        return Ok(Extracted { text, mime: "text/markdown".into(), kind: DocKind::Markdown });
    }
    if let Some((_, lang)) = CODE.iter().find(|(e, _)| *e == ext) {
        return Ok(Extracted { text, mime: format!("text/x-{lang}"), kind: DocKind::Code(ext) });
    }
    Ok(Extracted { text, mime: "text/plain".into(), kind: DocKind::Text })
}
