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

// Per-source prelude defines window.__inoutSource / __inoutCount / __inoutExtract;
// COMMON_INIT (below) wires the on-page button, auto-send and login detection.

const ML_PRELUDE: &str = r#"
window.__inoutSource = 'ml';
window.__inoutCount = function () { return document.querySelectorAll('.andes-money-amount__fraction').length; };
window.__inoutExtract = function () {
  const out = [];
  document.querySelectorAll('.andes-money-amount__fraction').forEach(function (el) {
    const card = el.closest('li, .andes-card, .poly-card, .ui-search-result, .ui-search-layout__item');
    if (!card || card.dataset.inoutSeen) return;
    card.dataset.inoutSeen = '1';
    const link = card.querySelector('a[href]');
    const title = card.querySelector('h2, .poly-component__title, .ui-search-item__title');
    const digits = el.textContent.replace(/[^0-9]/g, '');
    if (!digits) return;
    // Real seller signals read from the card text (defensive against DOM churn).
    const txt = (card.textContent || '').toLowerCase();
    let medal = null;
    if (txt.indexOf('platinum') >= 0 || txt.indexOf('platino') >= 0) medal = 'platinum';
    else if (txt.indexOf('mercadolíder') >= 0 || txt.indexOf('mercadolider') >= 0 || txt.indexOf('mercado líder') >= 0) medal = 'gold';
    const verified = txt.indexOf('tienda oficial') >= 0;
    const soldm = txt.match(/([\d.]+)\s*vendidos/);
    const sales = soldm ? parseInt(soldm[1].replace(/\./g, ''), 10) || 0 : 0;
    out.push({
      item_id: (link && link.href.match(/(ML[A-Z]-?\d+)/) || [,''])[1] || (link ? link.href : ''),
      title: title ? title.textContent.trim() : '(sin título)',
      price_cents: parseInt(digits, 10) * 100,
      currency: 'PEN', seller_status: medal, seller_sales: sales, verified: verified,
      permalink: link ? link.href : ''
    });
  });
  document.querySelectorAll('[data-inout-seen]').forEach(function (c) { delete c.dataset.inoutSeen; });
  return out;
};
"#;

const FB_PRELUDE: &str = r#"
window.__inoutSource = 'fb';
window.__inoutCount = function () {
  let n = 0;
  document.querySelectorAll('a[href*="/marketplace/item/"]').forEach(function (a) {
    if (/(?:S\/|US\$|\$)\s?[\d.,]+/.test(a.innerText || '')) n++;
  });
  return n;
};
window.__inoutExtract = function () {
  const out = [], seen = {};
  document.querySelectorAll('a[href*="/marketplace/item/"]').forEach(function (a) {
    const idm = a.href.match(/item\/(\d+)/);
    if (!idm || seen[idm[1]]) return;
    const txt = a.innerText || '';
    const m = txt.match(/(?:S\/|US\$|\$)\s?([\d.,]+)/);
    if (!m) return;
    const num = parseFloat(m[1].replace(/,/g, ''));
    if (!num) return;
    seen[idm[1]] = 1;
    const lines = txt.split('\n').map(function (s) { return s.trim(); })
      .filter(function (s) { return s && !/^(?:S\/|US\$|\$)/.test(s) && !/^\d/.test(s); });
    lines.sort(function (a, b) { return b.length - a.length; });
    out.push({
      item_id: 'FB' + idm[1],
      title: (lines[0] || 'Marketplace').slice(0, 80),
      price_cents: Math.round(num * 100),
      currency: 'PEN', seller_status: null, seller_sales: 0, verified: false,
      permalink: a.href.split('?')[0]
    });
  });
  return out;
};
"#;

const COMMON_INIT: &str = r#"
(function () {
  function toast(msg) {
    let t = document.getElementById('inout-toast');
    if (!t) { t = document.createElement('div'); t.id = 'inout-toast';
      t.style.cssText = 'position:fixed;z-index:2147483647;bottom:80px;right:20px;background:#111;color:#fff;padding:10px 14px;border-radius:8px;font:14px system-ui;box-shadow:0 4px 12px rgba(0,0,0,.4)';
      document.body.appendChild(t); }
    t.textContent = msg;
  }
  function send() {
    const src = window.__inoutSource;
    const items = window.__inoutExtract();
    const total = items.reduce(function (s, i) { return s + i.price_cents; }, 0);
    const medaled = items.filter(function (i) { return i.seller_status; }).length;
    toast('in_out [' + src + ']: ' + items.length + ' items · S/ ' + (total / 100).toFixed(2) + ' · ' + medaled + ' con medalla');
    try { window.__TAURI__.event.emit(src + '-items', { url: location.href, items: items }); } catch (e) { toast('in_out error: ' + e); }
  }
  window.addEventListener('load', function () {
    const b = document.createElement('button');
    b.textContent = '➡ Enviar a in_out';
    b.style.cssText = 'position:fixed;z-index:2147483647;bottom:20px;right:20px;padding:12px 18px;background:#3b82f6;color:#fff;border:none;border-radius:10px;font:600 15px system-ui;cursor:pointer;box-shadow:0 4px 12px rgba(0,0,0,.35)';
    b.onclick = send;
    document.body.appendChild(b);

    let last = -1, stable = 0, done = false;
    const iv = setInterval(function () {
      const n = window.__inoutCount();
      stable = (n > 0 && n === last) ? stable + 1 : 0;
      last = n;
      if (!done && stable >= 2) { done = true; clearInterval(iv); send(); }
    }, 900);
    setTimeout(function () {
      clearInterval(iv);
      if (!done) { try { window.__TAURI__.event.emit(window.__inoutSource + '-needs-login', location.href); } catch (e) {} }
    }, 18000);
  });
})();
"#;

fn scraper_init(source: &str) -> String {
    let prelude = if source == "fb" { FB_PRELUDE } else { ML_PRELUDE };
    format!("{prelude}\n{COMMON_INIT}")
}

/// Open/reuse a scraping webview for `source` ("ml" or "fb"). `hidden = true`
/// scrapes in the background reusing the persisted login session; otherwise it's
/// shown so the user can log in / navigate. `url` targets a saved page.
#[tauri::command]
async fn open_scraper(app: tauri::AppHandle, source: String, url: Option<String>, hidden: bool) -> Result<(), String> {
    let label = if source == "fb" { "fb" } else { "ml" };
    let home = if label == "fb" {
        "https://www.facebook.com/marketplace/"
    } else {
        "https://www.mercadolibre.com.pe/"
    };
    let target = url.unwrap_or_else(|| home.to_string());

    if let Some(w) = app.get_webview_window(label) {
        let _ = w.eval(&format!("window.location.replace({target:?})"));
        if hidden {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
        return Ok(());
    }

    let parsed = target.parse().map_err(|_| "bad url".to_string())?;
    let title = if label == "fb" { "Facebook Marketplace — in_out" } else { "MercadoLibre — in_out" };
    let mut b = WebviewWindowBuilder::new(&app, label, WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1150.0, 820.0)
        .initialization_script(scraper_init(label));
    if hidden {
        b = b.visible(false);
    }
    b.build().map_err(|e| e.to_string())?;
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
        .invoke_handler(tauri::generate_handler![plan_purchases, open_scraper, api_get])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
