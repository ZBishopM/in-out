//! Postgres access for finance reconciliation.

use anyhow::Result;
use chrono::{DateTime, Utc};
use in_out_core::RawEvent;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

/// A raw event loaded from the DB: the core fields for matching plus the
/// metadata we need to build the canonical transaction.
pub struct Ev {
    pub id: Uuid,
    pub source: String,
    pub direction: String,
    pub core: RawEvent,
}

pub async fn connect(url: &str) -> Result<PgPool> {
    Ok(PgPoolOptions::new().max_connections(4).connect(url).await?)
}

/// Raw events not yet linked to any transaction, oldest first.
pub async fn load_unlinked(pool: &PgPool, user: Uuid) -> Result<Vec<Ev>> {
    let rows: Vec<(Uuid, String, DateTime<Utc>, i64, String, String, Option<String>)> =
        sqlx::query_as(
            r#"
            select r.id, r.source, r.received_at,
                   r.amount_cents, r.currency, r.direction, r.merchant_raw
            from raw_events r
            where r.user_id = $1
              and not exists (
                select 1 from transaction_links l where l.raw_event_id = r.id
              )
            order by r.received_at
            "#,
        )
        .bind(user)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, source, received_at, amount_cents, currency, direction, merchant)| Ev {
            id,
            source,
            direction,
            core: RawEvent {
                amount_cents,
                currency,
                occurred_at: received_at,
                merchant: merchant.unwrap_or_default(),
            },
        })
        .collect())
}

/// Insert a canonical transaction, returning its id.
#[allow(clippy::too_many_arguments)]
pub async fn insert_transaction(
    pool: &PgPool,
    user: Uuid,
    occurred_at: DateTime<Utc>,
    amount_cents: i64,
    currency: &str,
    direction: &str,
    merchant: &str,
) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        insert into transactions
          (user_id, occurred_at, amount_cents, currency, direction, merchant, reconciled)
        values ($1, $2, $3, $4, $5, $6, true)
        returning id
        "#,
    )
    .bind(user)
    .bind(occurred_at)
    .bind(amount_cents)
    .bind(currency)
    .bind(direction)
    .bind(merchant)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn insert_link(pool: &PgPool, tx: Uuid, raw_event: Uuid, role: &str) -> Result<()> {
    sqlx::query(
        "insert into transaction_links (transaction_id, raw_event_id, role) values ($1, $2, $3)",
    )
    .bind(tx)
    .bind(raw_event)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

/// Recompute every account's balance as the net of the transactions attributed
/// to it (via a linked raw event). `distinct` guards against a transaction with
/// several links to the same account being counted twice.
pub async fn recompute_balances(pool: &PgPool, user: Uuid) -> Result<()> {
    sqlx::query(
        r#"
        update accounts a set balance_cents = coalesce((
            select sum(x.signed) from (
                select distinct t.id,
                    case when t.direction = 'in' then t.amount_cents else -t.amount_cents end as signed
                from transactions t
                join transaction_links l on l.transaction_id = t.id
                join raw_events r on r.id = l.raw_event_id
                where r.account_id = a.id
            ) x
        ), 0)
        where a.user_id = $1
        "#,
    )
    .bind(user)
    .execute(pool)
    .await?;
    Ok(())
}

/// Counts + balances for a run summary.
pub async fn summary(pool: &PgPool, user: Uuid) -> Result<(i64, i64, i64)> {
    let raw: i64 = sqlx::query_scalar("select count(*) from raw_events where user_id = $1")
        .bind(user).fetch_one(pool).await?;
    let tx: i64 = sqlx::query_scalar("select count(*) from transactions where user_id = $1")
        .bind(user).fetch_one(pool).await?;
    let links: i64 = sqlx::query_scalar(
        "select count(*) from transaction_links l join transactions t on t.id = l.transaction_id where t.user_id = $1",
    )
    .bind(user).fetch_one(pool).await?;
    Ok((raw, tx, links))
}
