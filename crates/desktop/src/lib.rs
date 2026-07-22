//! Tauri app: exposes core logic to the SvelteKit frontend, and drives an
//! embedded MercadoLibre webview to scrape wishlist prices (the ML API blocks
//! item reads, so we scrape from a real, logged-in webview on the user's box).

use in_out_core::{buy_plan, rank, BuyPlan, Filters, ItemSnapshot};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Rank a wishlist by the given filters and compute the buy plan for `budget_cents`.
#[tauri::command]
fn plan_purchases(items: Vec<ItemSnapshot>, filters: Filters, budget_cents: i64) -> BuyPlan {
    let ranked = rank(&items, &filters);
    buy_plan(&ranked, budget_cents)
}

/// Injected into the ML webview. Adds a button that scrapes the current page's
/// product cards (title + price + link), shows a quick on-page confirmation, and
/// emits the list to the app via the `ml-items` event.
const ML_INIT: &str = r#"
(function () {
  function extract() {
    const out = [];
    document.querySelectorAll('.andes-money-amount__fraction').forEach(function (el) {
      const card = el.closest('li, .andes-card, .poly-card, .ui-search-result, .ui-search-layout__item');
      if (!card) return;
      // avoid counting the same card twice (installments / crossed-out prices)
      if (card.dataset.inoutSeen) return;
      card.dataset.inoutSeen = '1';
      const link = card.querySelector('a[href]');
      const title = card.querySelector('h2, .poly-component__title, .ui-search-item__title');
      const digits = el.textContent.replace(/[^0-9]/g, '');
      if (!digits) return;
      out.push({
        item_id: (link && link.href.match(/(ML[A-Z]-?\d+)/) || [,''])[1] || (link ? link.href : ''),
        title: title ? title.textContent.trim() : '(sin título)',
        price_cents: parseInt(digits, 10) * 100,
        currency: 'PEN',
        seller_status: 'gold',
        seller_sales: 100,
        verified: true,
        permalink: link ? link.href : ''
      });
    });
    document.querySelectorAll('[data-inout-seen]').forEach(function (c) { delete c.dataset.inoutSeen; });
    return out;
  }
  function toast(msg) {
    let t = document.getElementById('inout-toast');
    if (!t) { t = document.createElement('div'); t.id = 'inout-toast';
      t.style.cssText = 'position:fixed;z-index:100000;bottom:80px;right:20px;background:#111;color:#fff;padding:10px 14px;border-radius:8px;font:14px system-ui;box-shadow:0 4px 12px rgba(0,0,0,.4)';
      document.body.appendChild(t); }
    t.textContent = msg;
  }
  function send() {
    const items = extract();
    const total = items.reduce(function (s, i) { return s + i.price_cents; }, 0);
    toast('in_out: ' + items.length + ' items · S/ ' + (total / 100).toFixed(2));
    try { window.__TAURI__.event.emit('ml-items', items); } catch (e) { toast('in_out error: ' + e); }
  }
  window.addEventListener('load', function () {
    const b = document.createElement('button');
    b.textContent = '➡ Enviar precios a in_out';
    b.style.cssText = 'position:fixed;z-index:100000;bottom:20px;right:20px;padding:12px 18px;background:#3b82f6;color:#fff;border:none;border-radius:10px;font:600 15px system-ui;cursor:pointer;box-shadow:0 4px 12px rgba(0,0,0,.35)';
    b.onclick = send;
    document.body.appendChild(b);

    // Auto-send once prices load and stabilise (so no manual click is needed).
    let last = -1, stable = 0, done = false;
    const iv = setInterval(function () {
      const n = document.querySelectorAll('.andes-money-amount__fraction').length;
      stable = (n > 0 && n === last) ? stable + 1 : 0;
      last = n;
      if (!done && stable >= 2) { done = true; clearInterval(iv); send(); }
    }, 800);
    setTimeout(function () { clearInterval(iv); }, 20000);
  });
})();
"#;

/// Open (or focus) the MercadoLibre webview. The user logs in and navigates to
/// their favorites, then clicks the injected button to send prices back.
#[tauri::command]
async fn scrape_ml_favorites(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("ml") {
        let _ = w.set_focus();
        return Ok(());
    }
    let url = "https://www.mercadolibre.com.pe/".parse().map_err(|_| "bad url".to_string())?;
    WebviewWindowBuilder::new(&app, "ml", WebviewUrl::External(url))
        .title("MercadoLibre — inicia sesión, abre tus Favoritos y pulsa el botón azul")
        .inner_size(1150.0, 820.0)
        .initialization_script(ML_INIT)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// GET a URL (the dashboard read API) with an optional Authorization header.
/// Runs in Rust to bypass the webview's CORS restrictions.
#[tauri::command]
async fn api_get(url: String, authorization: String) -> Result<String, String> {
    let mut req = reqwest::Client::new().get(&url);
    if !authorization.is_empty() {
        req = req.header("authorization", authorization);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{status}: {body}"));
    }
    Ok(body)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![plan_purchases, scrape_ml_favorites, api_get])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
