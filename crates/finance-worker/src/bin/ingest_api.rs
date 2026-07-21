//! HTTP ingest service. n8n's Gmail workflow POSTs notification emails here;
//! we parse, persist to raw_events, and reconcile.
//!
//! Endpoints:
//!   GET  /health           -> "ok"
//!   POST /ingest           -> body: JSON array of EmailIn; returns IngestReport
//!
//! Env:
//!   DATABASE_URL   (required)
//!   INGEST_BIND    (optional) default 0.0.0.0:8090
//!   INGEST_TOKEN   (optional) if set, requires `Authorization: Bearer <token>`
//!   INOUT_USER_ID  (optional)

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use finance_worker::ingest::{ingest, EmailIn, IngestReport};
use finance_worker::{db, read, user_id};
use serde::Deserialize;

struct AppState {
    pool: sqlx::PgPool,
    token: Option<String>,
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
    let state = Arc::new(AppState { pool, token });

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/health", get(|| async { "ok" }))
        .route("/api/summary", get(api_summary))
        .route("/api/daily", get(api_daily))
        .route("/api/hourly", get(api_hourly))
        .route("/api/accounts", get(api_accounts))
        .route("/api/transactions", get(api_transactions))
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
    ingest(&st.pool, user, emails)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

// ---- dashboard + read API (public route is gated by the reverse proxy) ----

type ApiErr = (StatusCode, String);
fn err500<E: std::fmt::Display>(e: E) -> ApiErr {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../dashboard.html"))
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

async fn api_summary(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::SummaryRow>>, ApiErr> {
    read::summary(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_daily(State(st): State<Arc<AppState>>, Query(q): Query<DaysQ>) -> Result<Json<Vec<read::DayRow>>, ApiErr> {
    read::daily(&st.pool, user_id(), q.days).await.map(Json).map_err(err500)
}

async fn api_hourly(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::HourRow>>, ApiErr> {
    read::hourly(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_accounts(State(st): State<Arc<AppState>>) -> Result<Json<Vec<read::AccountRow>>, ApiErr> {
    read::accounts(&st.pool, user_id()).await.map(Json).map_err(err500)
}

async fn api_transactions(State(st): State<Arc<AppState>>, Query(q): Query<LimitQ>) -> Result<Json<Vec<read::TxRow>>, ApiErr> {
    read::transactions(&st.pool, user_id(), q.limit.clamp(1, 500)).await.map(Json).map_err(err500)
}
