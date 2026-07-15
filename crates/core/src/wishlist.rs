//! Wishlist filtering + ranking.

use serde::{Deserialize, Serialize};

/// MercadoLíder medal. Declaration order defines ranking: Silver < Gold < Platinum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Medal {
    Silver,
    Gold,
    Platinum,
}

impl Medal {
    /// Map MercadoLibre's `power_seller_status` string to a medal.
    pub fn from_power_seller_status(s: &str) -> Option<Medal> {
        match s.to_ascii_lowercase().as_str() {
            "silver" => Some(Medal::Silver),
            "gold" => Some(Medal::Gold),
            "platinum" => Some(Medal::Platinum),
            _ => None,
        }
    }

    /// Lowercase wire name, matching `power_seller_status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Medal::Silver => "silver",
            Medal::Gold => "gold",
            Medal::Platinum => "platinum",
        }
    }
}

/// User-configurable seller filters. `None` = don't filter on that field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Filters {
    /// Minimum medal required (e.g. `Gold` accepts Gold + Platinum).
    pub min_medal: Option<Medal>,
    /// Minimum completed sales.
    pub min_sales: Option<u32>,
    /// Require the "verified" heuristic (see [`ItemSnapshot::verified`]).
    pub require_verified: bool,
}

/// A priced wishlist item plus the seller signals we filter on.
/// Prices are integer cents to avoid float rounding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSnapshot {
    pub item_id: String,
    pub title: String,
    pub price_cents: i64,
    pub currency: String,
    pub seller_status: Option<Medal>,
    pub seller_sales: u32,
    /// Heuristic: MercadoLibre exposes no clean "verified" flag; derived server-side.
    pub verified: bool,
}

impl ItemSnapshot {
    /// Whether this item satisfies every active filter.
    pub fn passes(&self, f: &Filters) -> bool {
        if let Some(min) = f.min_medal {
            match self.seller_status {
                Some(m) if m >= min => {}
                _ => return false,
            }
        }
        if let Some(min) = f.min_sales {
            if self.seller_sales < min {
                return false;
            }
        }
        if f.require_verified && !self.verified {
            return false;
        }
        true
    }
}

/// Keep only items that pass `filters`, sorted by price ascending (cheapest first).
pub fn rank(items: &[ItemSnapshot], filters: &Filters) -> Vec<ItemSnapshot> {
    let mut kept: Vec<ItemSnapshot> = items.iter().filter(|i| i.passes(filters)).cloned().collect();
    kept.sort_by_key(|i| i.price_cents);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, price: i64, medal: Option<Medal>, sales: u32, verified: bool) -> ItemSnapshot {
        ItemSnapshot {
            item_id: id.into(),
            title: id.into(),
            price_cents: price,
            currency: "PEN".into(),
            seller_status: medal,
            seller_sales: sales,
            verified,
        }
    }

    #[test]
    fn medal_ordering() {
        assert!(Medal::Gold >= Medal::Gold);
        assert!(Medal::Platinum > Medal::Gold);
        assert!(Medal::Silver < Medal::Gold);
    }

    #[test]
    fn filter_by_medal_and_sales() {
        let items = vec![
            item("cheap-silver", 1000, Some(Medal::Silver), 500, true),
            item("gold-lowsales", 2000, Some(Medal::Gold), 5, true),
            item("gold-ok", 3000, Some(Medal::Gold), 500, true),
            item("platinum-ok", 4000, Some(Medal::Platinum), 5000, true),
        ];
        let f = Filters { min_medal: Some(Medal::Gold), min_sales: Some(100), require_verified: false };
        let ranked = rank(&items, &f);
        let ids: Vec<_> = ranked.iter().map(|i| i.item_id.as_str()).collect();
        assert_eq!(ids, vec!["gold-ok", "platinum-ok"]);
    }

    #[test]
    fn ranks_cheapest_first() {
        let items = vec![
            item("b", 3000, Some(Medal::Gold), 100, true),
            item("a", 1000, Some(Medal::Gold), 100, true),
            item("c", 2000, Some(Medal::Gold), 100, true),
        ];
        let ranked = rank(&items, &Filters::default());
        let prices: Vec<_> = ranked.iter().map(|i| i.price_cents).collect();
        assert_eq!(prices, vec![1000, 2000, 3000]);
    }
}
