-- Ingest audit trail: which email produced a raw_event, and which ones the
-- parser discarded (unknown sender, or a template it doesn't recognize) — so
-- the dashboard can show a human-checkable list instead of only the end
-- result. raw_events had `subject` from the start but never `sender`; add
-- it, both were previously written by nothing. discarded_events is new: an
-- immutable log of every email that reached the ingester but never became a
-- raw_event, so nothing gets silently dropped without a trace.

alter table raw_events add column if not exists sender text;

create table if not exists discarded_events (
    id           uuid primary key default gen_random_uuid(),
    user_id      uuid not null,
    sender       text not null,
    subject      text,
    gmail_msg_id text not null,
    received_at  timestamptz not null,
    created_at   timestamptz not null default now(),
    unique (user_id, gmail_msg_id)
);
create index if not exists discarded_events_user_time on discarded_events (user_id, received_at);
