# in_out

Desktop app (Rust + Tauri 2) to (1) rank your MercadoLibre wishlist by price with
seller filters, (2) read your email and keep your accounts (PayPal/BCP/Yape/Plin)
with dedup, and (3) show a spending dashboard. Laptop first, phone later.

See [PLAN.md](PLAN.md) for the full architecture and roadmap.

## Layout

```
crates/
  core/        in_out_core — pure logic (ranking, buy plan, dedup) + tests
  ml-client/   MercadoLibre API client (OAuth, bookmarks, items, sellers)
  worker-ml/   cron worker: wishlist -> priced + filtered snapshots
  desktop/     Tauri 2 app (reuses core)
apps/ui/       SvelteKit frontend
server/        Docker Compose (Postgres + n8n) + Supabase migrations
```

## Prerequisites

- **Rust** (stable) — not yet installed on this machine. Get it from
  <https://rustup.rs>.
- **Tauri CLI**: `cargo install tauri-cli --version "^2.0"`
- Node ≥ 20 + pnpm (present), Docker (present).

## Run

```
# 1. frontend deps
pnpm --dir apps/ui install

# 2. core tests (the behavioral spec)
cargo test -p in_out_core

# 3. desktop app (starts Vite + the Tauri window)
cargo tauri dev --config crates/desktop/tauri.conf.json
#   or: cd crates/desktop && cargo tauri dev

# 4. server stack (Postgres + n8n)
cd server && cp .env.example .env && docker compose up -d
```

Browser-only preview of the UI (no Rust needed):

```
pnpm --dir apps/ui dev   # http://localhost:5173
```

## Status

- **F0 (scaffold)** — done.
- **F1 (MercadoLibre)** — OAuth + wishlist (`/users/me/bookmarks`) working and
  persisting to Postgres. Item **prices are blocked**: ML returns 403 for items
  you don't own, and anti-bot blocks scraping from datacenter IPs. Price fetch is
  **deferred** to a future client-side (Tauri webview) approach. See the
  2026-07 update in [PLAN.md](PLAN.md).
- **F2 (finances)** — in progress. Reconciliation engine done: `finance-worker`
  clusters raw events into deduplicated transactions + account balances
  (verified against Postgres; Uber receipt + card charge collapse to one
  transaction). **Pending:** email ingestion via n8n (Gmail) to fill
  `raw_events`, and real parsers per source (PayPal/BCP/Yape/Plin).
- **Next: F3 (dashboard).**
