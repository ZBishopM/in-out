//! HTTP ingest service. n8n's Gmail workflow POSTs notification emails here;
//! we parse, persist to raw_events, and reconcile.
//!
//! Endpoints:
//!   GET  /health           -> "ok"
//!   POST /ingest           -> body: JSON array of EmailIn; returns IngestReport
//!
//! Env:
//!   DATABASE_URL       (required)
//!   INGEST_BIND        (optional) default 0.0.0.0:8090
//!   INGEST_TOKEN       (optional) if set, requires `Authorization: Bearer <token>`
//!   INOUT_USER_ID      (optional)
//!   DISCORD_WEBHOOK_URL (optional) if set, posts one message per new
//!                        transaction created by an /ingest call (the same
//!                        webhook the watchdog uses)

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use finance_worker::ingest::{ingest, EmailIn, IngestReport};
use finance_worker::reconcile::NewTransaction;
use finance_worker::{db, read, user_id};
use serde::Deserialize;

struct AppState {
    pool: sqlx::PgPool,
    token: Option<String>,
    http: reqwest::Client,
    discord_webhook: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    for p in ["server/.env", ".env", "../server/.env", "../../server/.env"] {
        if dotenvy::from_filename(p).is_ok() {
            break;
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db_url = std::env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("DATABASE_URL not set"))?;
    let pool = db::connect(&db_url).await?;
    let token = std::env::var("INGEST_TOKEN").ok();
    let discord_webhook = std::env::var("DISCORD_WEBHOOK_URL").ok();
    let state = Arc::new(AppState { pool, token, http: reqwest::Client::new(), discord_webhook });

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/audit", get(audit_page))
        .route("/health", get(|| async { "ok" }))
        .route("/api/summary", get(api_summary))
        .route("/api/daily", get(api_daily))
        .route("/api/hourly", get(api_hourly))
        .route("/api/by_category", get(api_by_category))
        .route("/api/accounts", get(api_accounts))
        .route("/api/accounts/balance", post(api_set_balance))
        .route("/api/accounts/create", post(api_create_account))
        .route("/api/accounts/update", post(api_update_account))
        .route("/api/accounts/delete", post(api_delete_account))
        .route("/api/transactions", get(api_transactions))
        .route("/api/audit/parsed", get(api_audit_parsed))
        .route("/api/audit/discarded", get(api_audit_discarded))
        .route("/api/audit/note", post(api_set_note))
        .route("/ingest", post(ingest_handler))
        .with_state(state);

    let bind = std::env::var("INGEST_BIND").unwrap_or_else(|_| "0.0.0.0:8090".into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "ingest-api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ingest_handler(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(emails): Json<Vec<EmailIn>>,
) -> Result<Json<IngestReport>, (StatusCode, String)> {
    if let Some(expected) = &st.token {
        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
        if auth != format!("Bearer {expected}") {
            return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
        }
    }
    let user = user_id();
    let report = ingest(&st.pool, user, emails)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(webhook) = &st.discord_webhook {
        for tx in &report.new_transactions {
            notify_discord(&st.http, webhook, tx).await;
        }
    }
    Ok(Json(report))
}

/// Best-effort: a failed Discord post never fails the ingest itself.
async fn notify_discord(client: &reqwest::Client, webhook: &str, tx: &NewTransaction) {
    let lima = chrono::FixedOffset::west_opt(5 * 3600).expect("valid fixed offset");
    let when = tx.occurred_at.with_timezone(&lima).format("%d %b, %I:%M %p").to_string();
    let (emoji, sign) = if tx.direction == "in" { ("💰", "+") } else { ("💸", "−") };
    let symbol = if tx.currency == "USD" { "$" } else { "S/" };
    let content = format!(
        "{emoji} **{sign} {symbol} {:.2}** · {} ({})\n{when} · <https://finanzas.danassistantassistant.website/audit>",
        tx.amount_cents as f64 / 100.0,
        tx.merchant,
        tx.category,
    );
    match client.post(webhook).json(&serde_json::json!({ "content": content })).send().await {
        // `send()` only errors on a network-level failure (timeout, DNS,
        // connection refused) -- Discord returning a 4xx/5xx still comes
        // back as `Ok(response)`, so that has to be checked separately or a
        // bad/revoked webhook fails silently.
        Ok(resp) if !resp.status().is_success() => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(%status, body, transaction_id = %tx.id, "discord notify rejected");
        }
        Err(e) => tracing::warn!(error = %e, transaction_id = %tx.id, "discord notify failed"),
        Ok(_) => {}
    }
}

// ---- dashboard + read API (public route is gated by the reverse proxy) ----

type ApiErr = (StatusCode, String);
fn err500<E: std::fmt::Display>(e: E) -> ApiErr {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../dashboard.html"))
}

async fn audit_page() -> Html<&'static str> {
    Html(include_str!("../audit.html"))
}

#[derive(Deserialize)]
struct DaysQ {
    #[serde(default = "default_days")]
    days: i32,
}
fn default_days() -> i32 {
    30
}

#[derive(Deserialize)]
struct LimitQ {
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    50
}

#[derive(Deserialize)]
struct AuditLimitQ {
    #[serde(default = "default_audit_limit")]
    limit: i64,
}
fn default_audit_limit() -> i64 {
    300
}

async fn api_summary(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::SummaryRow>>, ApiErr> {
    read::summary(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_daily(State(st): State<Arc<AppState>>, Query(q): Query<DaysQ>) -> Result<Json<Vec<read::DayRow>>, ApiErr> {
    read::daily(&st.pool, user_id(), q.days).await.map(Json).map_err(err500)
}

async fn api_hourly(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::HourRow>>, ApiErr> {
    read::hourly(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_by_category(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::CategoryRow>>, ApiErr> {
    read::by_category(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_accounts(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    read::accounts(&st.pool, user_id()).await.map(Json).map_err(err500)
}

#[derive(Deserialize)]
struct SetBalanceReq {
    account_id: uuid::Uuid,
    current_cents: i64,
}

async fn api_set_balance(
    State(st): State<Arc<AppState>>,
    Json(b): Json<SetBalanceReq>,
) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    let user = user_id();
    db::set_account_balance(&st.pool, user, b.account_id, b.current_cents).await.map_err(err500)?;
    read::accounts(&st.pool, user).await.map(Json).map_err(err500)
}

#[derive(Deserialize)]
struct CreateAccountReq {
    name: String,
    kind: String,
    currency: String,
    credit_limit_cents: Option<i64>,
    trea_bps: Option<i32>,
}

async fn api_create_account(
    State(st): State<Arc<AppState>>,
    Json(b): Json<CreateAccountReq>,
) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    let user = user_id();
    db::create_account(&st.pool, user, &b.name, &b.kind, &b.currency, b.credit_limit_cents, b.trea_bps)
        .await
        .map_err(err500)?;
    read::accounts(&st.pool, user).await.map(Json).map_err(err500)
}

#[derive(Deserialize)]
struct UpdateAccountReq {
    id: uuid::Uuid,
    name: String,
    kind: String,
    currency: String,
    credit_limit_cents: Option<i64>,
    trea_bps: Option<i32>,
}

async fn api_update_account(
    State(st): State<Arc<AppState>>,
    Json(b): Json<UpdateAccountReq>,
) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    let user = user_id();
    db::update_account(&st.pool, user, b.id, &b.name, &b.kind, &b.currency, b.credit_limit_cents, b.trea_bps)
        .await
        .map_err(err500)?;
    read::accounts(&st.pool, user).await.map(Json).map_err(err500)
}

#[derive(Deserialize)]
struct DeleteAccountReq {
    id: uuid::Uuid,
}

async fn api_delete_account(
    State(st): State<Arc<AppState>>,
    Json(b): Json<DeleteAccountReq>,
) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    let user = user_id();
    db::delete_account(&st.pool, user, b.id).await.map_err(err500)?;
    read::accounts(&st.pool, user).await.map(Json).map_err(err500)
}

async fn api_transactions(State(st): State<Arc<AppState>>, Query(q): Query<LimitQ>) -> Result<Json<Vec<read::TxRow>>, ApiErr> {
    read::transactions(&st.pool, user_id(), q.limit.clamp(1, 500)).await.map(Json).map_err(err500)
}

async fn api_audit_parsed(
    State(st): State<Arc<AppState>>,
    Query(q): Query<AuditLimitQ>,
) -> Result<Json<Vec<read::ParsedEventRow>>, ApiErr> {
    read::audit_parsed(&st.pool, user_id(), q.limit.clamp(1, 2000)).await.map(Json).map_err(err500)
}

async fn api_audit_discarded(
    State(st): State<Arc<AppState>>,
    Query(q): Query<AuditLimitQ>,
) -> Result<Json<Vec<read::DiscardedEventRow>>, ApiErr> {
    read::audit_discarded(&st.pool, user_id(), q.limit.clamp(1, 2000)).await.map(Json).map_err(err500)
}

#[derive(Deserialize)]
struct SetNoteReq {
    transaction_id: uuid::Uuid,
    note: Option<String>,
}

async fn api_set_note(State(st): State<Arc<AppState>>, Json(b): Json<SetNoteReq>) -> Result<StatusCode, ApiErr> {
    let note = b.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    db::set_transaction_note(&st.pool, user_id(), b.transaction_id, note).await.map_err(err500)?;
    Ok(StatusCode::NO_CONTENT)
}
