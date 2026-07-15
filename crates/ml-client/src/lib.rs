//! Minimal async client for the MercadoLibre REST API.
//!
//! Scope for F1:
//!   * OAuth token refresh (`refresh_token` -> `access_token`).
//!   * `GET /users/me/bookmarks`  -> wishlist item ids.
//!   * `GET /items/{id}`          -> price + seller id.
//!   * `GET /users/{id}`          -> seller reputation (medal + sales).
//!
//! Docs:
//!   Bookmarks:  https://developers.mercadolibre.com.do/en_us/en_us/bookmarks
//!   Reputation: https://developers.mercadolibre.com.ar/en_us/sellers-reputation

use in_out_core::{ItemSnapshot, Medal};
use serde::Deserialize;

const API: &str = "https://api.mercadolibre.com";
const AUTH: &str = "https://api.mercadolibre.com/oauth/token";

#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected response shape: {0}")]
    Shape(String),
}

pub type Result<T> = std::result::Result<T, MlError>;

/// Long-lived credentials for one user. Persist `refresh_token` server-side.
#[derive(Debug, Clone)]
pub struct OAuthCreds {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenResp {
    access_token: String,
    #[allow(dead_code)]
    #[serde(default)]
    refresh_token: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    expires_in: Option<i64>,
}

pub struct MlClient {
    http: reqwest::Client,
    access_token: String,
}

impl MlClient {
    /// Build a client from an already-obtained access token.
    pub fn with_token(access_token: impl Into<String>) -> Self {
        Self { http: reqwest::Client::new(), access_token: access_token.into() }
    }

    /// Exchange a refresh token for a fresh access token, then build a client.
    pub async fn from_refresh(creds: &OAuthCreds) -> Result<Self> {
        let http = reqwest::Client::new();
        let resp: TokenResp = http
            .post(AUTH)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", &creds.client_id),
                ("client_secret", &creds.client_secret),
                ("refresh_token", &creds.refresh_token),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Self { http, access_token: resp.access_token })
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value> {
        Ok(self
            .http
            .get(format!("{API}{path}"))
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Wishlist item ids for the authenticated user.
    ///
    /// The bookmarks payload nests an item object per entry; we extract `item.id`.
    pub async fn bookmarks(&self) -> Result<Vec<String>> {
        let v = self.get("/users/me/bookmarks").await?;
        let arr = v
            .as_array()
            .or_else(|| v.get("results").and_then(|r| r.as_array()))
            .ok_or_else(|| MlError::Shape("bookmarks not an array".into()))?;

        let mut ids = Vec::new();
        for entry in arr {
            let id = entry
                .get("item_id")
                .or_else(|| entry.pointer("/item/id"))
                .or_else(|| entry.get("id"))
                .and_then(|x| x.as_str());
            if let Some(id) = id {
                ids.push(id.to_string());
            }
        }
        Ok(ids)
    }

    /// Fetch price + seller id for a single item.
    pub async fn item(&self, item_id: &str) -> Result<Item> {
        let v = self.get(&format!("/items/{item_id}")).await?;
        serde_json::from_value(v).map_err(|e| MlError::Shape(e.to_string()))
    }

    /// Fetch a seller's reputation signals.
    pub async fn seller(&self, seller_id: i64) -> Result<Seller> {
        let v = self.get(&format!("/users/{seller_id}")).await?;
        serde_json::from_value(v).map_err(|e| MlError::Shape(e.to_string()))
    }

    /// Fetch an item + its seller and fold into a core snapshot plus the extra
    /// metadata we persist (permalink, seller id).
    ///
    /// `verified` is a heuristic; refine server-side as filters mature.
    pub async fn priced_item(&self, item_id: &str) -> Result<PricedItem> {
        let item = self.item(item_id).await?;
        let seller = self.seller(item.seller_id).await?;
        let medal = seller
            .seller_reputation
            .power_seller_status
            .as_deref()
            .and_then(Medal::from_power_seller_status);
        let sales = seller.seller_reputation.transactions.completed;
        Ok(PricedItem {
            snapshot: ItemSnapshot {
                item_id: item.id,
                title: item.title,
                price_cents: (item.price * 100.0).round() as i64,
                currency: item.currency_id,
                seller_status: medal,
                seller_sales: sales,
                verified: medal.is_some() && sales > 0,
            },
            seller_id: item.seller_id,
            permalink: item.permalink,
        })
    }
}

/// A core snapshot plus the metadata persisted alongside it.
#[derive(Debug, Clone)]
pub struct PricedItem {
    pub snapshot: ItemSnapshot,
    pub seller_id: i64,
    pub permalink: String,
}

// ---- API response models (only the fields we use) ----

#[derive(Debug, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub price: f64,
    pub currency_id: String,
    pub seller_id: i64,
    #[serde(default)]
    pub permalink: String,
}

#[derive(Debug, Deserialize)]
pub struct Seller {
    pub id: i64,
    #[serde(default)]
    pub seller_reputation: SellerReputation,
}

#[derive(Debug, Default, Deserialize)]
pub struct SellerReputation {
    /// "silver" | "gold" | "platinum" | null
    #[serde(default)]
    pub power_seller_status: Option<String>,
    #[serde(default)]
    pub transactions: Transactions,
}

#[derive(Debug, Default, Deserialize)]
pub struct Transactions {
    #[serde(default)]
    pub completed: u32,
}
