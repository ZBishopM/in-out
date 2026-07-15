//! "What can I buy" planner — drives the progress bar.
//!
//! Given a budget and a price-ascending list of items, greedily take the
//! cheapest first. Because the input is sorted ascending, sequential take is
//! optimal for maximizing the count of affordable items.

use serde::{Deserialize, Serialize};

use crate::wishlist::ItemSnapshot;

/// One item annotated with whether it fits inside the running budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Affordable {
    pub item: ItemSnapshot,
    pub affordable: bool,
}

/// Result of planning a purchase run against a budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuyPlan {
    pub budget_cents: i64,
    /// Sum of prices actually taken.
    pub spent_cents: i64,
    /// Cents still free after taking everything affordable.
    pub remaining_cents: i64,
    /// Extra cents needed to also afford the next (first non-affordable) item.
    pub next_gap_cents: Option<i64>,
    pub items: Vec<Affordable>,
}

impl BuyPlan {
    /// Progress ratio 0.0..=1.0 for the loading bar.
    pub fn progress(&self) -> f64 {
        if self.budget_cents <= 0 {
            return 0.0;
        }
        (self.spent_cents as f64 / self.budget_cents as f64).clamp(0.0, 1.0)
    }

    /// Count of items marked affordable.
    pub fn affordable_count(&self) -> usize {
        self.items.iter().filter(|a| a.affordable).count()
    }
}

/// Build a buy plan. `ranked` must be sorted cheapest-first (see [`crate::rank`]).
pub fn buy_plan(ranked: &[ItemSnapshot], budget_cents: i64) -> BuyPlan {
    let mut spent = 0i64;
    let mut next_gap = None;
    let mut items = Vec::with_capacity(ranked.len());

    for it in ranked {
        if spent + it.price_cents <= budget_cents {
            spent += it.price_cents;
            items.push(Affordable { item: it.clone(), affordable: true });
        } else {
            if next_gap.is_none() {
                next_gap = Some(spent + it.price_cents - budget_cents);
            }
            items.push(Affordable { item: it.clone(), affordable: false });
        }
    }

    BuyPlan {
        budget_cents,
        spent_cents: spent,
        remaining_cents: budget_cents - spent,
        next_gap_cents: next_gap,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wishlist::Medal;

    fn item(id: &str, price: i64) -> ItemSnapshot {
        ItemSnapshot {
            item_id: id.into(),
            title: id.into(),
            price_cents: price,
            currency: "PEN".into(),
            seller_status: Some(Medal::Gold),
            seller_sales: 100,
            verified: true,
        }
    }

    #[test]
    fn greedy_takes_cheapest_until_budget() {
        let ranked = vec![item("a", 1000), item("b", 2000), item("c", 5000)];
        let plan = buy_plan(&ranked, 3500);
        // a + b = 3000 fits, c does not.
        assert_eq!(plan.affordable_count(), 2);
        assert_eq!(plan.spent_cents, 3000);
        assert_eq!(plan.remaining_cents, 500);
        assert_eq!(plan.next_gap_cents, Some(5000 - 500)); // need 4500 more for c
        assert!(plan.items[0].affordable && plan.items[1].affordable);
        assert!(!plan.items[2].affordable);
    }

    #[test]
    fn progress_clamped() {
        let ranked = vec![item("a", 1000)];
        let plan = buy_plan(&ranked, 0);
        assert_eq!(plan.progress(), 0.0);
        assert_eq!(plan.affordable_count(), 0);
    }
}
