use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::db::now;
use crate::error::Result;
use crate::memory::{Memory, get_memory, set_status_at};

const DAY_SECS: f64 = 86_400.0;
const ARCHIVE_BELOW: f64 = 0.1;

/// Half-life in days per memory type. `None` means it persists until superseded.
/// Facts and decisions are replaced only by newer evidence; episodes and attempts fade.
fn half_life_days(kind: &str) -> Option<f64> {
    match kind {
        "decision" | "fact" => None,
        "preference" => Some(180.0),
        "rejected_option" => Some(120.0),
        "dead_end" => Some(60.0),
        "episode" => Some(14.0),
        "task" => Some(7.0),
        _ => Some(30.0),
    }
}

fn parse(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|d| d.with_timezone(&Utc))
}

/// Recency-decayed salience, boosted by use so frequently recalled memories resist fading.
pub fn strength(m: &Memory, at: DateTime<Utc>) -> f64 {
    let last = m.last_accessed_at.as_deref().and_then(parse).or_else(|| parse(&m.observed_at)).unwrap_or(at);
    let age_days = ((at - last).num_seconds() as f64 / DAY_SECS).max(0.0);
    let decay = half_life_days(&m.kind).map_or(1.0, |half| 0.5_f64.powf(age_days / half));
    m.salience * decay + (m.access_count as f64).ln_1p() * 0.05 + (m.reinforcement as f64).ln_1p() * 0.1
}

/// Archive current memories whose strength fell below the threshold. Hubs are kept.
pub fn archive_stale(conn: &Connection, at: DateTime<Utc>) -> Result<Vec<String>> {
    expire_due(conn, at)?;
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM memories WHERE status = 'current' AND is_hub = 0")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let stamp = at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut archived = vec![];
    for id in ids {
        if let Some(m) = get_memory(conn, &id)?.filter(|m| strength(m, at) < ARCHIVE_BELOW) {
            set_status_at(conn, &m.id, "archived", &stamp)?;
            archived.push(m.id);
        }
    }
    Ok(archived)
}

/// Temporary facts whose `expires_at` has passed stop being current. History is kept.
pub fn expire_due(conn: &Connection, at: DateTime<Utc>) -> Result<usize> {
    let stamp = at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(conn.execute(
        "UPDATE memories SET status = 'expired', valid_to = COALESCE(valid_to, expires_at), updated_at = ?1
         WHERE status = 'current' AND expires_at IS NOT NULL AND expires_at <= ?1",
        [stamp],
    )?)
}

pub fn archive_stale_now(conn: &Connection) -> Result<Vec<String>> {
    let _ = now();
    archive_stale(conn, Utc::now())
}
