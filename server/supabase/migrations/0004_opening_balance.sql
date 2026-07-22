-- Real balances: an account's displayed balance = opening_balance + net of its
-- transactions. The user sets their current balance; we back-compute opening.

alter table accounts add column if not exists opening_balance_cents bigint not null default 0;
