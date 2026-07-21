# in_out — Plan de construcción

App de escritorio (Rust + Tauri 2) para (1) rankear tu lista de deseados de MercadoLibre por precio con filtros de vendedor, (2) leer tu correo y llevar tus cuentas (PayPal, BCP, Yape, Plin) con deduplicación, y (3) mostrar un dashboard de gastos. Sincroniza laptop → luego celular.

## Decisiones fijadas

| Área | Decisión |
|------|----------|
| Backend / datos / sync | **Supabase self-hosted** en agapornis (Hetzner) — auth + Postgres + REST + realtime |
| Acceso MercadoLibre | **API oficial** (OAuth): `bookmarks` + `items` + `users` (reputación). Scraping solo si la API falla |
| Frontend | **SvelteKit** dentro de Tauri |
| Orden de construcción | **MercadoLibre / compras primero**, luego finanzas, luego dashboard |
| Cliente | **Tauri 2** (Rust core reutilizable desktop + móvil), cache local SQLite |

---

## Arquitectura

```
Cliente (Tauri 2)                         Servidor agapornis (Docker Compose)
┌──────────────────────────┐             ┌────────────────────────────────────┐
│ SvelteKit (UI)           │  REST +     │ Supabase (auth, Postgres, realtime)│
│ src-tauri (Rust)         │◄─realtime──►│ n8n  (workflows lectura correo)    │
│ in_out_core (lógica)     │             │ worker-ml (cron: bookmarks→ranking)│
│ SQLite (cache offline)   │             │ Postgres = verdad única            │
└──────────────────────────┘             └────────────────────────────────────┘
```

- **Verdad única** en Postgres/Supabase. El cliente sincroniza; el celular (fase 4) usa el mismo backend → sync gratis vía realtime.
- **`in_out_core`**: crate Rust con modelos + lógica pura (ranking, cálculo "qué puedo comprar", motor de dedup). Reutilizado por desktop, worker y futuro móvil.
- **Tokens** (ML OAuth, Gmail) viven en el servidor, nunca en el cliente.

---

## Estructura de repo (workspace Rust + monorepo)

```
in_out/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ core/                   # in_out_core: modelos, ranking, dedup, cálculos (puro, sin IO)
│  ├─ ml-client/              # cliente API MercadoLibre (OAuth, bookmarks, items, sellers)
│  ├─ worker-ml/              # binario cron server-side (reusa ml-client + core)
│  └─ desktop/                # src-tauri (binario Tauri)
├─ apps/
│  └─ ui/                     # frontend SvelteKit
├─ server/
│  ├─ docker-compose.yml      # supabase + n8n + worker-ml
│  ├─ supabase/migrations/    # SQL: tablas + políticas RLS
│  └─ n8n/workflows/          # workflows exportados (JSON, versionados)
└─ PLAN.md
```

---

## Modelo de datos (Postgres)

Todas las tablas llevan `user_id` con Row-Level Security.

```
accounts(id, name, kind[paypal|bcp|yape|plin|card], currency, balance)

raw_events(id, account_id, source, gmail_msg_id, received_at,
           subject, body_hash, amount, currency,
           direction[in|out], merchant_raw, parsed jsonb, created_at)
  -- crudo, inmutable, reprocesable. Nunca se borra.

transactions(id, occurred_at, amount, currency, direction,
             merchant, category, section, reconciled bool)
  -- canónico, deduplicado. Lo que ve el dashboard.

transaction_links(transaction_id, raw_event_id, role[receipt|settlement])
  -- une p.ej. recibo Uber (receipt) + cargo tarjeta (settlement)

wishlist_items(item_id, title, permalink, added_at)

item_snapshots(item_id, captured_at, price, currency, seller_id,
               power_seller_status, seller_tx_completed, verified,
               passes_filter bool)

config(user_id, data jsonb)
  -- filtros ML (min_medal, min_sales, require_verified),
  -- budget, secciones dashboard, buckets horarios
```

---

## Módulo 1 — MercadoLibre / compras (POSPUESTO)

> **Actualización 2026-07 (hallazgos F1, validados en vivo):**
> - ✅ OAuth (authorization-code + refresh, con rotación de token) funciona.
> - ✅ `GET /users/me/bookmarks` funciona → devuelve la wishlist real (item_ids + fecha).
> - ❌ `GET /items/{id}` devuelve **403 `access_denied`** para items ajenos (política ML 2024-25), aun con scope `read`. No hay scope que lo habilite.
> - ❌ Scraping HTTP anónimo → muro anti-bot (`suspicious-traffic-frontend`).
> - ❌ Navegador headless sin sesión → rebota a login.
> - ⇒ **Precios de items ajenos requieren navegador real + IP residencial + sesión ML.** El worker en Hetzner (IP datacenter) no sirve para precios.
>
> **Decisión:** posponer el scraping de precios. Cuando se retome, el candidato es
> scraping desde el **webview de la app Tauri** (IP residencial + sesión del usuario),
> no desde el servidor. La parte de bookmarks (item_ids) sí queda funcional.
> Orden nuevo: **F2 finanzas → F3 dashboard → precios después.**

### Integración API (parcial: solo bookmarks utilizable hoy)
1. Registrar app en developers.mercadolibre.com → `client_id` / `client_secret`, redirect URI **https** (localhost/http rechazados).
2. OAuth2 authorization-code flow (una vez) → guardar `refresh_token` (ML **rota** el refresh token en cada uso → persistir el nuevo).
3. `worker-ml`:
   - `GET /users/me/bookmarks` → IDs de items → tabla `wishlist_items`. ✅
   - Por item: `GET /items/{id}` → **bloqueado (403)**. Pendiente vía webview.
   - Por vendedor: `GET /users/{seller_id}` → reputación. (No probado por el bloqueo de items.)
   - Escribe `item_snapshots` con `passes_filter` calculado.

### Filtros (configurables)
- **Medalla ≥ Gold** → `power_seller_status` en {`gold`,`platinum`} (orden silver < gold < platinum).
- **Ventas ≥ N** → `seller_reputation.transactions.completed >= N`.
- **Verificado** → aproximado: reputación en verde + medalla presente (ML no expone un flag "verified" limpio). *Marcar como heurística.*

### Ranking + barra "qué puedo comprar"
- Lista siempre ordenada **precio ascendente**, solo items con `passes_filter = true`.
- `budget` = saldo disponible (del módulo finanzas) o bolsa configurable.
- Greedy: recorre de más barato a más caro acumulando hasta `budget`.
  - Barra de progreso = `gastable / budget`.
  - Cada item marcado comprable / no comprable.
  - Muestra "cuánto falta" para el siguiente item.

### Riesgos
- Rate limits ML → todo por cache (`item_snapshots`), nunca consulta en vivo por render.
- Si `bookmarks` limita → fallback scraping de página de favoritos (crate aparte, aislado).

---

## Módulo 2 — Finanzas / correo

### Ingesta (n8n en agapornis)
- Credencial Gmail API (proyecto Google Cloud) — no scraping de correo.
- Un workflow por fuente: PayPal, BCP, Yape, Plin. Filtro por remitente/asunto.
- Parse monto / fecha / comercio: regex por plantilla; LLM-extract para correos sucios.
- Cada correo → fila en `raw_events` (idempotente por `gmail_msg_id`).

### Motor de deduplicación / reconciliación
Problema: un gasto genera 2+ correos (recibo Uber **y** cargo de tarjeta BCP).

Al llegar un `raw_event`:
1. Buscar `transaction` existente dentro de ventana (config, p.ej. ±72 h) con:
   - monto dentro de tolerancia (config), misma moneda, y
   - comercio con similitud fuzzy (trigram) alta.
2. Si hay match → `transaction_links` (uno `receipt`, otro `settlement`), **no** se crea transacción nueva.
3. Si no → nueva `transaction`.

Todo revisable manualmente. El crudo (`raw_events`) nunca se toca → reprocesable si cambian las reglas.

---

## Módulo 3 — Dashboard

- Gasto por **día**, por **bucket horario** (config: p.ej. 0-6, 6-12, 12-18, 18-24), por **sección/categoría**.
- "¿Cuánto gasté hoy?" + cortes: en qué horas gasté más.
- Todo configurable (`config.data`).
- Charts en SvelteKit (aplicar guía de dataviz al construir).

---

## Roadmap por fases

- **F0 — Scaffold**
  - Workspace Rust + app Tauri skeleton + SvelteKit.
  - `docker-compose.yml` en agapornis: Supabase + n8n.
  - Migraciones SQL + RLS. App OAuth ML registrada.
- **F1 — MercadoLibre** *(primero)*
  - OAuth ML, `worker-ml`, snapshots + filtros.
  - UI: lista ordenada + barra "qué puedo comprar".
- **F2 — Finanzas**
  - Workflows n8n por fuente, parsers, motor de reconciliación/dedup.
  - `accounts` + `transactions`.
- **F3 — Dashboard**
  - Analítica de gasto, cortes horarios/sección, config.
- **F4 — Sync + móvil**
  - Cache offline pulido, build móvil Tauri 2 (iOS/Android) reusando `in_out_core`.

---

## Riesgos transversales

- **ToS ML:** API oficial sobre tu propia cuenta = OK. Scraping = solo fallback aislado.
- **Parsers de correo:** el formato cambia → se rompen. Por eso `raw_events` inmutable + reprocesable.
- **Dedup:** falsos positivos/negativos → nunca borra crudo, siempre corregible.
- **Secretos:** tokens ML/Gmail solo en servidor.

---

## Fuentes
- MercadoLibre — Bookmarks: https://developers.mercadolibre.com.do/en_us/en_us/bookmarks
- MercadoLibre — Sellers reputation (`power_seller_status`, `transactions`): https://developers.mercadolibre.com.ar/en_us/sellers-reputation
