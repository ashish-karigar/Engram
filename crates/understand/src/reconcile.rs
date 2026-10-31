use std::collections::HashSet;

use engram_core::graph::{EdgeOpts, add_edge};
use engram_core::memory::{Memory, NewMemory, SearchOpts, create_memory, reinforce, search_memories, supersede};
use engram_core::{Connection, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAction {
    Created,
    Superseded,
    /// The same fact again: counted, not duplicated.
    Reinforced,
}

impl SaveAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SaveAction::Created => "created",
            SaveAction::Superseded => "superseded",
            SaveAction::Reinforced => "reinforced",
        }
    }
}

pub struct SaveResult {
    pub action: SaveAction,
    pub memory: Memory,
    /// The related memory this one adds detail to, if any (an `extends` edge was created).
    pub extends: Option<String>,
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
const RELATED_TOPIC: f64 = 0.25;
const SAME_CLAIM: f64 = 0.9;

/// Store a memory while keeping the graph consistent, following Supermemory's relationships:
/// - same topic, same claim → reinforce the existing memory;
/// - same topic, new claim → **updates** (supersede, history kept);
/// - related topic → create and link with **extends**;
/// - otherwise create.
pub fn save_memory(conn: &Connection, input: NewMemory) -> Result<SaveResult> {
    let opts = SearchOpts { limit: Some(8), ..Default::default() };
    let query = format!("{} {}", input.title, input.claim);
    let candidates: Vec<Memory> =
        search_memories(conn, &query, &opts)?.into_iter().map(|h| h.memory).filter(|m| m.kind == input.kind).collect();

    if let Some(m) = candidates.iter().find(|m| similarity(&m.title, &input.title) >= SAME_TOPIC) {
        return if similarity(&m.claim, &input.claim) >= SAME_CLAIM {
            Ok(SaveResult { action: SaveAction::Reinforced, memory: reinforce(conn, &m.id)?, extends: None })
        } else {
            Ok(SaveResult { action: SaveAction::Superseded, memory: supersede(conn, &m.id, input)?, extends: None })
        };
    }

    let related = candidates
        .iter()
        .map(|m| (similarity(&format!("{} {}", m.title, m.claim), &query), m))
        .filter(|(s, _)| *s >= RELATED_TOPIC)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, m)| m.id.clone());
    let memory = create_memory(conn, input)?;
    if let Some(parent) = &related {
        add_edge(conn, &memory.id, parent, "extends", EdgeOpts { weight: Some(0.9), ..Default::default() })?;
    }
    Ok(SaveResult { action: SaveAction::Created, memory, extends: related })
}
