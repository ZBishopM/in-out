-- Free-text note the user attaches to a transaction from the /audit page
-- (e.g. after a Discord ping tells them a new payment was ingested and they
-- want to remember what it was for before it's just a number in a list).

alter table transactions add column if not exists note text;
