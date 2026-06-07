use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::Serialize;

/// Allowed keys for `[[models]].features` (key → freeform value).
pub const ALLOWED_FEATURE_KEYS: &[&str] = &["mtp", "speculative", "multimodal", "long_context"];

#[derive(Debug, Clone, Serialize)]
pub struct FeatureKeyDoc {
    pub key: &'static str,
    pub description: &'static str,
}

pub const FEATURE_KEY_DOCS: &[FeatureKeyDoc] = &[
    FeatureKeyDoc {
        key: "mtp",
        description: "MTP / multi-token prediction setup (e.g. draft depth, backend note)",
    },
    FeatureKeyDoc {
        key: "speculative",
        description: "Speculative decoding path (e.g. dflash, eagle, draft model name)",
    },
    FeatureKeyDoc {
        key: "multimodal",
        description: "Vision / image input support (e.g. resolution limit, template)",
    },
    FeatureKeyDoc {
        key: "long_context",
        description: "Long-context claim (e.g. effective window, yarn note)",
    },
];

pub fn is_allowed_feature_key(key: &str) -> bool {
    ALLOWED_FEATURE_KEYS.contains(&key)
}

/// Validate feature map keys and non-empty values.
pub fn validate_features(features: &HashMap<String, String>, model_id: &str) -> Result<()> {
    for (key, value) in features {
        if !is_allowed_feature_key(key) {
            return Err(anyhow!(
                "model {model_id}: unknown feature key '{key}' (allowed: {})",
                ALLOWED_FEATURE_KEYS.join(", ")
            ));
        }
        if value.trim().is_empty() {
            return Err(anyhow!(
                "model {model_id}: feature '{key}' value must be non-empty"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_key() {
        let mut f = HashMap::new();
        f.insert("unknown".into(), "x".into());
        assert!(validate_features(&f, "m").is_err());
    }

    #[test]
    fn accepts_valid_features() {
        let mut f = HashMap::new();
        f.insert("mtp".into(), "8-token".into());
        assert!(validate_features(&f, "m").is_ok());
    }
}
