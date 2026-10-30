use std::collections::HashSet;

use engram_core::memory::{Memory, NewMemory, SearchOpts, create_memory, search_memories, supersede};
use engram_core::{Connection, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAction {
    Created,
    Superseded,
    Duplicate,
}

impl SaveAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SaveAction::Created => "created",
            SaveAction::Superseded => "superseded",
            SaveAction::Duplicate => "duplicate",
        }
    }
}

pub struct SaveResult {
    pub action: SaveAction,
    pub memory: Memory,
}

fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(String::from).collect()
}

/// Jaccard similarity of word sets: |A∩B| / |A∪B|.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (x, y) = (tokens(a), tokens(b));
    if x.is_empty() && y.is_empty() {
        return 1.0;
    }
    let shared = x.intersection(&y).count() as f64;
    shared / (x.len() as f64 + y.len() as f64 - shared)
}

const SAME_TOPIC: f64 = 0.6;
const SAME_CLAIM: f64 = 0.9;

/// Store a memory while keeping the graph consistent:
/// same topic and claim is a duplicate; same topic with a new claim supersedes; otherwise create.
pub fn save_memory(conn: &Connection, input: NewMemory) -> Result<SaveResult> {
    let opts = SearchOpts { limit: Some(5), ..Default::default() };
    let matched = search_memories(conn, &input.title, &opts)?
        .into_iter()
        .map(|h| h.memory)
        .find(|m| m.kind == input.kind && similarity(&m.title, &input.title) >= SAME_TOPIC);
    match matched {
        None => Ok(SaveResult { action: SaveAction::Created, memory: create_memory(conn, input)? }),
        Some(m) if similarity(&m.claim, &input.claim) >= SAME_CLAIM => Ok(SaveResult { action: SaveAction::Duplicate, memory: m }),
        Some(m) => Ok(SaveResult { action: SaveAction::Superseded, memory: supersede(conn, &m.id, input)? }),
    }
}
