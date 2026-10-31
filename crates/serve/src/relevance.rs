//! One relevance scale for memories and document chunks.
//!
//! Each ranker (memory recall, chunk search) normalizes to its own best hit, so "top memory" and
//! "top chunk" both look perfect even when one is barely related. Coverage fixes the scale: the
//! share of the query's distinctive words an item contains, each weighted by how rare it is
//! across memories and chunks (IDF). "port" in "what port does the daemon use" outweighs
//! "daemon", which half the project mentions.

use engram_core::{Connection, Result};

const STOPWORDS: &[&str] = &[
    "about", "after", "also", "and", "any", "are", "can", "could", "did", "does", "each", "for", "from", "has", "have", "how",
    "into", "its", "may", "not", "our", "should", "that", "the", "their", "them", "then", "there", "these", "this", "use",
    "used", "using", "was", "way", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// Words are compared by their first few letters after common endings are dropped, so "deploy",
/// "deployed" and "deployment" match, and so do "rankings" and "rankers".
const STEM: usize = 5;
const SUFFIXES: &[&str] = &["ings", "ing", "ers", "er", "ed", "es", "s"];

pub struct Vocabulary {
    words: Vec<(String, f64)>,
}

fn stem(word: &str) -> String {
    let root = SUFFIXES.iter().find_map(|s| word.strip_suffix(s).filter(|r| r.chars().count() >= 4)).unwrap_or(word);
    root.chars().take(STEM).collect()
}

/// Distinct content words of the query with their IDF over the project's memories and chunks.
pub fn vocabulary(conn: &Connection, query: &str) -> Result<Vocabulary> {
    let mut stems: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(w))
        .map(stem)
        .collect();
    stems.sort_unstable();
    stems.dedup();
    let total: f64 = conn.query_row("SELECT (SELECT COUNT(*) FROM memories WHERE status = 'current') + (SELECT COUNT(*) FROM chunks)", [], |r| r.get(0))?;
    let mut words = Vec::with_capacity(stems.len());
    for s in stems {
        let like = format!("%{s}%");
        let df: f64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM memories WHERE status = 'current' AND lower(title || ' ' || claim || ' ' || COALESCE(rationale, '')) LIKE ?1)
                  + (SELECT COUNT(*) FROM chunks WHERE lower(context || ' ' || content) LIKE ?1)",
            [&like],
            |r| r.get(0),
        )?;
        // Smoothed so weights stay positive in a new, nearly empty project.
        words.push((s, (1.0 + (total + 1.0) / (1.0 + df)).ln()));
    }
    Ok(Vocabulary { words })
}

impl Vocabulary {
    /// Share (0..=1) of the query's IDF weight present in `text`. 0 when the query has no content words.
    pub fn coverage(&self, text: &str) -> f64 {
        let total: f64 = self.words.iter().map(|(_, w)| w).sum();
        if total == 0.0 {
            return 0.0;
        }
        let text = text.to_lowercase();
        self.words.iter().filter(|(s, _)| text.contains(s.as_str())).map(|(_, w)| w).sum::<f64>() / total
    }

    /// The `chars`-long stretch of `text`, starting at a line or sentence, that covers the most of
    /// the query, so a snippet shows the answer rather than the start of the chunk.
    pub fn best_window<'t>(&self, text: &'t str, chars: usize) -> &'t str {
        let starts = std::iter::once(0).chain(text.match_indices(['\n', '.']).map(|(i, _)| i + 1)).filter(|&i| i < text.len());
        let window = |start: usize| {
            let end = text[start..].char_indices().nth(chars).map_or(text.len(), |(i, _)| start + i);
            &text[start..end]
        };
        let mut best = (window(0), self.coverage(window(0)));
        for start in starts {
            let w = window(start);
            let c = self.coverage(w);
            if c > best.1 + 1e-9 {
                best = (w, c);
            }
        }
        best.0.trim_start()
    }
}

/// Weight of coverage against the ranker's own (per-list) score.
const COVERAGE_WEIGHT: f64 = 0.6;

pub fn blend(coverage: f64, rank: f64) -> f64 {
    COVERAGE_WEIGHT * coverage + (1.0 - COVERAGE_WEIGHT) * rank
}

/// Keep results until the score falls well below the best one: a short answer when one item
/// clearly answers the question, more when several are about equally relevant.
pub const RELATIVE_CUTOFF: f64 = 0.7;
pub const ABSOLUTE_FLOOR: f64 = 0.15;
pub const MAX_RESULTS: usize = 6;

pub fn cut<T>(mut scored: Vec<(T, f64)>) -> Vec<(T, f64)> {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = scored.first().map_or(0.0, |s| s.1);
    scored.into_iter().take_while(|(_, s)| *s >= best * RELATIVE_CUTOFF && *s >= ABSOLUTE_FLOOR).take(MAX_RESULTS).collect()
}
