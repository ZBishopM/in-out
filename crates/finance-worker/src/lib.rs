//! Shared finance library: DB access, reconciliation, seed, and the ingest
//! pipeline that turns notification emails into `raw_events`.

pub mod db;
pub mod ingest;
pub mod reconcile;
pub mod seed;

use uuid::Uuid;

/// The user whose finances we process. From `INOUT_USER_ID`, else the demo id.
pub fn user_id() -> Uuid {
    std::env::var("INOUT_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}
