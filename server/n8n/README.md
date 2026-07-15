# n8n — email ingestion (F2)

n8n runs at `http://<agapornis>:5678` (basic auth from `server/.env`).

## Per-source workflows

One workflow per money source. Each follows the same shape:

```
Gmail Trigger (label/search filter)
  -> Function/Code: parse amount, currency, date, merchant  (regex per template)
  -> HTTP Request: upsert into Postgres `raw_events` (dedupe on gmail_msg_id)
```

Sources to build: **PayPal, BCP, Yape, Plin** (plus merchant receipts like Uber
for reconciliation).

## Gmail credential

Use n8n's **Gmail OAuth2** credential (a Google Cloud project with the Gmail API
enabled), scope `gmail.readonly`. Do not scrape mail or store the password.

## Reconciliation

Dedup logic lives in `in_out_core::finance` (the Rust spec + tests). Either:
- port it into an n8n Code node, or
- have n8n only write `raw_events` and run reconciliation in a small Rust job.

Export finished workflows to `server/n8n/workflows/*.json` and commit them.
