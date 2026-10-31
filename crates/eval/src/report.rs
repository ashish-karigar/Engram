use std::collections::BTreeMap;

/// Mean of booleans as a percentage.
pub fn pct(hits: &[bool]) -> f64 {
    if hits.is_empty() {
        return 0.0;
    }
    100.0 * hits.iter().filter(|h| **h).count() as f64 / hits.len() as f64
}

pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// A plain-text table: one row per group, then the overall row.
pub fn table(header: &[&str], rows: &BTreeMap<String, Vec<String>>, total: Vec<String>) -> String {
    let mut lines = vec![header.join(" | "), header.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")];
    lines.extend(rows.iter().map(|(name, cells)| format!("{name} | {}", cells.join(" | "))));
    lines.push(format!("**all** | {}", total.join(" | ")));
    lines.join("\n")
}
