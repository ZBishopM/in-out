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

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use finance_worker::ingest::{ingest, EmailIn, IngestReport};
use finance_worker::{db, user_id};

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
        .route("/health", get(|| async { "ok" }))
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
