-- Row-Level Security. Apply on Supabase (needs auth.uid() from Supabase Auth).
-- Each user can touch only their own rows. Skip this file on a plain Postgres
-- that has no `auth.uid()` function.

alter table accounts        enable row level security;
alter table raw_events      enable row level security;
alter table transactions    enable row level security;
alter table transaction_links enable row level security;
alter table wishlist_items  enable row level security;
alter table item_snapshots  enable row level security;
alter table config          enable row level security;

-- Straightforward owner policies.
create policy owner_all on accounts       using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy owner_all on raw_events      using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy owner_all on transactions    using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy owner_all on wishlist_items  using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy owner_all on item_snapshots  using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy owner_all on config          using (user_id = auth.uid()) with check (user_id = auth.uid());

-- transaction_links has no user_id: gate through the parent transaction.
create policy owner_all on transaction_links
    using (exists (
        select 1 from transactions t
        where t.id = transaction_links.transaction_id and t.user_id = auth.uid()
    ))
    with check (exists (
        select 1 from transactions t
        where t.id = transaction_links.transaction_id and t.user_id = auth.uid()
    ));
