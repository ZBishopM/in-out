//! One-shot OAuth helper: obtain a MercadoLibre refresh token.
//!
//! MercadoLibre requires an **https** redirect URI on a valid public domain
//! (plain http, localhost and IPs are all rejected), so we don't run a local
//! server. Register a harmless reserved domain and read the `code` from the
//! browser's address bar after authorizing.
//!
//! Setup at https://developers.mercadolibre.com:
//!   * Redirect URI (exact): value of ML_REDIRECT_URI below
//!     (default https://example.com/callback — RFC 2606 reserved, benign)
//!   * OAuth flows: Authorization Code + Refresh Token
//!   * Permission "Usuarios": read (enough for /users/me/bookmarks)
//!
//! Run:
//!   ML_CLIENT_ID=... ML_CLIENT_SECRET=... cargo run -p worker-ml --bin auth

use anyhow::{anyhow, Context, Result};
use std::io::{self, Write};

const TOKEN_URL: &str = "https://api.mercadolibre.com/oauth/token";

#[tokio::main]
async fn main() -> Result<()> {
    worker_ml::load_env();
    let client_id = env("ML_CLIENT_ID")?;
    let client_secret = env("ML_CLIENT_SECRET")?;
    let auth_domain =
        std::env::var("ML_AUTH_DOMAIN").unwrap_or_else(|_| "auth.mercadolibre.com.pe".into());
    // Must exactly match the redirect URI registered on the ML app.
    let redirect_uri =
        std::env::var("ML_REDIRECT_URI").unwrap_or_else(|_| "https://example.com/callback".into());

    let auth_url = format!(
        "https://{auth_domain}/authorization?response_type=code&client_id={client_id}&redirect_uri={redirect_uri}"
    );
    println!("\nUsing redirect_uri: {redirect_uri}");
    println!("(register this EXACT value on the ML app)\n");
    println!("1) Open this URL, log in and authorize:\n\n{auth_url}\n");
    println!("2) The browser redirects to {redirect_uri}?code=...");
    println!("   Copy the `code` from the URL bar (the page content doesn't matter).\n");
    print!("Paste the code (or the whole redirected URL) here: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let code = extract_code(input.trim());
    if code.is_empty() {
        return Err(anyhow!("no code provided"));
    }

    println!("\nExchanging code for tokens...");
    let http = reqwest::Client::new();
    let http_resp = http
        .post(TOKEN_URL)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
        ])
        .send()
        .await?;
    let status = http_resp.status();
    let text = http_resp.text().await?;
    if !status.is_success() {
        eprintln!("\nToken exchange failed ({status}). MercadoLibre says:\n{text}");
        eprintln!("\nMost likely the code was already used or expired — codes last only a few\nminutes. Re-run and paste the code immediately after authorizing.");
        return Err(anyhow!("token exchange failed"));
    }
    let resp: serde_json::Value = serde_json::from_str(&text).context("parsing token response")?;

    let refresh = resp.get("refresh_token").and_then(|v| v.as_str());
    let access = resp.get("access_token").and_then(|v| v.as_str()).unwrap_or("<none>");
    println!("\naccess_token : {access}");
    match refresh {
        Some(rt) => {
            let prefix = &rt[..rt.len().min(8)];
            match worker_ml::persist_refresh_token(rt) {
                Ok(()) => println!("refresh_token ({prefix}...) saved to server/.env (ML_REFRESH_TOKEN)."),
                Err(e) => println!("refresh_token: {rt}\n(could not write server/.env: {e} — add it manually)"),
            }
        }
        None => println!("No refresh_token in response — is the Refresh Token flow enabled on the app?"),
    }
    Ok(())
}

/// Accept either a bare code or a full `...?code=XXX&...` URL.
fn extract_code(input: &str) -> String {
    match input.find("code=") {
        Some(i) => input[i + 5..].split(['&', ' ']).next().unwrap_or("").to_string(),
        None => input.to_string(),
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow!("missing env var {key}"))
}
