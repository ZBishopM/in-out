//! Synthetic finance data to exercise reconciliation without real email yet.
//!
//! Scenario: an Uber ride produces a merchant receipt AND a BCP card charge
//! (must collapse to one transaction); plus a standalone Yape payment and a
//! PayPal inflow.

use anyhow::Result;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed(pool: &PgPool, user: Uuid) -> Result<()> {
    // Clean slate for this user (children first).
    for tbl in ["transaction_links", "transactions", "raw_events", "accounts"] {
        let sql = match tbl {
            "transaction_links" => "delete from transaction_links l using transactions t \
                                     where l.transaction_id = t.id and t.user_id = $1"
                .to_string(),
            other => format!("delete from {other} where user_id = $1"),
        };
        sqlx::query(&sql).bind(user).execute(pool).await?;
    }

    let card = insert_account(pool, user, "BCP Visa", "card").await?;
    let yape = insert_account(pool, user, "Yape", "yape").await?;
    let paypal = insert_account(pool, user, "PayPal", "paypal").await?;

    let base = Utc.with_ymd_and_hms(2026, 7, 14, 9, 0, 0).unwrap();

    // 0: Uber receipt (merchant email, no account)
    insert_raw(pool, user, None, "uber", "seed-uber-receipt", base, 2550, "out", "Uber Trip").await?;
    // 1: BCP card charge ~1h later -> duplicate of the receipt
    insert_raw(pool, user, Some(card), "bcp", "seed-bcp-charge", base + chrono::Duration::hours(1), 2550, "out", "UBER *TRIP HELP.UBER.COM").await?;
    // 2: standalone Yape payment
    insert_raw(pool, user, Some(yape), "yape", "seed-yape", base + chrono::Duration::hours(3), 1500, "out", "Yape a Juan Perez").await?;
    // 3: standalone PayPal inflow
    insert_raw(pool, user, Some(paypal), "paypal", "seed-paypal", base + chrono::Duration::hours(5), 10000, "in", "Cliente X").await?;

    Ok(())
}

async fn insert_account(pool: &PgPool, user: Uuid, name: &str, kind: &str) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "insert into accounts (user_id, name, kind, currency) values ($1,$2,$3,'PEN') returning id",
    )
    .bind(user)
    .bind(name)
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
async fn insert_raw(
    pool: &PgPool,
    user: Uuid,
    account: Option<Uuid>,
    source: &str,
    gmail_id: &str,
    received_at: chrono::DateTime<Utc>,
    amount_cents: i64,
    direction: &str,
    merchant: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        insert into raw_events
          (user_id, account_id, source, gmail_msg_id, received_at,
           amount_cents, currency, direction, merchant_raw)
        values ($1,$2,$3,$4,$5,$6,'PEN',$7,$8)
        "#,
    )
    .bind(user)
    .bind(account)
    .bind(source)
    .bind(gmail_id)
    .bind(received_at)
    .bind(amount_cents)
    .bind(direction)
    .bind(merchant)
    .execute(pool)
    .await?;
    Ok(())
}
