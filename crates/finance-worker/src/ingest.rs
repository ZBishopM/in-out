//! Email → raw_events pipeline: parse notification emails, upsert accounts,
//! insert raw events (deduped), then reconcile.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{db, reconcile};

/// One notification email handed in by the ingester (n8n Gmail node, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct EmailIn {
    pub gmail_msg_id: String,
    pub sender: String,
    #[serde(default)]
    pub subject: String,
    /// Plaintext body (or HTML stripped to text).
    pub text: String,
    /// When the email was received; used as the transaction time.
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestReport {
    pub received: usize,
    pub parsed: usize,
    pub inserted: usize,
    pub created_transactions: usize,
}

/// Map an `account_hint` from the parser to (display name, kind, currency).
fn account_meta(hint: &str) -> (&'static str, &'static str, &'static str) {
    match hint {
        "paypal" => ("PayPal", "paypal", "USD"),
        "bcp_debito" => ("BCP Débito", "bcp", "PEN"),
        "bcp_credito" => ("BCP Crédito", "card", "PEN"),
        "interbank_amex" => ("Interbank Amex", "card", "PEN"),
        "interbank" => ("Interbank", "interbank", "PEN"),
        "sip" => ("Sip", "card", "PEN"),
        "scotiabank" => ("Scotiabank", "scotiabank", "PEN"),
        _ => ("Otros", "other", "PEN"),
    }
}

/// Parse + persist a batch of emails, then reconcile. Unparseable emails
/// (marketing, unknown senders) are logged to `discarded_events` — not
/// dropped silently, so the dashboard's audit view can show them for manual
/// review.
pub async fn ingest(pool: &PgPool, user: Uuid, emails: Vec<EmailIn>) -> Result<IngestReport> {
    let received = emails.len();
    let mut parsed = 0;
    let mut inserted = 0;

    for e in &emails {
        let Some(p) = email_parse::parse(&e.sender, &e.subject, &e.text) else {
            db::insert_discarded_event(pool, user, &e.sender, &e.subject, &e.gmail_msg_id, e.received_at).await?;
            continue;
        };
        parsed += 1;
        // A parser may get added for a template that was previously logged
        // as discarded (this is how the two Scotiabank ones above were
        // found); clear the stale entry so it doesn't linger next to the
        // now-successful raw_event.
        db::delete_discarded_event(pool, user, &e.gmail_msg_id).await?;

        let (name, kind, currency) = account_meta(&p.account_hint);
        let account = db::ensure_account(pool, user, name, kind, currency).await?;

        if db::insert_raw_event(
            pool,
            user,
            Some(account),
            &p.source,
            &e.sender,
            &e.subject,
            &e.gmail_msg_id,
            e.received_at,
            p.amount_cents,
            &p.currency,
            &p.direction,
            &p.merchant,
        )
        .await?
        {
            inserted += 1;
        }
    }

    let created_transactions = reconcile::reconcile(pool, user).await?;
    Ok(IngestReport { received, parsed, inserted, created_transactions })
}
