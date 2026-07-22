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
    pub name: String,
    pub kind: String,
    pub currency: String,
    pub balance_cents: i64,
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
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        r#"select name, kind, currency, balance_cents from accounts
           where user_id = $1 order by name"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, kind, currency, balance_cents)| AccountRow { name, kind, currency, balance_cents })
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
