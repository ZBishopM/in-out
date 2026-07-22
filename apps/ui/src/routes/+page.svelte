<script lang="ts">
  import { buyPlan, progress, rank, type BuyPlan, type Filters, type ItemSnapshot, type Medal } from '$lib/plan';

  const ls = (k: string, d = '') => { try { return localStorage.getItem(k) ?? d; } catch { return d; } };
  const lset = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch {} };

  const MOCK: ItemSnapshot[] = [
    { item_id: 'MPE1', title: 'Teclado mecánico', price_cents: 12000, currency: 'PEN', seller_status: 'gold', seller_sales: 1200, verified: true }
  ];
  function loadSaved(): ItemSnapshot[] {
    try { const a = JSON.parse(ls('inout-wishlist', '')); return Array.isArray(a) && a.length ? a : MOCK; }
    catch { return MOCK; }
  }

  // Persisted wishlist (scraped prices survive restarts); replaced by 'ml-items'.
  const saved = loadSaved();
  let items = $state<ItemSnapshot[]>(saved);
  let mlStatus = $state(saved === MOCK ? '' : `${saved.length} items guardados.`);

  async function connectML() {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('scrape_ml_favorites');
    mlStatus = 'Ventana ML abierta — ve a tus Favoritos; los precios llegan solos.';
  }

  // Listen for scraped items from the ML webview; persist them.
  $effect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    let un: (() => void) | undefined;
    import('@tauri-apps/api/event').then(({ listen }) =>
      listen<ItemSnapshot[]>('ml-items', (e) => {
        if (Array.isArray(e.payload) && e.payload.length) {
          items = e.payload;
          lset('inout-wishlist', JSON.stringify(e.payload));
          mlStatus = `Recibidos ${e.payload.length} items de MercadoLibre.`;
        }
      }).then((f) => (un = f))
    );
    return () => un?.();
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
      const raw = await invoke<string>('api_get', { url: apiUrl.replace(/\/$/, '') + '/api/summary', authorization: auth });
      const rows = JSON.parse(raw) as { currency: string; direction: string; total_cents: number }[];
      const find = (c: string, d: string) => rows.find((r) => r.currency === c && r.direction === d)?.total_cents ?? 0;
      const RATE = 3.75;
      const net = (find('PEN', 'in') - find('PEN', 'out')) + (find('USD', 'in') - find('USD', 'out')) * RATE;
      budgetSoles = Math.max(0, Math.round(net / 100));
      saldoStatus = `Saldo estimado S/ ${budgetSoles} (PEN neto + USD×${RATE}).`;
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
    <button onclick={connectML}>Actualizar precios (MercadoLibre)</button>
    {#if mlStatus}<span class="ml-status">{mlStatus}</span>{/if}
  </div>

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
  .ml button:hover { filter: brightness(.95); }
  .ml-status { font-size: .78rem; color: #a9a9b3; }
  .saldo { margin-bottom: 1.25rem; font-size: .82rem; color: #a9a9b3; }
  .saldo summary { cursor: pointer; }
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
