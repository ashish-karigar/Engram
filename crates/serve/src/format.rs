use engram_core::memory::Memory;

fn day(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// Reasons a model gives when it has none ("This is a durable truth about the system"). They cost
/// tokens and tell the agent nothing, so they are left out.
fn is_filler(rationale: &str) -> bool {
    let r = rationale.trim().to_lowercase();
    const FILLER: &[&str] = &["durable truth", "durable fact", "worth remembering", "important to remember", "important for future", "this is a fact", "this is a decision", "this is a preference"];
    r.is_empty() || FILLER.iter().any(|f| r.contains(f))
}

/// One compact, self-describing line per memory. The id is a handle for `expand`.
pub fn format_memory(m: &Memory, reason: Option<&str>) -> String {
    let why = m.rationale.as_deref().filter(|r| !is_filler(r)).map(|r| format!(" Why: {}", r.trim())).unwrap_or_default();
    let when = match &m.valid_to {
        Some(to) => format!("{}→{}", day(&m.valid_from), day(to)),
        None => format!("since {}", day(&m.valid_from)),
    };
    let sha = m.commit_sha.as_deref().map(|s| s.chars().take(7).collect::<String>());
    let tail = [Some(m.id.clone()), Some(when), sha, reason.map(String::from)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    format!("• [{}] {}: {}.{why} ({tail})", m.kind, m.title, m.claim.trim().trim_end_matches('.'))
}

/// One line per document chunk: where it lives and a short snippet. The id opens it with `expand`.
pub fn format_chunk(hit: &engram_rag::ChunkHit) -> String {
    let place = match (&hit.path, hit.start_line, hit.end_line) {
        (Some(p), Some(a), Some(b)) => format!("{p}:{a}-{b}"),
        (Some(p), _, _) => p.clone(),
        _ => hit.title.clone(),
    };
    let section = hit.heading.as_deref().map(|h| format!(" › {h}")).unwrap_or_default();
    let flat: String = hit.content.split_whitespace().collect::<Vec<_>>().join(" ");
    let snippet: String = flat.chars().take(280).collect();
    let more = if flat.chars().count() > 280 { "…" } else { "" };
    format!("• [doc] {}{section} ({place}): {snippet}{more} ({} · {})", hit.title, hit.chunk_id, hit.reason)
}
