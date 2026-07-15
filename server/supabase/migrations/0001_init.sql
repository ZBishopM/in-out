-- in_out schema. Plain SQL; runs on Supabase's Postgres (or any Postgres 14+).
-- Money is stored as integer cents. Every user-owned row carries user_id.

create extension if not exists pg_trgm;

-- Accounts: PayPal / BCP / Yape / Plin / credit card.
create table if not exists accounts (
    id            uuid primary key default gen_random_uuid(),
    user_id       uuid not null,
    name          text not null,
    kind          text not null check (kind in ('paypal','bcp','yape','plin','card')),
    currency      text not null default 'PEN',
    balance_cents bigint not null default 0,
    created_at    timestamptz not null default now()
);

-- Raw parsed email events. Immutable, reprocessable — never mutated in place.
create table if not exists raw_events (
    id           uuid primary key default gen_random_uuid(),
    user_id      uuid not null,
    account_id   uuid references accounts(id) on delete set null,
    source       text not null,                 -- 'paypal' | 'bcp' | 'yape' | 'plin' | 'uber' | ...
    gmail_msg_id text not null,
    received_at  timestamptz not null,
    subject      text,
    body_hash    text,
    amount_cents bigint not null,
    currency     text not null default 'PEN',
    direction    text not null check (direction in ('in','out')),
    merchant_raw text,
    parsed       jsonb not null default '{}'::jsonb,
    created_at   timestamptz not null default now(),
    unique (user_id, gmail_msg_id)
);
create index if not exists raw_events_user_time on raw_events (user_id, received_at);

-- Canonical, deduplicated transactions. What the dashboard reads.
create table if not exists transactions (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null,
    occurred_at timestamptz not null,
    amount_cents bigint not null,
    currency    text not null default 'PEN',
    direction   text not null check (direction in ('in','out')),
    merchant    text,
    category    text,
    section     text,
    reconciled  boolean not null default false,
    created_at  timestamptz not null default now()
);
create index if not exists transactions_user_time on transactions (user_id, occurred_at);

-- Links a canonical transaction to the raw emails that evidence it.
-- e.g. Uber receipt (receipt) + BCP card charge (settlement).
create table if not exists transaction_links (
    transaction_id uuid not null references transactions(id) on delete cascade,
    raw_event_id   uuid not null references raw_events(id) on delete cascade,
    role           text not null check (role in ('receipt','settlement')),
    primary key (transaction_id, raw_event_id)
);

-- MercadoLibre wishlist (from the bookmarks API).
create table if not exists wishlist_items (
    user_id   uuid not null,
    item_id   text not null,
    title     text,
    permalink text,
    added_at  timestamptz not null default now(),
    primary key (user_id, item_id)
);

-- Per-poll price + seller signal snapshots. History kept for trends.
create table if not exists item_snapshots (
    id                  bigint generated always as identity primary key,
    user_id             uuid not null,
    item_id             text not null,
    captured_at         timestamptz not null default now(),
    price_cents         bigint not null,
    currency            text not null default 'PEN',
    seller_id           bigint,
    power_seller_status text,                    -- 'silver' | 'gold' | 'platinum' | null
    seller_tx_completed integer not null default 0,
    verified            boolean not null default false,
    passes_filter       boolean not null default false
);
create index if not exists item_snapshots_latest
    on item_snapshots (user_id, item_id, captured_at desc);

-- Per-user config blob: filters, budget, dashboard sections, hour buckets.
create table if not exists config (
    user_id    uuid primary key,
    data       jsonb not null default '{}'::jsonb,
    updated_at timestamptz not null default now()
);
