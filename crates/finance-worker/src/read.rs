//! Read queries for the dashboard. All scoped to one user.

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize)]
pub struct SummaryRow {
    pub currency: String,
    pub direction: String,
    pub total_cents: i64,
    pub count: i64,
}

#[derive(Serialize)]
pub struct DayRow {
    pub day: NaiveDate,
    pub out_cents: i64,
    pub in_cents: i64,
}

#[derive(Serialize)]
pub struct HourRow {
    pub hour: i32,
    pub out_cents: i64,
}

#[derive(Serialize)]
pub struct CategoryRow {
    pub category: String,
    pub out_cents: i64,
}

#[derive(Serialize)]
pub struct AccountRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub currency: String,
    pub balance_cents: i64,
    pub opening_balance_cents: i64,
    pub credit_limit_cents: Option<i64>,
    pub trea_bps: Option<i32>,
}

#[derive(Serialize)]
pub struct TxRow {
    pub occurred_at: DateTime<Utc>,
    pub direction: String,
    pub amount_cents: i64,
    pub currency: String,
    pub merchant: Option<String>,
}

pub async fn summary(pool: &PgPool, user: Uuid) -> Result<Vec<SummaryRow>> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        r#"select currency, direction, coalesce(sum(amount_cents),0)::bigint, count(*)::bigint
           from transactions where user_id = $1
           group by currency, direction order by currency, direction"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(currency, direction, total_cents, count)| SummaryRow { currency, direction, total_cents, count })
        .collect())
}

pub async fn daily(pool: &PgPool, user: Uuid, days: i32) -> Result<Vec<DayRow>> {
    let rows: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        r#"select (occurred_at at time zone 'America/Lima')::date as day,
                  coalesce(sum(amount_cents) filter (where direction='out'),0)::bigint as out_cents,
                  coalesce(sum(amount_cents) filter (where direction='in'),0)::bigint as in_cents
           from transactions
           where user_id = $1 and currency = 'PEN'
             and (occurred_at at time zone 'America/Lima')::date
                 >= (now() at time zone 'America/Lima')::date - $2::int
           group by day order by day"#,
    )
    .bind(user)
    .bind(days)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(day, out_cents, in_cents)| DayRow { day, out_cents, in_cents }).collect())
}

pub async fn hourly(pool: &PgPool, user: Uuid) -> Result<Vec<HourRow>> {
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        r#"select extract(hour from (occurred_at at time zone 'America/Lima'))::int as hour,
                  coalesce(sum(amount_cents),0)::bigint as out_cents
           from transactions
           where user_id = $1 and direction = 'out' and currency = 'PEN'
           group by hour order by hour"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(hour, out_cents)| HourRow { hour, out_cents }).collect())
}

pub async fn by_category(pool: &PgPool, user: Uuid) -> Result<Vec<CategoryRow>> {
    let rows: Vec<(Option<String>, i64)> = sqlx::query_as(
        r#"select coalesce(category, 'Otros') as category,
                  coalesce(sum(amount_cents),0)::bigint as out_cents
           from transactions
           where user_id = $1 and direction = 'out' and currency = 'PEN'
           group by category order by out_cents desc"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(category, out_cents)| CategoryRow { category: category.unwrap_or_else(|| "Otros".into()), out_cents })
        .collect())
}

pub async fn accounts(pool: &PgPool, user: Uuid) -> Result<Vec<AccountRow>> {
    let rows: Vec<(Uuid, String, String, String, i64, i64, Option<i64>, Option<i32>)> = sqlx::query_as(
        r#"select id, name, kind, currency, balance_cents, opening_balance_cents,
                  credit_limit_cents, trea_bps
           from accounts where user_id = $1 order by name"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, kind, currency, balance_cents, opening_balance_cents, credit_limit_cents, trea_bps)| {
                AccountRow {
                    id,
                    name,
                    kind,
                    currency,
                    balance_cents,
                    opening_balance_cents,
                    credit_limit_cents,
                    trea_bps,
                }
            },
        )
        .collect())
}

/// One email that produced a raw_event — the audit view's "parsed" side.
#[derive(Serialize)]
pub struct ParsedEventRow {
    pub received_at: DateTime<Utc>,
    pub sender: Option<String>,
    pub subject: Option<String>,
    pub gmail_msg_id: String,
    pub source: String,
    pub amount_cents: i64,
    pub currency: String,
    pub direction: String,
    pub merchant_raw: Option<String>,
    /// Whether this raw_event is folded into a canonical transaction yet
    /// (reconcile runs after every ingest batch, so "false" here usually
    /// means something's actually wrong, not just "pending").
    pub linked: bool,
}

/// One email the parser returned `None` for — the audit view's "discarded"
/// side, for manually checking nothing real got missed.
#[derive(Serialize)]
pub struct DiscardedEventRow {
    pub received_at: DateTime<Utc>,
    pub sender: String,
    pub subject: Option<String>,
    pub gmail_msg_id: String,
}

pub async fn audit_parsed(pool: &PgPool, user: Uuid, limit: i64) -> Result<Vec<ParsedEventRow>> {
    let rows: Vec<(
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        String,
        String,
        i64,
        String,
        String,
        Option<String>,
        bool,
    )> = sqlx::query_as(
        r#"select r.received_at, r.sender, r.subject, r.gmail_msg_id, r.source,
                  r.amount_cents, r.currency, r.direction, r.merchant_raw,
                  exists(select 1 from transaction_links l where l.raw_event_id = r.id) as linked
           from raw_events r
           where r.user_id = $1
           order by r.received_at desc
           limit $2"#,
    )
    .bind(user)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(received_at, sender, subject, gmail_msg_id, source, amount_cents, currency, direction, merchant_raw, linked)| {
                ParsedEventRow {
                    received_at,
                    sender,
                    subject,
                    gmail_msg_id,
                    source,
                    amount_cents,
                    currency,
                    direction,
                    merchant_raw,
                    linked,
                }
            },
        )
        .collect())
}

pub async fn audit_discarded(pool: &PgPool, user: Uuid, limit: i64) -> Result<Vec<DiscardedEventRow>> {
    let rows: Vec<(DateTime<Utc>, String, Option<String>, String)> = sqlx::query_as(
        r#"select received_at, sender, subject, gmail_msg_id
           from discarded_events where user_id = $1
           order by received_at desc limit $2"#,
    )
    .bind(user)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(received_at, sender, subject, gmail_msg_id)| DiscardedEventRow { received_at, sender, subject, gmail_msg_id })
        .collect())
}

pub async fn transactions(pool: &PgPool, user: Uuid, limit: i64) -> Result<Vec<TxRow>> {
    let rows: Vec<(DateTime<Utc>, String, i64, String, Option<String>)> = sqlx::query_as(
        r#"select occurred_at, direction, amount_cents, currency, merchant
           from transactions where user_id = $1
           order by occurred_at desc limit $2"#,
    )
    .bind(user)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(occurred_at, direction, amount_cents, currency, merchant)| TxRow {
            occurred_at,
            direction,
            amount_cents,
            currency,
            merchant,
        })
        .collect())
}
