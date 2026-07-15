# Server (agapornis / Hetzner)

## Bring up the dev stack

```
cd server
cp .env.example .env      # fill in passwords + ML creds
docker compose up -d
```

- Postgres → `localhost:5432` (db `inout`, user `inout`)
- n8n → `http://localhost:5678`

## Apply the schema

```
# schema (works on any Postgres)
docker compose exec -T postgres psql -U inout -d inout < supabase/migrations/0001_init.sql
# RLS — only on Supabase (needs auth.uid())
```

## Supabase self-hosted

For auth + auto-generated REST + realtime sync (laptop ↔ phone), run the full
Supabase stack from the upstream repo and point it at a dedicated Postgres:

```
git clone --depth 1 https://github.com/supabase/supabase
cd supabase/docker
cp .env.example .env       # set POSTGRES_PASSWORD, JWT_SECRET, ANON_KEY, etc.
docker compose up -d
```

Then apply both migrations (`0001_init.sql`, `0002_rls.sql`) against the
Supabase Postgres. The Tauri client talks to the Supabase REST/realtime
endpoints; the `worker-ml` job writes `wishlist_items` + `item_snapshots`.

> Keep this dev `docker-compose.yml` for Postgres + n8n, or fold n8n into the
> Supabase compose network once it's up — your call.
