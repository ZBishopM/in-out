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

mod db;
mod reconcile;
mod seed;

use anyhow::{Context, Result};
use uuid::Uuid;

fn user_id() -> Uuid {
    std::env::var("INOUT_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}

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
