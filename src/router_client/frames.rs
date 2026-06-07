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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        moniker: Option<String>,
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

    #[test]
    fn auth_roundtrip_with_moniker() {
        let frame = NodeToRouterFrame::Auth {
            node_id: "0xabc".into(),
            signature: "0xsig".into(),
            ed25519_pubkey: Some("pk".into()),
            moniker: Some("my-node".into()),
        };
        let json = frame.to_json().unwrap();
        assert!(json.contains("moniker"));
        let parsed: NodeToRouterFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            NodeToRouterFrame::Auth { moniker, .. } => assert_eq!(moniker.as_deref(), Some("my-node")),
            _ => panic!("expected auth"),
        }
    }
}
