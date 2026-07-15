//! Shared pieces for the worker binaries (`worker`, `auth`).

pub mod db;
pub mod mock;

use uuid::Uuid;

/// The user whose wishlist we sync. From `INOUT_USER_ID`, else a fixed demo id.
/// In F2+ this comes from the authenticated Supabase session.
pub fn user_id() -> Uuid {
    std::env::var("INOUT_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}
