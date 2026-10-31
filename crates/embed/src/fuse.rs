use std::collections::HashMap;

/// Reciprocal Rank Fusion: score(d) = Σ 1 / (k + rank_i(d)). Robust to the different scales of
/// BM25, cosine and graph scores because it only uses ranks. k = 60 is the usual constant.
pub fn rrf(rankings: &[Vec<String>], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<&str, f64> = HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.iter().enumerate() {
            *scores.entry(id.as_str()).or_default() += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().map(|(id, s)| (id.to_string(), s)).collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
}
