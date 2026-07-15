//! Deterministic demo data so the DB pipeline can be exercised without ML creds.

use in_out_core::{ItemSnapshot, Medal};
use ml_client::PricedItem;

fn priced(id: &str, title: &str, price_cents: i64, medal: Option<Medal>, sales: u32) -> PricedItem {
    PricedItem {
        snapshot: ItemSnapshot {
            item_id: id.into(),
            title: title.into(),
            price_cents,
            currency: "PEN".into(),
            seller_status: medal,
            seller_sales: sales,
            verified: medal.is_some() && sales > 0,
        },
        seller_id: 100000 + (id.len() as i64),
        permalink: format!("https://articulo.mercadolibre.com.pe/{id}"),
    }
}

pub fn demo_items() -> Vec<PricedItem> {
    vec![
        priced("MPE001", "Teclado mecánico", 12000, Some(Medal::Gold), 1200),
        priced("MPE002", "Mouse inalámbrico", 6500, Some(Medal::Platinum), 8000),
        priced("MPE003", "Hub USB-C", 4500, Some(Medal::Silver), 40),
        priced("MPE004", "Monitor 27\" 144Hz", 89000, Some(Medal::Gold), 300),
        priced("MPE005", "Auriculares", 15000, Some(Medal::Gold), 90),
    ]
}
