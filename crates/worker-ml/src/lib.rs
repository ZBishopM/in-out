//! Shared pieces for the worker binaries (`worker`, `auth`).

pub mod db;
pub mod mock;

use uuid::Uuid;

/// Load env vars from `server/.env` (or a nearby `.env`) if present. Best-effort;
/// real env vars already set always win.
pub fn load_env() {
    for path in ["server/.env", ".env", "../server/.env", "../../server/.env"] {
        if dotenvy::from_filename(path).is_ok() {
            break;
        }
    }
}

/// Write the rotated refresh token back to the env file so the next run uses it.
/// Target defaults to `server/.env` (override with `ML_ENV_FILE`).
///
/// F2+ will store this in the DB per user instead of a flat file.
pub fn persist_refresh_token(token: &str) -> std::io::Result<()> {
    let path = std::env::var("ML_ENV_FILE").unwrap_or_else(|_| "server/.env".into());
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut replaced = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("ML_REFRESH_TOKEN=") {
                replaced = true;
                format!("ML_REFRESH_TOKEN={token}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !replaced {
        lines.push(format!("ML_REFRESH_TOKEN={token}"));
    }
    std::fs::write(&path, lines.join("\n") + "\n")
}

/// The user whose wishlist we sync. From `INOUT_USER_ID`, else a fixed demo id.
/// In F2+ this comes from the authenticated Supabase session.
pub fn user_id() -> Uuid {
    std::env::var("INOUT_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}
