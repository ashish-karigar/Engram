use engram_core::db::open_memory;
use engram_core::events::list_events;
use engram_core::graph::{ExpandOpts, expand, upsert_entity};
use engram_embed::HashEmbedder;
use engram_rag::chunk::chunk;
use engram_rag::document::{delete_document, list_documents};
use engram_rag::extract::{DocKind, extract, html_to_text};
use engram_rag::project::{ingest_file, should_index};
use engram_rag::{DocumentInput, IngestAction, ingest, search_chunks};
use serde_json::json;

fn input(id: &str, name: &str, body: &str) -> DocumentInput {
    DocumentInput {
        external_id: Some(id.into()),
        title: name.into(),
        source: "upload".into(),
        path: None,
        file_name: name.into(),
        bytes: body.as_bytes().to_vec(),
        metadata: json!({}),
    }
}

const DESIGN: &str = "# Perception design\n\nIntro text.\n\n## Lidar\n\nWe downsample the point cloud with a 0.2m voxel grid in lidar.py.\nRadius outlier removal was too slow at 80ms per frame.\n\n```\n# not a heading\n```\n\n### Ground removal\n\nRANSAC plane fit before clustering.\n\n## Camera\n\nYOLO detections are projected onto lidar clusters.\n";

#[test]
fn markdown_chunks_follow_headings_and_keep_the_section_path() {
    let chunks = chunk(DESIGN, &DocKind::Markdown);
    let headings: Vec<_> = chunks.iter().map(|c| c.heading.clone().unwrap_or_default()).collect();
    assert_eq!(headings, ["Perception design", "Perception design > Lidar", "Perception design > Lidar > Ground removal", "Perception design > Camera"]);
    assert!(chunks[1].content.contains("# not a heading"), "fenced code is not split");
    assert_eq!(chunks[3].start_line, 18);
}

#[test]
fn python_chunks_are_one_per_top_level_symbol() {
    let src = "import numpy as np\n\nVOXEL = 0.2\n\ndef voxel_filter(points):\n    return points\n\n@dataclass\nclass Fusion:\n    def update(self):\n        pass\n";
    let chunks = chunk(src, &DocKind::Code("py".into()));
    let names: Vec<_> = chunks.iter().map(|c| c.heading.clone().unwrap_or_else(|| "(module)".into())).collect();
    assert_eq!(names, ["(module)", "voxel_filter", "Fusion"]);
    assert_eq!((chunks[2].start_line, chunks[2].end_line), (8, 11), "decorators stay with their class");
}

#[test]
fn long_plain_text_is_packed_under_the_size_limit() {
    let text = (0..200).map(|i| format!("Paragraph {i} about sensors.\n")).collect::<String>();
    let chunks = chunk(&text, &DocKind::Text);
    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|c| c.content.len() <= engram_rag::chunk::MAX_CHARS + 100));
}

#[test]
fn html_is_cleaned_and_headings_survive() {
    let text = html_to_text("<html><style>x{}</style><h2>Auth</h2><p>Use <b>sessions</b> &amp; cookies</p><script>evil()</script></html>");
    assert!(text.contains("## Auth") && text.contains("Use sessions & cookies"));
    assert!(!text.contains("evil") && !text.contains("<b>"));
    assert_eq!(extract("page.html", b"<h1>T</h1>").unwrap().kind, DocKind::Markdown);
    assert!(extract("x.bin.pdf", b"not a pdf").is_err());
}

#[test]
fn ingest_creates_skips_unchanged_and_updates_changed_documents() {
    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    let first = ingest(&db, &e, input("design", "design.md", DESIGN)).unwrap();
    assert_eq!(first.action, IngestAction::Created);
    assert_eq!(first.document.status, "done");
    assert_eq!(first.document.chunk_count, 4);

    assert_eq!(ingest(&db, &e, input("design", "design.md", DESIGN)).unwrap().action, IngestAction::Unchanged);
    let changed = ingest(&db, &e, input("design", "design.md", "# Perception\n\nNow 0.1m voxels.")).unwrap();
    assert_eq!(changed.action, IngestAction::Updated);
    assert_eq!(changed.document.version, 2);
    assert_eq!(changed.document.chunk_count, 1);
    let vectors: i64 = db.query_row("SELECT COUNT(*) FROM embeddings WHERE kind = 'chunk'", [], |r| r.get(0)).unwrap();
    assert_eq!(vectors, 1, "old chunk vectors are removed on update");
    assert_eq!(list_events(&db, 0, Some("document.ingested"), 10).unwrap().len(), 2);
}

#[test]
fn bad_input_marks_the_document_failed_instead_of_erroring() {
    let db = open_memory().unwrap();
    let out = ingest(&db, &HashEmbedder::default(), input("broken", "scan.pdf", "garbage")).unwrap();
    assert_eq!(out.action, IngestAction::Failed);
    assert_eq!(out.document.status, "failed");
    assert!(out.document.error.is_some());
}

#[test]
fn hybrid_search_finds_sections_by_keyword_and_by_meaning() {
    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    ingest(&db, &e, input("design", "design.md", DESIGN)).unwrap();
    ingest(&db, &e, input("billing", "billing.md", "# Billing\n\nStripe webhooks are verified before charging.")).unwrap();

    let hits = search_chunks(&db, Some(&e), "why not radius outlier removal", 3).unwrap();
    assert_eq!(hits[0].heading.as_deref(), Some("Perception design > Lidar"));
    assert!(hits[0].reason.contains("keyword"));

    let fuzzy = search_chunks(&db, Some(&e), "voxelization downsampling", 3).unwrap();
    assert_eq!(fuzzy[0].heading.as_deref(), Some("Perception design > Lidar"), "sub-word match via vectors");

    assert!(search_chunks(&db, Some(&e), "stripe", 3).unwrap()[0].title == "billing.md");
    assert!(search_chunks(&db, None, "zzzz", 3).unwrap().is_empty());
}

#[test]
fn documents_link_to_the_code_they_talk_about() {
    let db = open_memory().unwrap();
    let lidar = upsert_entity(&db, "file", "perception/lidar.py", Some("perception/lidar.py"), None).unwrap();
    let out = ingest(&db, &HashEmbedder::default(), input("design", "design.md", DESIGN)).unwrap();
    let near = expand(&db, &[out.document.id.clone()], &ExpandOpts { max_depth: 1, ..Default::default() }).unwrap();
    assert!(near.iter().any(|n| n.id == lidar.id), "design.md mentions lidar.py");

    delete_document(&db, &out.document.id).unwrap();
    assert!(list_documents(&db, None, 10).unwrap().is_empty());
    let chunks: i64 = db.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).unwrap();
    assert_eq!(chunks, 0);
}

#[test]
fn project_files_are_filtered_and_tracked_by_path() {
    assert!(should_index("docs/design.md") && should_index("perception/lidar.py"));
    assert!(!should_index("node_modules/x/readme.md") && !should_index("Cargo.lock") && !should_index("logo.png"));

    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("README.md"), "# Demo\n\nSelf-driving stack.").unwrap();
    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    let out = ingest_file(&db, &e, root.path(), "README.md", engram_rag::project::MAX_FILE_BYTES).unwrap().unwrap();
    assert_eq!(out.document.source, "project");
    std::fs::remove_file(root.path().join("README.md")).unwrap();
    assert!(ingest_file(&db, &e, root.path(), "README.md", engram_rag::project::MAX_FILE_BYTES).unwrap().is_none());
    assert!(list_documents(&db, None, 10).unwrap().is_empty(), "deleted files leave the index");
}

#[test]
fn rust_and_tsx_chunks_follow_top_level_items() {
    let rs = "use std::fmt;\n\npub struct Store;\n\nimpl Store {\n    pub fn open() {}\n}\n\npub fn run() {}\n";
    let heads: Vec<_> = chunk(rs, &DocKind::Code("rs".into())).into_iter().map(|c| c.heading.unwrap_or_else(|| "(module)".into())).collect();
    assert_eq!(heads, ["(module)", "Store", "impl Store", "run"]);
    let tsx = "import React from 'react';\n\nexport function App() {\n  return <div/>;\n}\n\nexport const Panel = () => <p/>;\n";
    let heads: Vec<_> = chunk(tsx, &DocKind::Code("tsx".into())).into_iter().map(|c| c.heading.unwrap_or_else(|| "(module)".into())).collect();
    assert_eq!(heads, ["(module)", "App", "Panel"]);
}
