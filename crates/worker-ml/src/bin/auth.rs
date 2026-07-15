//! One-shot OAuth helper: obtain a MercadoLibre refresh token.
//!
//! Usage:
//!   1. Register an app at https://developers.mercadolibre.com with redirect URI
//!      exactly `http://localhost:8788/callback`.
//!   2. ML_CLIENT_ID=... ML_CLIENT_SECRET=... cargo run -p worker-ml --bin auth
//!   3. Open the printed URL, log in, authorize. The token prints here.
//!   4. Put the refresh token in server/.env as ML_REFRESH_TOKEN.
//!
//! Auth domain defaults to Peru (`auth.mercadolibre.com.pe`); override with
//! ML_AUTH_DOMAIN for another country site.

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const REDIRECT_URI: &str = "http://localhost:8788/callback";
const TOKEN_URL: &str = "https://api.mercadolibre.com/oauth/token";

#[tokio::main]
async fn main() -> Result<()> {
    let client_id = env("ML_CLIENT_ID")?;
    let client_secret = env("ML_CLIENT_SECRET")?;
    let auth_domain =
        std::env::var("ML_AUTH_DOMAIN").unwrap_or_else(|_| "auth.mercadolibre.com.pe".into());

    let auth_url = format!(
        "https://{auth_domain}/authorization?response_type=code&client_id={client_id}&redirect_uri={REDIRECT_URI}"
    );
    println!("\n1) Open this URL in your browser, log in and authorize:\n\n{auth_url}\n");
    println!("2) Waiting for the redirect on {REDIRECT_URI} ...\n");

    let code = wait_for_code().await?;
    println!("Got authorization code, exchanging for tokens...\n");

    let http = reqwest::Client::new();
    let resp: serde_json::Value = http
        .post(TOKEN_URL)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("code", &code),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await?
        .error_for_status()
        .context("token exchange failed")?
        .json()
        .await?;

    let refresh = resp.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("<none>");
    let access = resp.get("access_token").and_then(|v| v.as_str()).unwrap_or("<none>");
    println!("access_token : {access}");
    println!("refresh_token: {refresh}");
    println!("\nSet ML_REFRESH_TOKEN={refresh} in server/.env");
    Ok(())
}

/// Accept one HTTP request on the callback port and pull `code` from the query.
async fn wait_for_code() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:8788").await.context("binding :8788")?;
    let (mut stream, _) = listener.accept().await?;

    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    // First line: "GET /callback?code=XXX&state=... HTTP/1.1"
    let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
    let code = target
        .split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("code=")))
        .map(|c| c.to_string());

    let body = "<html><body><h3>in_out: authorization received. You can close this tab.</h3></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;

    code.ok_or_else(|| anyhow!("no `code` in redirect query"))
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow!("missing env var {key}"))
}
