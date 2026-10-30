use engram_core::memory::Memory;

fn day(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// One compact, self-describing line per memory. The id is a handle for `expand`.
pub fn format_memory(m: &Memory, reason: Option<&str>) -> String {
    let why = m.rationale.as_deref().map(|r| format!(" Why: {r}")).unwrap_or_default();
    let when = match &m.valid_to {
        Some(to) => format!("{}→{}", day(&m.valid_from), day(to)),
        None => format!("since {}", day(&m.valid_from)),
    };
    let sha = m.commit_sha.as_deref().map(|s| s.chars().take(7).collect::<String>());
    let tail = [Some(m.id.clone()), Some(when), sha, reason.map(String::from)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    format!("• [{}] {}: {}.{why} ({tail})", m.kind, m.title, m.claim)
}
