//! Tauri app: exposes core logic to the SvelteKit frontend as commands.

use in_out_core::{buy_plan, rank, BuyPlan, Filters, ItemSnapshot};

/// Rank a wishlist by the given filters and compute the buy plan for `budget_cents`.
/// The frontend calls this via `invoke('plan_purchases', { items, filters, budgetCents })`.
#[tauri::command]
fn plan_purchases(items: Vec<ItemSnapshot>, filters: Filters, budget_cents: i64) -> BuyPlan {
    let ranked = rank(&items, &filters);
    buy_plan(&ranked, budget_cents)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![plan_purchases])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
