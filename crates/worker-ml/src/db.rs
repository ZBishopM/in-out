//! Postgres persistence for wishlist items + price/seller snapshots (sqlx).

use anyhow::Result;
use in_out_core::{Filters, ItemSnapshot};
use ml_client::PricedItem;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

/// Open a small connection pool.
pub async fn connect(url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new().max_connections(4).connect(url).await?;
    Ok(pool)
}

/// Upsert the wishlist row (title/permalink can change between polls).
pub async fn upsert_wishlist(pool: &PgPool, user: Uuid, item: &PricedItem) -> Result<()> {
    sqlx::query(
        r#"
        insert into wishlist_items (user_id, item_id, title, permalink)
        values ($1, $2, $3, $4)
        on conflict (user_id, item_id)
        do update set title = excluded.title, permalink = excluded.permalink
        "#,
    )
    .bind(user)
    .bind(&item.snapshot.item_id)
    .bind(&item.snapshot.title)
    .bind(&item.permalink)
    .execute(pool)
    .await?;
    Ok(())
}

/// Append a price/seller snapshot, tagged with whether it passes the filters.
pub async fn insert_snapshot(
    pool: &PgPool,
    user: Uuid,
    item: &PricedItem,
    filters: &Filters,
) -> Result<()> {
    let s: &ItemSnapshot = &item.snapshot;
    let status = s.seller_status.map(|m| m.as_str());
    sqlx::query(
        r#"
        insert into item_snapshots
          (user_id, item_id, price_cents, currency, seller_id,
           power_seller_status, seller_tx_completed, verified, passes_filter)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(user)
    .bind(&s.item_id)
    .bind(s.price_cents)
    .bind(&s.currency)
    .bind(item.seller_id)
    .bind(status)
    .bind(s.seller_sales as i32)
    .bind(s.verified)
    .bind(s.passes(filters))
    .execute(pool)
    .await?;
    Ok(())
}

/// Latest snapshot per item that passes the filter, cheapest first.
/// The client reads this (or the equivalent Supabase view) to render the list.
pub async fn latest_passing(pool: &PgPool, user: Uuid) -> Result<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        select distinct on (item_id) item_id, price_cents
        from item_snapshots
        where user_id = $1 and passes_filter
        order by item_id, captured_at desc
        "#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    let mut v = rows;
    v.sort_by_key(|(_, price)| *price);
    Ok(v)
}
