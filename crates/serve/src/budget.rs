/// ~4 characters per token is close enough for budgeting English and code.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

pub struct Packable<T> {
    pub item: T,
    pub value: f64,
    pub cost: usize,
}

/// Greedy knapsack approximation: take items by value per token until the budget is full.
/// O(n log n). Items that do not fit are skipped so smaller, denser ones can still be included.
pub fn pack_by_density<T>(mut items: Vec<Packable<T>>, budget: usize) -> (Vec<T>, usize) {
    let density = |p: &Packable<T>| p.value / p.cost.max(1) as f64;
    items.sort_by(|a, b| density(b).total_cmp(&density(a)));
    let mut used = 0;
    let mut packed = vec![];
    for p in items {
        if used + p.cost > budget {
            continue;
        }
        used += p.cost;
        packed.push(p.item);
    }
    (packed, used)
}
