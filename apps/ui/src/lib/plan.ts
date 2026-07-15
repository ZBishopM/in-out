// TypeScript mirror of `in_out_core` (wishlist + budget).
// Used for the in-browser preview; inside Tauri the Rust command is the source
// of truth. Keep these two in sync — the Rust tests are the spec.

export type Medal = 'silver' | 'gold' | 'platinum';

export interface ItemSnapshot {
  item_id: string;
  title: string;
  price_cents: number;
  currency: string;
  seller_status: Medal | null;
  seller_sales: number;
  verified: boolean;
}

export interface Filters {
  min_medal: Medal | null;
  min_sales: number | null;
  require_verified: boolean;
}

export interface Affordable {
  item: ItemSnapshot;
  affordable: boolean;
}

export interface BuyPlan {
  budget_cents: number;
  spent_cents: number;
  remaining_cents: number;
  next_gap_cents: number | null;
  items: Affordable[];
}

const MEDAL_RANK: Record<Medal, number> = { silver: 0, gold: 1, platinum: 2 };

function passes(it: ItemSnapshot, f: Filters): boolean {
  if (f.min_medal) {
    if (!it.seller_status || MEDAL_RANK[it.seller_status] < MEDAL_RANK[f.min_medal]) return false;
  }
  if (f.min_sales != null && it.seller_sales < f.min_sales) return false;
  if (f.require_verified && !it.verified) return false;
  return true;
}

/** Filter by `f`, then sort cheapest-first. */
export function rank(items: ItemSnapshot[], f: Filters): ItemSnapshot[] {
  return items.filter((i) => passes(i, f)).sort((a, b) => a.price_cents - b.price_cents);
}

/** Greedy buy plan over a price-ascending list. */
export function buyPlan(ranked: ItemSnapshot[], budgetCents: number): BuyPlan {
  let spent = 0;
  let nextGap: number | null = null;
  const items: Affordable[] = [];
  for (const it of ranked) {
    if (spent + it.price_cents <= budgetCents) {
      spent += it.price_cents;
      items.push({ item: it, affordable: true });
    } else {
      if (nextGap === null) nextGap = spent + it.price_cents - budgetCents;
      items.push({ item: it, affordable: false });
    }
  }
  return {
    budget_cents: budgetCents,
    spent_cents: spent,
    remaining_cents: budgetCents - spent,
    next_gap_cents: nextGap,
    items
  };
}

export function progress(plan: BuyPlan): number {
  if (plan.budget_cents <= 0) return 0;
  return Math.min(1, Math.max(0, plan.spent_cents / plan.budget_cents));
}
