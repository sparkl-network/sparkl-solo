//! JSON wire protocol between sparkl-router and provider nodes (mirrors router `protocol.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeToRouterFrame {
    Auth {
        node_id: String,
        signature: String,
        #[serde(default)]
        ed25519_pubkey: Option<String>,
    },
    Pong,
    Response {
        rid: Uuid,
        status: u16,
        #[serde(default)]
        headers: Value,
    },
    Chunk {
        rid: Uuid,
        data: String,
    },
    End {
        rid: Uuid,
        status: u16,
    },
    Error {
        rid: Uuid,
        code: u16,
        message: String,
    },
    ActivateResponse {
        rid: Uuid,
        api_key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RouterToNodeFrame {
    Challenge {
        nonce: String,
        block: u64,
    },
    Ready {
        router_url: String,
    },
    Request {
        rid: Uuid,
        method: String,
        path: String,
        #[serde(default)]
        headers: Value,
        body: Option<String>,
    },
    ActivateRequest {
        rid: Uuid,
        session_id: String,
        signature: String,
        block_number: u64,
        #[serde(default)]
        message: Option<String>,
    },
    Ping,
}

impl NodeToRouterFrame {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl RouterToNodeFrame {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_challenge() {
        let json = r#"{"type":"challenge","nonce":"ab","block":1}"#;
        let f: RouterToNodeFrame = serde_json::from_str(json).unwrap();
        assert!(matches!(f, RouterToNodeFrame::Challenge { .. }));
    }
}
