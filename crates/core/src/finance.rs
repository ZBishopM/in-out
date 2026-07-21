//! Duplicate detection for financial email events.
//!
//! One real spend often produces several emails: e.g. an Uber receipt *and* a
//! BCP credit-card charge. We collapse them into one logical transaction.
//! Matching is fuzzy on three axes, all configurable:
//!   * same currency + amount within tolerance,
//!   * timestamps within a window,
//!   * merchant name trigram similarity above a threshold.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A parsed line item extracted from one email. Immutable / reprocessable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub amount_cents: i64,
    pub currency: String,
    pub occurred_at: DateTime<Utc>,
    pub merchant: String,
}

/// Tunables for the matcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileConfig {
    /// Max timestamp gap, in seconds.
    pub window_secs: i64,
    /// Max absolute amount difference, in cents.
    pub amount_tol_cents: i64,
    /// Min merchant trigram Jaccard similarity, 0.0..=1.0.
    pub merchant_min_sim: f32,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            window_secs: 72 * 3600, // 72h: card settlement can lag the receipt
            amount_tol_cents: 0,
            merchant_min_sim: 0.6,
        }
    }
}

/// Group events into clusters where every member is a duplicate of at least one
/// other member (transitively). Each returned inner vec holds indices into
/// `events`; singletons are their own cluster. Order within a cluster is
/// ascending by index. This is the reconciliation step: one cluster = one
/// canonical transaction.
pub fn cluster(events: &[RawEvent], cfg: &ReconcileConfig) -> Vec<Vec<usize>> {
    let n = events.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path halving
            x = parent[x];
        }
        x
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if is_duplicate(&events[i], &events[j], cfg) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    // Bucket indices by their representative root, preserving ascending order.
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    groups.into_values().collect()
}

/// True if `a` and `b` most likely describe the same real-world spend.
pub fn is_duplicate(a: &RawEvent, b: &RawEvent, cfg: &ReconcileConfig) -> bool {
    if !a.currency.eq_ignore_ascii_case(&b.currency) {
        return false;
    }
    if (a.amount_cents - b.amount_cents).abs() > cfg.amount_tol_cents {
        return false;
    }
    if (a.occurred_at - b.occurred_at).num_seconds().abs() > cfg.window_secs {
        return false;
    }
    trigram_sim(&a.merchant, &b.merchant) >= cfg.merchant_min_sim
}

/// Overlap coefficient (Szymkiewicz–Simpson) of the character-trigram sets:
/// `|A ∩ B| / min(|A|, |B|)`.
///
/// We use overlap, not Jaccard, on purpose: a bank charge often wraps the
/// merchant name in noise (`"UBER *TRIP HELP.UBER.COM"`), so the receipt's
/// trigrams are a near-subset of the charge's. Jaccard would penalize that
/// extra noise; overlap measures how fully the smaller name is contained.
pub fn trigram_sim(a: &str, b: &str) -> f32 {
    let ta = trigrams(a);
    let tb = trigrams(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    let smaller = ta.len().min(tb.len()) as f32;
    if smaller == 0.0 {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    inter / smaller
}

fn trigrams(s: &str) -> HashSet<String> {
    let norm: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let padded = format!("  {}  ", norm.trim());
    let chars: Vec<char> = padded.chars().collect();
    chars.windows(3).map(|w| w.iter().collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ev(amount: i64, merchant: &str, hour: u32) -> RawEvent {
        RawEvent {
            amount_cents: amount,
            currency: "PEN".into(),
            occurred_at: Utc.with_ymd_and_hms(2026, 7, 14, hour, 0, 0).unwrap(),
            merchant: merchant.into(),
        }
    }

    #[test]
    fn uber_receipt_matches_card_charge() {
        // Uber sends a receipt; BCP charges the card ~an hour later.
        let receipt = ev(2550, "Uber Trip", 9);
        let charge = ev(2550, "UBER *TRIP HELP.UBER.COM", 10);
        assert!(is_duplicate(&receipt, &charge, &ReconcileConfig::default()));
    }

    #[test]
    fn different_merchant_not_duplicate() {
        let a = ev(2550, "Uber", 9);
        let b = ev(2550, "Rappi", 9);
        assert!(!is_duplicate(&a, &b, &ReconcileConfig::default()));
    }

    #[test]
    fn amount_outside_tolerance_not_duplicate() {
        let a = ev(2550, "Uber", 9);
        let b = ev(9999, "Uber", 9);
        assert!(!is_duplicate(&a, &b, &ReconcileConfig::default()));
    }

    #[test]
    fn outside_window_not_duplicate() {
        let cfg = ReconcileConfig { window_secs: 3600, ..Default::default() };
        let a = ev(2550, "Uber", 9);
        let b = ev(2550, "Uber", 20);
        assert!(!is_duplicate(&a, &b, &cfg));
    }

    #[test]
    fn cluster_merges_receipt_and_charge() {
        let events = vec![
            ev(2550, "Uber Trip", 9),                  // 0: receipt
            ev(2550, "UBER *TRIP HELP.UBER.COM", 10),  // 1: card charge -> dup of 0
            ev(1500, "Yape a Juan", 12),               // 2: standalone
            ev(9900, "Netflix", 20),                   // 3: standalone
        ];
        let clusters = cluster(&events, &ReconcileConfig::default());
        assert_eq!(clusters.len(), 3);
        // The two-member cluster is the Uber pair.
        let pair = clusters.iter().find(|c| c.len() == 2).expect("a 2-member cluster");
        assert_eq!(pair, &vec![0, 1]);
        assert_eq!(clusters.iter().filter(|c| c.len() == 1).count(), 2);
    }
}
