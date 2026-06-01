//! Normalize router WebSocket URL from config / CLI.

use anyhow::{anyhow, Context, Result};
use url::Url;

const DEFAULT_CONNECT_PATH: &str = "/node/connect";

/// Validate and normalize `ws://` / `wss://` router tunnel URL.
pub fn normalize_router_ws_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("router URL is empty"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(anyhow!(
            "router URL must use ws:// or wss://, not http(s) (got {trimmed})"
        ));
    }
    let mut url = Url::parse(trimmed).context("invalid router WebSocket URL")?;
    let scheme = url.scheme();
    if scheme != "ws" && scheme != "wss" {
        return Err(anyhow!("router URL scheme must be ws or wss (got {scheme})"));
    }
    if url.host().is_none() {
        return Err(anyhow!("router URL missing host"));
    }
    let path = url.path();
    if path.is_empty() || path == "/" {
        tracing::warn!(
            url = %trimmed,
            "router URL missing path; appending {DEFAULT_CONNECT_PATH}"
        );
        url.set_path(DEFAULT_CONNECT_PATH);
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_connect_path() {
        let out = normalize_router_ws_url("ws://127.0.0.1:3001").unwrap();
        assert!(out.ends_with("/node/connect"));
    }

    #[test]
    fn rejects_http() {
        assert!(normalize_router_ws_url("http://127.0.0.1:3001/node/connect").is_err());
    }
}
