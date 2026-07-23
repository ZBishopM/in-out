//! Finance reconciliation worker.
//!
//! Modes:
//!   default    — reconcile currently-unlinked raw events into transactions.
//!   `--seed`   — wipe this user's finance rows, insert synthetic data, then
//!                reconcile (for testing without real email ingestion).
//!
//! Env:
//!   DATABASE_URL    (required)
//!   INOUT_USER_ID   (optional) uuid; defaults to the demo user.

use anyhow::{Context, Result};
use finance_worker::{db, reconcile, seed, user_id};

#[tokio::main]
async fn main() -> Result<()> {
    for p in ["server/.env", ".env", "../server/.env", "../../server/.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db_url = std::env::var("DATABASE_URL").context("DATABASE_URL not set")?;
    let user = user_id();
    let pool = db::connect(&db_url).await.context("connecting to Postgres")?;

    // Backfill categories for existing transactions, then exit.
    if std::env::args().any(|a| a == "--recategorize") {
        let n = db::recategorize_all(&pool, user).await?;
        println!("recategorized {n} transactions");
        return Ok(());
    }

    // Merge already-stored duplicate transactions, then exit.
    if std::env::args().any(|a| a == "--rededup") {
        let n = db::rededup(&pool, user).await?;
        println!("merged {n} duplicate transactions");
        return Ok(());
    }

    if std::env::args().any(|a| a == "--seed") {
        seed::seed(&pool, user).await?;
        tracing::info!("seeded synthetic finance data");
    }

    let created = reconcile::reconcile(&pool, user).await?;
    let (raw, tx, links) = db::summary(&pool, user).await?;
    tracing::info!(created, raw_events = raw, transactions = tx, links, "reconcile done");
    println!("raw_events={raw} transactions={tx} links={links} (created {created} this run)");
    Ok(())
}
