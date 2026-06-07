use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::capacity::ModelAdmission;
use crate::config::ModelEntryConfig;
use crate::config::NodeConfig;
use crate::proxy::BackendProxy;

/// Model row exposed on `/v1/models` and registry heartbeats.
#[derive(Debug, Clone, Serialize)]
pub struct PublishedModel {
    pub id: String,
    pub context_size: u32,
    pub quantization: String,
    pub parameter_count: String,
    pub source_url: String,
    pub concurrency: u32,
    pub active_requests: u32,
    /// Deprecated alias of `active_requests`.
    pub active_sessions: u32,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub features: HashMap<String, String>,
}

impl PublishedModel {
    pub fn to_openai_entry(&self) -> Value {
        let mut sparkl = json!({
            "quantization": self.quantization,
            "parameter_count": self.parameter_count,
            "source_url": self.source_url,
            "active_requests": self.active_requests,
            "active_sessions": self.active_sessions,
        });
        if self.concurrency > 0 {
            sparkl["concurrency"] = json!(self.concurrency);
        }
        if !self.features.is_empty() {
            sparkl["features"] = json!(self.features);
        }
        let mut entry = json!({
            "id": self.id,
            "object": "model",
            "created": 0,
            "owned_by": "sparkl",
            "sparkl": sparkl,
        });
        if self.context_size > 0 {
            entry["context_length"] = json!(self.context_size);
        }
        entry
    }
}

/// Build the published model list from config, backend, and live admission counts.
pub async fn build_catalog(
    proxy: &BackendProxy,
    config_models: &[ModelEntryConfig],
    node: &NodeConfig,
    admission: &ModelAdmission,
) -> Result<Vec<PublishedModel>> {
    let backend_ids: HashSet<String> = proxy
        .list_models()
        .await
        .context("backend model listing failed")?
        .into_iter()
        .map(|m| m.id)
        .collect();

    let mut out = Vec::new();

    if config_models.is_empty() {
        for id in backend_ids {
            if !node.is_model_allowed(&id) {
                continue;
            }
            let active = admission.active_count_for_model(&id);
            out.push(PublishedModel {
                id: id.clone(),
                context_size: 0,
                quantization: String::new(),
                parameter_count: String::new(),
                source_url: String::new(),
                concurrency: 0,
                active_requests: active,
                active_sessions: active,
                features: HashMap::new(),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(out);
    }

    for entry in config_models {
        let id = entry.id.trim();
        if !backend_ids.contains(id) {
            continue;
        }
        if !node.is_model_allowed(id) {
            continue;
        }
        let active = admission.active_count_for_model(id);
        out.push(PublishedModel {
            id: id.to_string(),
            context_size: entry.context_size,
            quantization: entry.quantization.clone(),
            parameter_count: entry.parameter_count.clone(),
            source_url: entry.source_url.clone(),
            concurrency: entry.concurrency,
            active_requests: active,
            active_sessions: active,
            features: entry.features.clone(),
        });
    }

    Ok(out)
}

pub fn catalog_to_openai_list(models: &[PublishedModel]) -> Value {
    let data: Vec<Value> = models.iter().map(|m| m.to_openai_entry()).collect();
    json!({
        "object": "list",
        "data": data
    })
}

pub fn catalog_ids(models: &[PublishedModel]) -> Vec<String> {
    models.iter().map(|m| m.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelEntryConfig, NodeConfig, NodeMode};
    use crate::session::SecurityTier;

    #[test]
    fn config_models_filter_backend_ids() {
        let backend_ids = [
            "qwen/qwen3.6-27b".to_string(),
            "other/model".to_string(),
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        let node = NodeConfig {
            moniker: "t".into(),
            data_dir: std::path::PathBuf::from("/tmp"),
            log_level: "info".into(),
            mode: NodeMode::Solo,
            receipt_cadence_tokens: 1,
            include_models: vec![],
            exclude_models: vec![],
            session_security_tier: SecurityTier::BestEffort,
        };
        let config_models = vec![ModelEntryConfig {
            id: "qwen/qwen3.6-27b".into(),
            quantization: "Q4_K_M".into(),
            parameter_count: "27B".into(),
            context_size: 128000,
            concurrency: 4,
            source_url: "https://example.com".into(),
            features: HashMap::new(),
        }];
        let admission = ModelAdmission::new();
        let mut out = Vec::new();
        for entry in &config_models {
            let id = entry.id.trim();
            if !backend_ids.contains(id) || !node.is_model_allowed(id) {
                continue;
            }
            out.push(PublishedModel {
                id: id.to_string(),
                context_size: entry.context_size,
                quantization: entry.quantization.clone(),
                parameter_count: entry.parameter_count.clone(),
                source_url: entry.source_url.clone(),
                concurrency: entry.concurrency,
                active_requests: 0,
                active_sessions: 0,
                features: entry.features.clone(),
            });
        }
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "qwen/qwen3.6-27b");
        assert_eq!(out[0].concurrency, 4);
        let _ = admission;
    }

    #[test]
    fn openai_entry_includes_sparkl_metadata() {
        let mut features = HashMap::new();
        features.insert("mtp".into(), "8-token".into());
        let m = PublishedModel {
            id: "test/model".into(),
            context_size: 128000,
            quantization: "Q4_K_M".into(),
            parameter_count: "27B".into(),
            source_url: "https://example.com/m".into(),
            concurrency: 8,
            active_requests: 2,
            active_sessions: 2,
            features,
        };
        let v = m.to_openai_entry();
        assert_eq!(v["context_length"], 128000);
        assert_eq!(v["sparkl"]["concurrency"], 8);
        assert_eq!(v["sparkl"]["active_requests"], 2);
        assert_eq!(v["sparkl"]["active_sessions"], 2);
        assert_eq!(v["sparkl"]["features"]["mtp"], "8-token");
    }
}
