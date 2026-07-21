//! Turn clusters of raw events into canonical transactions + links.

use anyhow::Result;
use in_out_core::{cluster, RawEvent, ReconcileConfig};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{self, Ev};

/// Sources that represent the actual money movement (bank/card), as opposed to
/// a merchant receipt. Extend as more banks are added.
fn is_settlement(source: &str) -> bool {
    matches!(source, "bcp" | "bbva" | "interbank" | "scotiabank" | "card" | "visa" | "bank")
}

/// Reconcile all currently-unlinked raw events. Returns the number of
/// transactions created this run.
pub async fn reconcile(pool: &PgPool, user: Uuid) -> Result<usize> {
    let evs = db::load_unlinked(pool, user).await?;
    if evs.is_empty() {
        return Ok(0);
    }

    let raws: Vec<RawEvent> = evs.iter().map(|e| e.core.clone()).collect();
    let clusters = cluster(&raws, &ReconcileConfig::default());

    let mut created = 0usize;
    for c in &clusters {
        let members: Vec<&Ev> = c.iter().map(|&i| &evs[i]).collect();
        let settlement = members.iter().copied().find(|m| is_settlement(&m.source));
        let receipt = members.iter().copied().find(|m| !is_settlement(&m.source));

        // Prefer the bank event for amount/currency/direction (what actually
        // moved), and the merchant receipt for the human-readable name.
        let canon = settlement.or(receipt).expect("cluster is non-empty");
        let name_src = receipt.or(settlement).expect("cluster is non-empty");
        let occurred = members.iter().map(|m| m.core.occurred_at).min().unwrap();

        let tx = db::insert_transaction(
            pool,
            user,
            occurred,
            canon.core.amount_cents,
            &canon.core.currency,
            &canon.direction,
            &name_src.core.merchant,
        )
        .await?;

        for m in &members {
            let role = if is_settlement(&m.source) { "settlement" } else { "receipt" };
            db::insert_link(pool, tx, m.id, role).await?;
        }
        created += 1;
    }

    db::recompute_balances(pool, user).await?;
    Ok(created)
}
