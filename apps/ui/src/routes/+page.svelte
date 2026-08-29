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
      const accts = JSON.parse(raw) as { currency: string; balance_cents: number; credit_limit_cents: number | null }[];
      const RATE = 3.75;
      // Available cash = accounts without a credit line (debit/wallet/PayPal), PEN-equivalent.
      const cents = accts
        .filter((a) => a.credit_limit_cents == null)
        .reduce((s, a) => s + a.balance_cents * (a.currency === 'USD' ? RATE : 1), 0);
      budgetSoles = Math.max(0, Math.round(cents / 100));
      saldoStatus = `Disponible S/ ${budgetSoles} (cuentas sin tarjetas, USD×${RATE}). Ajusta saldos reales en el dashboard.`;
    } catch (e) {
      saldoStatus = 'Error: ' + e;
    }
  }

  let budgetSoles = $state(Number(ls('inout-budget', '300')) || 300);
  $effect(() => { lset('inout-budget', String(budgetSoles)); });
  // Permissive by default: scraped items may lack seller reputation, so don't
  // hide them. Tighten these to require a MercadoLíder medal / sales / verified.
  let minMedal = $state<Medal | ''>((ls('inout-minmedal') as Medal) || '');
  let minSales = $state(Number(ls('inout-minsales', '0')) || 0);
  let requireVerified = $state(ls('inout-verified') === '1');
  $effect(() => { lset('inout-minmedal', minMedal); lset('inout-minsales', String(minSales)); lset('inout-verified', requireVerified ? '1' : '0'); });

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
  <header class="head">
    <span class="brand">in_out</span>
    <h1>¿qué puedo comprar?</h1>
  </header>

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
  </div>
  <p class="bar-label">{pct}% · {soles(plan.spent_cents)} de {soles(plan.budget_cents)}</p>
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
  :global(body) { margin: 0; background: #0a0a0a; }

  /* scrollbars — neutral ink, no color cast */
  :global(*) { scrollbar-width: thin; scrollbar-color: rgb(255 255 255 / 18%) transparent; }
  :global(::-webkit-scrollbar) { width: 11px; height: 11px; }
  :global(::-webkit-scrollbar-track) { background: transparent; }
  :global(::-webkit-scrollbar-thumb) { background: rgb(255 255 255 / 18%); border-radius: 999px; border: 3px solid transparent; background-clip: padding-box; }
  :global(::-webkit-scrollbar-thumb:hover) { background: rgb(255 255 255 / 30%); background-clip: padding-box; }
  :global(::-webkit-scrollbar-corner) { background: transparent; }

  main {
    /* scandinavian-design dark-canvas tokens — near-black canvas, white ink,
       alpha ladder inverted per the skill's dark-canvas guidance (not a 1:1
       flip of the light percentages: interaction fills run ~1.5x, tertiary
       lifted to 50% since prices/status here are real content, not glyphs) */
    --canvas: #0a0a0a;
    --ink: #ffffff;
    --ink-secondary: rgb(255 255 255 / 56%);
    --ink-tertiary: rgb(255 255 255 / 50%);
    --border: rgb(255 255 255 / 11%);
    --border-strong: rgb(255 255 255 / 18%);
    --hover-fill: rgb(255 255 255 / 9%);
    --pressed-fill: rgb(255 255 255 / 14%);
    --radius: 10px;
    --radius-pill: 999px;

    font-family: system-ui, "Segoe UI", sans-serif;
    max-width: 560px;
    margin: 0 auto;
    padding: 2.25rem 1.5rem 3rem;
    color: var(--ink);
    background: var(--canvas);
    min-height: 100vh;
    box-sizing: border-box;
  }

  /* header */
  .head { margin-bottom: 1.75rem; }
  .brand {
    display: inline-block; font-size: .72rem; font-weight: 600;
    letter-spacing: .12em; color: var(--ink-secondary);
    margin-bottom: .3rem;
  }
  h1 { font-size: 1.7rem; font-weight: 600; letter-spacing: -.02em; margin: 0; line-height: 1.1; color: var(--ink); }

  /* shared control primitives — flat surface, hairline border */
  .ml button, .saldo summary, .saldo-form button, input, select {
    font-family: inherit;
    background: transparent;
    color: var(--ink);
    border: 1px solid var(--border);
    outline: none;
  }
  button { cursor: pointer; }
  button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible {
    outline: 2px solid var(--ink); outline-offset: 2px;
  }

  /* source-actions row — pill button family, consistent across all buttons */
  .ml { display: flex; gap: .8rem; margin-bottom: .9rem; flex-wrap: wrap; }
  .ml button {
    border-radius: var(--radius-pill); padding: .6rem 1.1rem; font-weight: 500; font-size: .9rem;
    transition: background-color .18s ease, border-color .18s ease;
  }
  .ml button::before { content: '↻ '; color: currentColor; }
  .ml button:hover { background: var(--hover-fill); border-color: var(--border-strong); }
  .ml button:active { background: var(--pressed-fill); }
  .ml-status { font-size: .76rem; color: var(--ink-secondary); margin: .15rem 0; }

  /* saldo disclosure — same pill family as the source buttons */
  .saldo { margin: 1rem 0 1.5rem; font-size: .82rem; color: var(--ink-secondary); }
  .saldo summary {
    list-style: none; display: inline-block; border-radius: var(--radius-pill);
    padding: .5rem .95rem; font-weight: 500; color: var(--ink);
    transition: background-color .18s ease, border-color .18s ease;
  }
  .saldo summary:hover { background: var(--hover-fill); border-color: var(--border-strong); }
  .saldo summary::-webkit-details-marker { display: none; }
  .saldo summary::before { content: '◐  '; color: currentColor; }
  .saldo[open] summary { margin-bottom: .8rem; }
  .saldo-form { display: flex; flex-wrap: wrap; gap: .6rem; margin: .2rem 0; }
  .saldo-form input { flex: 1 1 8rem; }
  .saldo-form button {
    border-radius: var(--radius-pill); padding: .5rem 1rem; font-weight: 500;
    transition: background-color .18s ease, border-color .18s ease;
  }
  .saldo-form button:hover { background: var(--hover-fill); border-color: var(--border-strong); }
  .saldo-form button:active { background: var(--pressed-fill); }

  /* controls */
  .controls { display: flex; flex-wrap: wrap; gap: 1rem 1.1rem; margin-bottom: 1.75rem; }
  label {
    display: flex; flex-direction: column; gap: .4rem;
    font-size: .68rem; letter-spacing: .04em; color: var(--ink-secondary);
  }
  label.chk { flex-direction: row; align-items: center; gap: .5rem; align-self: end; letter-spacing: 0; font-size: .82rem; }
  input, select {
    border-radius: var(--radius); padding: .55rem .7rem; font-size: .9rem; color: var(--ink);
  }
  input[type='number'] { width: 6.5rem; font-variant-numeric: tabular-nums; }
  input[type='checkbox'] { width: 1.15rem; height: 1.15rem; accent-color: var(--ink); border: none; }
  select { appearance: none; }
  option { background: #0a0a0a; }

  /* the affordability meter — flat fill, no glow, label lives below (was
     centered inside the bar; unreadable once the neomorphic glow that gave
     it contrast against the fill is gone) */
  .bar {
    position: relative; height: 32px; border-radius: var(--radius-pill); overflow: hidden;
    background: var(--canvas); border: 1px solid var(--border); margin-bottom: .5rem;
  }
  .fill {
    height: 100%; border-radius: var(--radius-pill);
    background: var(--ink);
    transition: width .28s ease-out;
  }
  .bar-label {
    font-size: .8rem; color: var(--ink-secondary); margin: 0 0 1.5rem;
    font-variant-numeric: tabular-nums;
  }
  .hint { font-size: .8rem; color: var(--ink-secondary); margin: 0 0 1.5rem; }

  /* wishlist */
  .list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: .7rem; }
  li {
    display: grid; grid-template-columns: 1.6rem 1fr auto; align-items: center;
    gap: .2rem .8rem; padding: .85rem 1.05rem; border-radius: var(--radius);
    border: 1px solid var(--border);
    transition: border-color .2s ease;
  }
  li.affordable { border-color: var(--border-strong); }
  .mark { font-size: 1rem; font-weight: 600; color: var(--ink-tertiary); grid-row: span 2; }
  li.affordable .mark { color: var(--ink); }
  .title { font-weight: 500; color: var(--ink-secondary); }
  li.affordable .title { color: var(--ink); }
  .meta { grid-column: 2; font-size: .72rem; color: var(--ink-tertiary); }
  .price { font-variant-numeric: tabular-nums; font-weight: 600; color: var(--ink-tertiary); grid-row: span 2; }
  li.affordable .price { color: var(--ink); }
  .empty { display: block; text-align: center; color: var(--ink-secondary); }

  /* entrance cascade — kept (state indication that the panel loaded), trimmed
     under 300ms per the skill's ordinary-motion ceiling */
  @keyframes rise { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: none; } }
  .head, .ml, .ml-status, .saldo, .controls, .bar, .bar-label, .hint, .list {
    animation: rise .28s ease-out both;
  }
  .head { animation-delay: .02s } .ml { animation-delay: .06s }
  .saldo { animation-delay: .1s } .controls { animation-delay: .14s }
  .bar { animation-delay: .18s } .bar-label { animation-delay: .2s }
  .hint { animation-delay: .22s } .list { animation-delay: .24s }

  @media (prefers-reduced-motion: reduce) {
    *, ::before, ::after { animation: none !important; transition: none !important; }
  }
</style>
