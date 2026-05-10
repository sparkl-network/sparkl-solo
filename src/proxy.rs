use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::config::BackendConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
}

#[derive(Clone)]
pub struct BackendProxy {
    client: reqwest::Client,
    base_url: Url,
    health_path: String,
    models_path: String,
}

impl BackendProxy {
    pub fn new(cfg: &BackendConfig) -> Result<Self> {
        let base_url = Url::parse(&cfg.url).context("invalid backend url")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .build()
            .context("failed to build reqwest client")?;
        Ok(Self {
            client,
            base_url,
            health_path: cfg.health_path.clone(),
            models_path: cfg.models_path.clone(),
        })
    }

    pub async fn check_health(&self) -> Result<()> {
        let url = self.base_url.join(&self.health_path)?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("health request failed")?;
        if !resp.status().is_success() {
            return Err(anyhow!("backend health check failed: {}", resp.status()));
        }
        Ok(())
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = self.base_url.join(&self.models_path)?;
        let body: Value = self
            .client
            .get(url)
            .send()
            .await
            .context("models request failed")?
            .json()
            .await
            .context("models response was not json")?;
        let mut models = Vec::new();
        if let Some(items) = body.get("data").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                    models.push(ModelInfo { id: id.to_string() });
                }
            }
        }
        Ok(models)
    }

    pub async fn stream_completion(
        &self,
        request: Value,
    ) -> Result<BoxStream<'static, Result<Bytes>>> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = self.base_url.join("/v1/chat/completions")?;
        let resp = self
            .client
            .post(url)
            .headers(headers)
            .json(&request)
            .send()
            .await
            .context("backend stream request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "<body unavailable>".to_string());
            return Err(anyhow!("backend error {status}: {text}"));
        }

        let stream = resp
            .bytes_stream()
            .map(|item| item.context("stream chunk error"));
        Ok(stream.boxed())
    }
}
