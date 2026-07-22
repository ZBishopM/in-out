<script lang="ts">
  import { buyPlan, progress, rank, type BuyPlan, type Filters, type ItemSnapshot, type Medal } from '$lib/plan';

  const ls = (k: string, d = '') => { try { return localStorage.getItem(k) ?? d; } catch { return d; } };
  const lset = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch {} };

  type Src = 'ml' | 'fb';
  const MOCK: ItemSnapshot[] = [
    { item_id: 'MPE1', title: '(sin wishlist aún — actualiza ML o FB)', price_cents: 0, currency: 'PEN', seller_status: 'gold', seller_sales: 1200, verified: true }
  ];
  function loadSaved(src: Src): ItemSnapshot[] {
    const legacy = src === 'ml' ? ls('inout-wishlist') : '';
    try { const a = JSON.parse(ls('inout-wishlist-' + src) || legacy || '[]'); return Array.isArray(a) ? a : []; }
    catch { return []; }
  }

  // Per-source scraped items, merged into `items` for ranking / buy plan.
  let srcItems = $state<Record<Src, ItemSnapshot[]>>({ ml: loadSaved('ml'), fb: loadSaved('fb') });
  const merged = $derived([...srcItems.ml, ...srcItems.fb]);
  const items = $derived(merged.length ? merged : MOCK);

  // Saved page URL per source → refresh hidden (no visible window).
  let favUrls = $state<Record<Src, string>>({
    ml: ls('inout-fav-ml') || ls('inout-fav-url'),
    fb: ls('inout-fav-fb')
  });
  let mlStatus = $state('');
  let fbStatus = $state('');
  const setStatus = (src: Src, m: string) => { if (src === 'ml') mlStatus = m; else fbStatus = m; };

  async function refresh(src: Src) {
    const { invoke } = await import('@tauri-apps/api/core');
    const fav = favUrls[src];
    if (fav) {
      setStatus(src, 'Actualizando en segundo plano…');
      await invoke('open_scraper', { source: src, url: fav, hidden: true });
    } else {
      setStatus(src, src === 'ml' ? 'Inicia sesión en ML y abre Favoritos (1ª vez).' : 'Inicia sesión en FB y abre Marketplace (1ª vez).');
      await invoke('open_scraper', { source: src, url: null, hidden: false });
    }
  }

  // Listen for scraped items + login-needed for both sources.
  $effect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    const uns: Array<() => void> = [];
    import('@tauri-apps/api/event').then(async ({ listen }) => {
      for (const src of ['ml', 'fb'] as Src[]) {
        uns.push(
          await listen<{ url: string; items: ItemSnapshot[] }>(`${src}-items`, (e) => {
            const p = e.payload;
            if (p && Array.isArray(p.items) && p.items.length) {
              srcItems = { ...srcItems, [src]: p.items };
              lset('inout-wishlist-' + src, JSON.stringify(p.items));
              if (p.url) { favUrls = { ...favUrls, [src]: p.url }; lset('inout-fav-' + src, p.url); }
              setStatus(src, `${p.items.length} items actualizados.`);
            }
          })
        );
        uns.push(
          await listen<string>(`${src}-needs-login`, async () => {
            setStatus(src, 'Necesitas iniciar sesión — abriendo…');
            const { invoke } = await import('@tauri-apps/api/core');
            await invoke('open_scraper', { source: src, url: null, hidden: false });
          })
        );
      }
    });
    return () => uns.forEach((u) => u());
  });

  // Auto-refresh (hidden) once on open for any source with a saved URL.
  let autoRan = false;
  $effect(() => {
    if (autoRan || typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    autoRan = true;
    if (favUrls.ml) refresh('ml');
    if (favUrls.fb) refresh('fb');
  });

  // Real balance from the dashboard API (settings persisted locally).
  let apiUrl = $state(ls('inout-api-url', 'https://finanzas.danassistantassistant.website'));
  let apiUser = $state(ls('inout-api-user'));
  let apiPass = $state(ls('inout-api-pass'));
  let saldoStatus = $state('');

  async function loadBalance() {
    lset('inout-api-url', apiUrl); lset('inout-api-user', apiUser); lset('inout-api-pass', apiPass);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const auth = apiUser ? 'Basic ' + btoa(`${apiUser}:${apiPass}`) : '';
      const raw = await invoke<string>('api_get', { url: apiUrl.replace(/\/$/, '') + '/api/accounts', authorization: auth });
      const accts = JSON.parse(raw) as { kind: string; currency: string; balance_cents: number }[];
      const RATE = 3.75;
      // Available cash = non-card accounts (debit/wallet/PayPal), PEN-equivalent.
      const cents = accts
        .filter((a) => a.kind !== 'card')
        .reduce((s, a) => s + a.balance_cents * (a.currency === 'USD' ? RATE : 1), 0);
      budgetSoles = Math.max(0, Math.round(cents / 100));
      saldoStatus = `Disponible S/ ${budgetSoles} (cuentas sin tarjetas, USD×${RATE}). Ajusta saldos reales en el dashboard.`;
    } catch (e) {
      saldoStatus = 'Error: ' + e;
    }
  }

  let budgetSoles = $state(Number(ls('inout-budget', '300')) || 300);
  $effect(() => { lset('inout-budget', String(budgetSoles)); });
  let minMedal = $state<Medal | ''>('gold');
  let minSales = $state(100);
  let requireVerified = $state(false);

  const filters = $derived<Filters>({
    min_medal: minMedal || null,
    min_sales: Number.isFinite(minSales) ? minSales : null,
    require_verified: requireVerified
  });

  const budgetCents = $derived(Math.round(budgetSoles * 100));

  // Local compute (browser preview). Inside Tauri, prefer the Rust command.
  const localPlan = $derived(buyPlan(rank(items, filters), budgetCents));
  let tauriPlan = $state<BuyPlan | null>(null);

  $effect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    // Read deps synchronously so Svelte tracks them (items included — it was
    // read only inside the async .then() before, so updates weren't picked up).
    const its = items;
    const f = filters;
    const b = budgetCents;
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<BuyPlan>('plan_purchases', { items: its, filters: f, budgetCents: b }))
      .then((p) => (tauriPlan = p))
      .catch((e) => console.error('plan_purchases failed', e));
  });

  const plan = $derived(tauriPlan ?? localPlan);
  const pct = $derived(Math.round(progress(plan) * 100));

  const soles = (cents: number) => `S/ ${(cents / 100).toFixed(2)}`;
</script>

<main>
  <h1>in_out — ¿qué puedo comprar?</h1>
  <p class="src">Fuente: {tauriPlan ? 'Rust (Tauri)' : 'preview navegador'}</p>

  <div class="ml">
    <button onclick={() => refresh('ml')}>Actualizar MercadoLibre</button>
    <button class="fb" onclick={() => refresh('fb')}>Actualizar Facebook</button>
  </div>
  {#if mlStatus}<p class="ml-status">ML: {mlStatus}</p>{/if}
  {#if fbStatus}<p class="ml-status">FB: {fbStatus}</p>{/if}

  <details class="saldo">
    <summary>Usar mi saldo real (dashboard)</summary>
    <div class="saldo-form">
      <input placeholder="URL API" bind:value={apiUrl} />
      <input placeholder="usuario" bind:value={apiUser} />
      <input type="password" placeholder="contraseña" bind:value={apiPass} />
      <button onclick={loadBalance}>Cargar saldo</button>
    </div>
    {#if saldoStatus}<span class="ml-status">{saldoStatus}</span>{/if}
  </details>

  <section class="controls">
    <label>Presupuesto (S/)
      <input type="number" min="0" step="10" bind:value={budgetSoles} />
    </label>
    <label>Medalla mín.
      <select bind:value={minMedal}>
        <option value="">cualquiera</option>
        <option value="silver">Silver</option>
        <option value="gold">Gold</option>
        <option value="platinum">Platinum</option>
      </select>
    </label>
    <label>Ventas mín.
      <input type="number" min="0" step="50" bind:value={minSales} />
    </label>
    <label class="chk">
      <input type="checkbox" bind:checked={requireVerified} /> Solo verificados
    </label>
  </section>

  <div class="bar" role="progressbar" aria-valuenow={pct} aria-valuemin="0" aria-valuemax="100">
    <div class="fill" style="width: {pct}%"></div>
    <span class="bar-label">{pct}% · {soles(plan.spent_cents)} de {soles(plan.budget_cents)}</span>
  </div>
  <p class="hint">
    Comprables ya: {plan.items.filter((a) => a.affordable).length}/{plan.items.length}.
    {#if plan.next_gap_cents !== null}
      Faltan {soles(plan.next_gap_cents)} para el siguiente.
    {/if}
  </p>

  <ul class="list">
    {#each plan.items as a (a.item.item_id)}
      <li class:affordable={a.affordable}>
        <span class="mark">{a.affordable ? '✓' : '·'}</span>
        <span class="title">{a.item.title}</span>
        <span class="meta">{a.item.seller_status ?? '—'} · {a.item.seller_sales} ventas</span>
        <span class="price">{soles(a.item.price_cents)}</span>
      </li>
    {/each}
    {#if plan.items.length === 0}
      <li class="empty">Ningún item pasa los filtros.</li>
    {/if}
  </ul>
</main>

<style>
  :global(body) { margin: 0; }
  main {
    font-family: system-ui, sans-serif;
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem 1.25rem;
    color: #e7e7ea;
    background: #16161a;
    min-height: 100vh;
    box-sizing: border-box;
  }
  h1 { font-size: 1.4rem; margin: 0 0 .25rem; }
  .src { margin: 0 0 .75rem; font-size: .8rem; color: #8a8a94; }
  .ml { display: flex; align-items: center; gap: .75rem; margin-bottom: 1.25rem; flex-wrap: wrap; }
  .ml button { background: #ffe600; color: #2d3277; border: none; border-radius: 8px; padding: .55rem .9rem; font-weight: 700; cursor: pointer; }
  .ml button.fb { background: #1877f2; color: #fff; }
  .ml button:hover { filter: brightness(.95); }
  .ml-status { font-size: .78rem; color: #a9a9b3; }
  .saldo { margin-bottom: 1.25rem; font-size: .82rem; color: #a9a9b3; }
  .saldo summary { cursor: pointer; display: inline-block; color: #e7e7ea; background: #26262e; border: 1px solid #3a3a44; border-radius: 8px; padding: .5rem .8rem; font-weight: 600; list-style: none; }
  .saldo summary::-webkit-details-marker { display: none; }
  .saldo summary::before { content: '⚙ '; }
  .saldo[open] summary { margin-bottom: .3rem; }
  .saldo-form { display: flex; flex-wrap: wrap; gap: .5rem; margin: .6rem 0; }
  .saldo-form input { background: #26262e; color: #e7e7ea; border: 1px solid #3a3a44; border-radius: 6px; padding: .4rem .5rem; }
  .saldo-form button { background: #3b82f6; color: #fff; border: none; border-radius: 6px; padding: .4rem .8rem; cursor: pointer; font-weight: 600; }
  .controls { display: flex; flex-wrap: wrap; gap: 1rem; margin-bottom: 1.25rem; }
  label { display: flex; flex-direction: column; font-size: .78rem; color: #a9a9b3; gap: .3rem; }
  label.chk { flex-direction: row; align-items: center; align-self: end; }
  input, select {
    background: #26262e; color: #e7e7ea; border: 1px solid #3a3a44;
    border-radius: 6px; padding: .4rem .5rem; font-size: .9rem;
  }
  input[type='number'] { width: 6rem; }
  .bar {
    position: relative; height: 30px; border-radius: 8px;
    background: #26262e; overflow: hidden; margin-bottom: .4rem;
  }
  .fill { height: 100%; background: linear-gradient(90deg, #3b82f6, #22c55e); transition: width .25s ease; }
  .bar-label {
    position: absolute; inset: 0; display: flex; align-items: center;
    justify-content: center; font-size: .8rem; font-weight: 600;
  }
  .hint { font-size: .8rem; color: #a9a9b3; margin: 0 0 1rem; }
  .list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: .4rem; }
  li {
    display: grid; grid-template-columns: 1.2rem 1fr auto; align-items: center;
    gap: .25rem .75rem; padding: .6rem .8rem; border-radius: 8px;
    background: #1f1f26; opacity: .55;
  }
  li.affordable { opacity: 1; outline: 1px solid #22c55e44; }
  .mark { color: #22c55e; font-weight: 700; }
  .title { font-weight: 600; }
  .meta { grid-column: 2; font-size: .74rem; color: #8a8a94; }
  .price { font-variant-numeric: tabular-nums; font-weight: 600; }
  .empty { justify-items: center; text-align: center; color: #8a8a94; opacity: 1; }
</style>
