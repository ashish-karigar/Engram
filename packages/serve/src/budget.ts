/** ~4 characters per token is close enough for budgeting English and code. */
export const estimateTokens = (text: string): number => Math.ceil(text.length / 4);

export interface Packable<T> {
  item: T;
  value: number;
  cost: number;
}

/**
 * Greedy knapsack approximation: take items in order of value per token until the budget is full.
 * O(n log n). Items that do not fit are skipped so smaller, denser ones can still be included.
 */
export function packByDensity<T>(items: readonly Packable<T>[], budget: number): { packed: T[]; used: number } {
  const ranked = [...items].sort((a, b) => b.value / Math.max(1, b.cost) - a.value / Math.max(1, a.cost));
  const packed: T[] = [];
  let used = 0;
  for (const { item, cost } of ranked) {
    if (used + cost > budget) continue;
    packed.push(item);
    used += cost;
  }
  return { packed, used };
}
