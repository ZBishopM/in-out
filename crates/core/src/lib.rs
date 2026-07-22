//! `in_out_core` — pure domain logic shared by desktop, mobile and the server worker.
//!
//! No IO here. Three concerns:
//!   * [`wishlist`] — filter + rank MercadoLibre items by price.
//!   * [`budget`]   — greedy "what can I buy" plan for the progress bar.
//!   * [`finance`]  — duplicate detection / reconciliation of raw email events.

pub mod wishlist;
pub mod budget;
pub mod category;
pub mod finance;

pub use budget::{buy_plan, Affordable, BuyPlan};
pub use category::categorize;
pub use finance::{cluster, is_duplicate, RawEvent, ReconcileConfig};
pub use wishlist::{rank, Filters, ItemSnapshot, Medal};
