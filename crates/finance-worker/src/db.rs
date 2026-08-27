//! Postgres access for finance reconciliation.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use in_out_core::{merchant_sim, RawEvent, ReconcileConfig};
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

/// Find an account by (user, name), creating it if missing. Returns its id.
pub async fn ensure_account(
    pool: &PgPool,
    user: Uuid,
    name: &str,
    kind: &str,
    currency: &str,
) -> Result<Uuid> {
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("select id from accounts where user_id = $1 and name = $2")
            .bind(user)
            .bind(name)
            .fetch_optional(pool)
            .await?
    {
        return Ok(id);
    }
    let id: Uuid = sqlx::query_scalar(
        "insert into accounts (user_id, name, kind, currency) values ($1,$2,$3,$4) returning id",
    )
    .bind(user)
    .bind(name)
    .bind(kind)
    .bind(currency)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert a raw event, deduped on (user, gmail_msg_id). Returns true if a new
/// row was inserted (false if it already existed). On a duplicate,
/// `sender`/`subject` are still refreshed — those two columns were added
/// after the first rows landed, so a re-ingest is how the backlog gets them
/// filled in. `xmax = 0` is Postgres's own tell for "this row was inserted,
/// not updated" inside an upsert's RETURNING.
#[allow(clippy::too_many_arguments)]
pub async fn insert_raw_event(
    pool: &PgPool,
    user: Uuid,
    account: Option<Uuid>,
    source: &str,
    sender: &str,
    subject: &str,
    gmail_msg_id: &str,
    received_at: DateTime<Utc>,
    amount_cents: i64,
    currency: &str,
    direction: &str,
    merchant: &str,
) -> Result<bool> {
    let inserted: bool = sqlx::query_scalar(
        r#"
        insert into raw_events
          (user_id, account_id, source, sender, subject, gmail_msg_id, received_at,
           amount_cents, currency, direction, merchant_raw)
        values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        on conflict (user_id, gmail_msg_id) do update
          set sender = excluded.sender, subject = excluded.subject
        returning (xmax = 0)
        "#,
    )
    .bind(user)
    .bind(account)
    .bind(source)
    .bind(sender)
    .bind(subject)
    .bind(gmail_msg_id)
    .bind(received_at)
    .bind(amount_cents)
    .bind(currency)
    .bind(direction)
    .bind(merchant)
    .fetch_one(pool)
    .await?;
    Ok(inserted)
}

/// Log an email the parser discarded (unknown sender, or a body it couldn't
/// match) — so it shows up for manual review instead of vanishing. Deduped
/// like `insert_raw_event`; returns true if newly logged.
pub async fn insert_discarded_event(
    pool: &PgPool,
    user: Uuid,
    sender: &str,
    subject: &str,
    gmail_msg_id: &str,
    received_at: DateTime<Utc>,
) -> Result<bool> {
    let res = sqlx::query(
        r#"
        insert into discarded_events (user_id, sender, subject, gmail_msg_id, received_at)
        values ($1,$2,$3,$4,$5)
        on conflict (user_id, gmail_msg_id) do nothing
        "#,
    )
    .bind(user)
    .bind(sender)
    .bind(subject)
    .bind(gmail_msg_id)
    .bind(received_at)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Remove a stale `discarded_events` row once its email successfully parses
/// on a later ingest (a parser can be added after the fact for a template
/// that was previously unrecognized). No-op if it was never there.
pub async fn delete_discarded_event(pool: &PgPool, user: Uuid, gmail_msg_id: &str) -> Result<()> {
    sqlx::query("delete from discarded_events where user_id = $1 and gmail_msg_id = $2")
        .bind(user)
        .bind(gmail_msg_id)
        .execute(pool)
        .await?;
    Ok(())
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
    category: &str,
) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        insert into transactions
          (user_id, occurred_at, amount_cents, currency, direction, merchant, category, reconciled)
        values ($1, $2, $3, $4, $5, $6, $7, true)
        returning id
        "#,
    )
    .bind(user)
    .bind(occurred_at)
    .bind(amount_cents)
    .bind(currency)
    .bind(direction)
    .bind(merchant)
    .bind(category)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Set (or clear, with `None`) a transaction's user-written note.
pub async fn set_transaction_note(pool: &PgPool, user: Uuid, id: Uuid, note: Option<&str>) -> Result<()> {
    sqlx::query("update transactions set note = $1 where id = $2 and user_id = $3")
        .bind(note)
        .bind(id)
        .bind(user)
        .execute(pool)
        .await?;
    Ok(())
}

/// Recompute `category` for every transaction from its merchant + direction.
/// Returns the number updated. Used to backfill after category rules change.
pub async fn recategorize_all(pool: &PgPool, user: Uuid) -> Result<u64> {
    let rows: Vec<(Uuid, Option<String>, String)> =
        sqlx::query_as("select id, merchant, direction from transactions where user_id = $1")
            .bind(user)
            .fetch_all(pool)
            .await?;
    let mut n = 0u64;
    for (id, merchant, direction) in rows {
        let cat = in_out_core::categorize(merchant.as_deref().unwrap_or(""), &direction);
        sqlx::query("update transactions set category = $1 where id = $2")
            .bind(cat)
            .bind(id)
            .execute(pool)
            .await?;
        n += 1;
    }
    Ok(n)
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
        update accounts a set balance_cents = a.opening_balance_cents + coalesce((
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

/// Find an existing transaction that this candidate is a duplicate of — used to
/// catch a bank auth + settlement that arrive in *different* ingest batches
/// (the second one is compared against already-reconciled transactions, not
/// just the current batch). SQL prefilters by currency/direction/amount/window;
/// merchant similarity is checked in Rust.
#[allow(clippy::too_many_arguments)]
pub async fn find_matching_transaction(
    pool: &PgPool,
    user: Uuid,
    amount_cents: i64,
    currency: &str,
    direction: &str,
    occurred_at: DateTime<Utc>,
    merchant: &str,
    cfg: &ReconcileConfig,
) -> Result<Option<Uuid>> {
    let lo = occurred_at - Duration::seconds(cfg.window_secs);
    let hi = occurred_at + Duration::seconds(cfg.window_secs);
    let cands: Vec<(Uuid, Option<String>)> = sqlx::query_as(
        r#"select id, merchant from transactions
           where user_id = $1 and currency = $2 and direction = $3
             and abs(amount_cents - $4) <= $5
             and occurred_at between $6 and $7"#,
    )
    .bind(user)
    .bind(currency)
    .bind(direction)
    .bind(amount_cents)
    .bind(cfg.amount_tol_cents)
    .bind(lo)
    .bind(hi)
    .fetch_all(pool)
    .await?;
    for (id, m) in cands {
        if merchant_sim(merchant, m.as_deref().unwrap_or("")) >= cfg.merchant_min_sim {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Merge already-stored duplicate transactions (one-shot cleanup for data that
/// was reconciled before cross-batch matching existed). Keeps the earliest of
/// each duplicate group, moves the others' links onto it, deletes the extras.
/// Returns how many were removed.
pub async fn rededup(pool: &PgPool, user: Uuid) -> Result<u64> {
    let txs: Vec<(Uuid, String, i64, String, DateTime<Utc>, Option<String>)> = sqlx::query_as(
        r#"select id, currency, amount_cents, direction, occurred_at, merchant
           from transactions where user_id = $1 order by occurred_at"#,
    )
    .bind(user)
    .fetch_all(pool)
    .await?;

    let cfg = ReconcileConfig::default();
    let mut kept: Vec<(Uuid, String, i64, String, DateTime<Utc>, String)> = Vec::new();
    let mut removed = 0u64;

    for (id, cur, amt, dir, at, merch) in txs {
        let m = merch.unwrap_or_default();
        let hit = kept.iter().find(|(_, kcur, kamt, kdir, kat, km)| {
            kcur == &cur
                && kdir == &dir
                && (kamt - amt).abs() <= cfg.amount_tol_cents
                && (*kat - at).num_seconds().abs() <= cfg.window_secs
                && merchant_sim(&m, km) >= cfg.merchant_min_sim
        });
        if let Some((keep_id, ..)) = hit {
            let keep_id = *keep_id;
            sqlx::query(
                r#"update transaction_links l set transaction_id = $1
                   where l.transaction_id = $2
                     and not exists (select 1 from transaction_links k
                                     where k.transaction_id = $1 and k.raw_event_id = l.raw_event_id)"#,
            )
            .bind(keep_id)
            .bind(id)
            .execute(pool)
            .await?;
            sqlx::query("delete from transactions where id = $1 and user_id = $2")
                .bind(id)
                .bind(user)
                .execute(pool)
                .await?;
            removed += 1;
        } else {
            kept.push((id, cur, amt, dir, at, m));
        }
    }

    recompute_balances(pool, user).await?;
    Ok(removed)
}

/// Net (signed) of the transactions attributed to `account`.
async fn account_net(pool: &PgPool, account: Uuid) -> Result<i64> {
    let net: Option<i64> = sqlx::query_scalar(
        r#"select sum(x.signed)::bigint from (
             select distinct t.id,
               case when t.direction = 'in' then t.amount_cents else -t.amount_cents end as signed
             from transactions t
             join transaction_links l on l.transaction_id = t.id
             join raw_events r on r.id = l.raw_event_id
             where r.account_id = $1
           ) x"#,
    )
    .bind(account)
    .fetch_one(pool)
    .await?;
    Ok(net.unwrap_or(0))
}

/// Create a manual account (one with no email source). Returns its id.
#[allow(clippy::too_many_arguments)]
pub async fn create_account(
    pool: &PgPool,
    user: Uuid,
    name: &str,
    kind: &str,
    currency: &str,
    credit_limit_cents: Option<i64>,
    trea_bps: Option<i32>,
) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        r#"insert into accounts (user_id, name, kind, currency, credit_limit_cents, trea_bps)
           values ($1,$2,$3,$4,$5,$6) returning id"#,
    )
    .bind(user)
    .bind(name)
    .bind(kind)
    .bind(currency)
    .bind(credit_limit_cents)
    .bind(trea_bps)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Update an account's editable fields (not its balance).
#[allow(clippy::too_many_arguments)]
pub async fn update_account(
    pool: &PgPool,
    user: Uuid,
    id: Uuid,
    name: &str,
    kind: &str,
    currency: &str,
    credit_limit_cents: Option<i64>,
    trea_bps: Option<i32>,
) -> Result<()> {
    sqlx::query(
        r#"update accounts set name=$1, kind=$2, currency=$3,
             credit_limit_cents=$4, trea_bps=$5
           where id=$6 and user_id=$7"#,
    )
    .bind(name)
    .bind(kind)
    .bind(currency)
    .bind(credit_limit_cents)
    .bind(trea_bps)
    .bind(id)
    .bind(user)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete an account. Its raw_events keep their history (account_id is set null
/// by the FK), so no transactions are lost.
pub async fn delete_account(pool: &PgPool, user: Uuid, id: Uuid) -> Result<()> {
    sqlx::query("delete from accounts where id = $1 and user_id = $2")
        .bind(id)
        .bind(user)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set an account's *current* balance: back-compute the opening balance so that
/// opening + net = current, then refresh all balances.
pub async fn set_account_balance(pool: &PgPool, user: Uuid, account: Uuid, current_cents: i64) -> Result<()> {
    let net = account_net(pool, account).await?;
    let opening = current_cents - net;
    sqlx::query("update accounts set opening_balance_cents = $1 where id = $2 and user_id = $3")
        .bind(opening)
        .bind(account)
        .bind(user)
        .execute(pool)
        .await?;
    recompute_balances(pool, user).await?;
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
