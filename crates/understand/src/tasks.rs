use chrono::DateTime;
use engram_core::events::StoredEvent;

pub struct Task {
    pub events: Vec<StoredEvent>,
    pub started_at: String,
    pub ended_at: String,
}

fn millis(ts: &str) -> i64 {
    DateTime::parse_from_rfc3339(ts).map(|d| d.timestamp_millis()).unwrap_or(0)
}

/// Sessionize time-ordered events: start a new task after an idle gap.
/// Single pass, O(n). Events must be ordered by seq, which the log guarantees.
pub fn group_tasks(events: &[StoredEvent], idle_gap_ms: i64) -> Vec<Task> {
    let mut tasks: Vec<Task> = vec![];
    let mut current: Vec<StoredEvent> = vec![];
    let mut close = |current: &mut Vec<StoredEvent>| {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            tasks.push(Task { started_at: first.ts.clone(), ended_at: last.ts.clone(), events: std::mem::take(current) });
        }
    };
    for e in events {
        if let Some(prev) = current.last() {
            if millis(&e.ts) - millis(&prev.ts) > idle_gap_ms {
                close(&mut current);
            }
        }
        current.push(e.clone());
    }
    close(&mut current);
    tasks
}
