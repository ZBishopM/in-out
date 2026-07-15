//! Poll worker: refresh one user's wishlist snapshot and persist it.
//!
//! Modes:
//!   real  (default) — pull from MercadoLibre via OAuth refresh token.
//!   demo  (`--demo` or `ML_DEMO=1`) — use bundled mock data, no network/creds.
//!
//! Env:
//!   DATABASE_URL                 (required) postgres://user:pass@host/db
//!   INOUT_USER_ID                (optional) uuid; defaults to the demo user
//!   ML_CLIENT_ID/SECRET/REFRESH_TOKEN  (real mode only)
//!   BUDGET_CENTS                 (optional) for the printed buy plan

use anyhow::{Context, Result};
use in_out_core::{buy_plan, rank, Filters, Medal};
use ml_client::{MlClient, OAuthCreds, PricedItem};
use worker_ml::{db, mock, user_id};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let demo = std::env::args().any(|a| a == "--demo")
        || std::env::var("ML_DEMO").is_ok_and(|v| v == "1");

    let db_url = env("DATABASE_URL")?;
    let user = user_id();
    let pool = db::connect(&db_url).await.context("connecting to Postgres")?;

    // TODO(F2): load filters from the `config` table instead of hardcoding.
    let filters = Filters {
        min_medal: Some(Medal::Gold),
        min_sales: Some(100),
        require_verified: false,
    };

    let items: Vec<PricedItem> = if demo {
        tracing::info!("demo mode: using mock wishlist");
        mock::demo_items()
    } else {
        fetch_from_ml().await?
    };
    tracing::info!(count = items.len(), "wishlist items");

    for item in &items {
        db::upsert_wishlist(&pool, user, item).await?;
        db::insert_snapshot(&pool, user, item, &filters).await?;
    }

    // Report: rank the just-fetched snapshots and print the buy plan.
    let budget_cents: i64 = std::env::var("BUDGET_CENTS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let snapshots: Vec<_> = items.iter().map(|i| i.snapshot.clone()).collect();
    let plan = buy_plan(&rank(&snapshots, &filters), budget_cents);
    println!("{}", serde_json::to_string_pretty(&plan)?);

    let passing = db::latest_passing(&pool, user).await?;
    tracing::info!(passing = passing.len(), "snapshots persisted (latest passing per item)");
    Ok(())
}

async fn fetch_from_ml() -> Result<Vec<PricedItem>> {
    let creds = OAuthCreds {
        client_id: env("ML_CLIENT_ID")?,
        client_secret: env("ML_CLIENT_SECRET")?,
        refresh_token: env("ML_REFRESH_TOKEN")?,
    };
    let client = MlClient::from_refresh(&creds).await.context("ML token refresh")?;
    let ids = client.bookmarks().await.context("fetching bookmarks")?;

    let mut out = Vec::new();
    for id in &ids {
        match client.priced_item(id).await {
            Ok(pi) => out.push(pi),
            Err(e) => tracing::warn!(item = %id, error = %e, "snapshot failed"),
        }
    }
    Ok(out)
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow::anyhow!("missing env var {key}"))
}
