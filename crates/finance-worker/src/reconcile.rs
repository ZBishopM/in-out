//! Turn clusters of raw events into canonical transactions + links.

use anyhow::Result;
use chrono::{DateTime, Utc};
use in_out_core::{categorize, cluster, RawEvent, ReconcileConfig};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{self, Ev};

/// Sources that represent the actual money movement (bank/card), as opposed to
/// a merchant receipt. Extend as more banks are added.
fn is_settlement(source: &str) -> bool {
    matches!(source, "bcp" | "bbva" | "interbank" | "scotiabank" | "card" | "visa" | "bank")
}

/// A transaction newly created by this reconcile run (as opposed to an
/// existing one a new raw_event just got attached to) — enough detail to
/// post a Discord notification about it.
#[derive(Debug, Clone, Serialize)]
pub struct NewTransaction {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub amount_cents: i64,
    pub currency: String,
    pub direction: String,
    pub merchant: String,
    pub category: String,
}

/// Reconcile all currently-unlinked raw events. Returns the transactions
/// created this run (not ones an event merely got attached to).
pub async fn reconcile(pool: &PgPool, user: Uuid) -> Result<Vec<NewTransaction>> {
    let evs = db::load_unlinked(pool, user).await?;
    if evs.is_empty() {
        return Ok(Vec::new());
    }

    let cfg = ReconcileConfig::default();
    let raws: Vec<RawEvent> = evs.iter().map(|e| e.core.clone()).collect();
    let clusters = cluster(&raws, &cfg);

    let mut created = Vec::new();
    for c in &clusters {
        let members: Vec<&Ev> = c.iter().map(|&i| &evs[i]).collect();
        let settlement = members.iter().copied().find(|m| is_settlement(&m.source));
        let receipt = members.iter().copied().find(|m| !is_settlement(&m.source));

        // Prefer the bank event for amount/currency/direction (what actually
        // moved), and the merchant receipt for the human-readable name.
        let canon = settlement.or(receipt).expect("cluster is non-empty");
        let name_src = receipt.or(settlement).expect("cluster is non-empty");
        let occurred = members.iter().map(|m| m.core.occurred_at).min().unwrap();
        let merchant = &name_src.core.merchant;

        // Cross-batch dedup: if this matches a transaction reconciled in an
        // earlier batch (e.g. a bank auth then its settlement), attach to it
        // instead of creating a duplicate.
        let existing = db::find_matching_transaction(
            pool, user, canon.core.amount_cents, &canon.core.currency, &canon.direction, occurred, merchant, &cfg,
        )
        .await?;

        let tx = match existing {
            Some(id) => id,
            None => {
                let category = categorize(merchant, &canon.direction);
                let id = db::insert_transaction(
                    pool, user, occurred, canon.core.amount_cents, &canon.core.currency,
                    &canon.direction, merchant, category,
                )
                .await?;
                created.push(NewTransaction {
                    id,
                    occurred_at: occurred,
                    amount_cents: canon.core.amount_cents,
                    currency: canon.core.currency.clone(),
                    direction: canon.direction.clone(),
                    merchant: merchant.clone(),
                    category: category.to_string(),
                });
                id
            }
        };

        for m in &members {
            let role = if is_settlement(&m.source) { "settlement" } else { "receipt" };
            db::insert_link(pool, tx, m.id, role).await?;
        }
    }

    db::recompute_balances(pool, user).await?;
    Ok(created)
}
