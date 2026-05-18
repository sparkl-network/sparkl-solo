//! Settlement API endpoints — deposit / withdraw on Hub EVM `SettlementEscrow`.
//!
//! All functions are gated behind the `evm-settlement` feature flag.
//! When settlement is disabled or the escrow contract is zero, endpoints
//! return a 501 (Not Implemented) with a JSON body explaining the reason.

use axum::Json;
use serde::{Deserialize, Serialize};
#[cfg(feature = "evm-settlement")]
use tracing::error;

#[derive(Debug, Deserialize)]
pub struct DepositDotRequest {
    pub amount_native: u64,
}

#[derive(Debug, Deserialize)]
pub struct DepositUsdcAsDotRequest {
    pub usdc_amount: u64,
    #[serde(default = "default_min_dot_out")]
    pub min_dot_internal_out: u64,
    #[serde(default = "default_max_oracle_age")]
    pub max_oracle_age_secs: u64,
}

fn default_min_dot_out() -> u64 {
    0
}

fn default_max_oracle_age() -> u64 {
    u64::MAX
}

#[derive(Debug, Deserialize)]
pub struct WithdrawDotRequest {
    pub amount_internal: u64,
}

#[derive(Debug, Deserialize)]
pub struct WithdrawProviderDotRequest {
    pub node_id: String, // hex-encoded 32 bytes
    pub amount_internal: u64,
}

#[derive(Debug, Serialize)]
pub struct SettlementResponse {
    pub success: bool,
    pub tx_hash: Option<String>,
    pub message: String,
}

/// Deposit native DOT into the escrow contract.
///
/// POST /settlement/deposit-dot
#[cfg(feature = "evm-settlement")]
pub async fn deposit_dot(
    state: axum::extract::State<crate::server::AppState>,
    Json(req): Json<DepositDotRequest>,
) -> Json<SettlementResponse> {
    use crate::settlement::evm::deposit_dot;

    if !state.config.settlement.enabled {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "settlement is disabled".to_string(),
        });
    }

    if state.config.settlement.escrow_contract.is_empty()
        || state.config.settlement.escrow_contract == "0x0000000000000000000000000000000000000000"
    {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "escrow_contract not configured".to_string(),
        });
    }

    match deposit_dot(
        &state.config.settlement.escrow_contract,
        &state.config.settlement.evm_rpc_url,
        &state.config.settlement.evm_provider_wallet_private_key,
        req.amount_native,
    )
    .await
    {
        Ok(Some(tx_hash)) => {
            Json(SettlementResponse {
                success: true,
                tx_hash: Some(tx_hash),
                message: "deposit confirmed".to_string(),
            })
        }
        Ok(None) => {
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: "deposit skipped (graceful degradation)".to_string(),
            })
        }
        Err(e) => {
            error!(error = %e, "deposit_dot failed");
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: format!("deposit failed: {e}"),
            })
        }
    }
}

/// Deposit USDC as DOT via the escrow's oracle conversion.
///
/// POST /settlement/deposit-usdc
#[cfg(feature = "evm-settlement")]
pub async fn deposit_usdc_as_dot(
    state: axum::extract::State<crate::server::AppState>,
    Json(req): Json<DepositUsdcAsDotRequest>,
) -> Json<SettlementResponse> {
    use crate::settlement::evm::deposit_usdc_as_dot;

    if !state.config.settlement.enabled {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "settlement is disabled".to_string(),
        });
    }

    if state.config.settlement.escrow_contract.is_empty()
        || state.config.settlement.escrow_contract == "0x0000000000000000000000000000000000000000"
    {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "escrow_contract not configured".to_string(),
        });
    }

    match deposit_usdc_as_dot(
        &state.config.settlement.escrow_contract,
        &state.config.settlement.evm_rpc_url,
        &state.config.settlement.evm_provider_wallet_private_key,
        req.usdc_amount,
        req.min_dot_internal_out,
        req.max_oracle_age_secs,
    )
    .await
    {
        Ok(Some(tx_hash)) => {
            Json(SettlementResponse {
                success: true,
                tx_hash: Some(tx_hash),
                message: "USDC deposit confirmed".to_string(),
            })
        }
        Ok(None) => {
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: "USDC deposit skipped (graceful degradation)".to_string(),
            })
        }
        Err(e) => {
            error!(error = %e, "deposit_usdc_as_dot failed");
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: format!("USDC deposit failed: {e}"),
            })
        }
    }
}

/// Withdraw native DOT from the escrow contract.
///
/// POST /settlement/withdraw-dot
#[cfg(feature = "evm-settlement")]
pub async fn withdraw_dot(
    state: axum::extract::State<crate::server::AppState>,
    Json(req): Json<WithdrawDotRequest>,
) -> Json<SettlementResponse> {
    use crate::settlement::evm::withdraw_dot;

    if !state.config.settlement.enabled {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "settlement is disabled".to_string(),
        });
    }

    if state.config.settlement.escrow_contract.is_empty()
        || state.config.settlement.escrow_contract == "0x0000000000000000000000000000000000000000"
    {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "escrow_contract not configured".to_string(),
        });
    }

    match withdraw_dot(
        &state.config.settlement.escrow_contract,
        &state.config.settlement.evm_rpc_url,
        &state.config.settlement.evm_provider_wallet_private_key,
        req.amount_internal,
    )
    .await
    {
        Ok(Some(tx_hash)) => {
            Json(SettlementResponse {
                success: true,
                tx_hash: Some(tx_hash),
                message: "withdraw confirmed".to_string(),
            })
        }
        Ok(None) => {
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: "withdraw skipped (graceful degradation)".to_string(),
            })
        }
        Err(e) => {
            error!(error = %e, "withdraw_dot failed");
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: format!("withdraw failed: {e}"),
            })
        }
    }
}

/// Withdraw provider earnings from the escrow contract.
///
/// POST /settlement/withdraw-provider
#[cfg(feature = "evm-settlement")]
pub async fn withdraw_provider(
    state: axum::extract::State<crate::server::AppState>,
    Json(req): Json<WithdrawProviderDotRequest>,
) -> Json<SettlementResponse> {
    use crate::settlement::evm::withdraw_provider_dot;

    if !state.config.settlement.enabled {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "settlement is disabled".to_string(),
        });
    }

    if state.config.settlement.escrow_contract.is_empty()
        || state.config.settlement.escrow_contract == "0x0000000000000000000000000000000000000000"
    {
        return Json(SettlementResponse {
            success: false,
            tx_hash: None,
            message: "escrow_contract not configured".to_string(),
        });
    }

    let node_id_bytes = match hex::decode(req.node_id.trim().strip_prefix("0x").unwrap_or(req.node_id.trim())) {
        Ok(bytes) => {
            if bytes.len() != 32 {
                return Json(SettlementResponse {
                    success: false,
                    tx_hash: None,
                    message: "node_id must be 32 bytes hex".to_string(),
                });
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        Err(e) => {
            return Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: format!("invalid node_id hex: {e}"),
            });
        }
    };

    match withdraw_provider_dot(
        &state.config.settlement.escrow_contract,
        &state.config.settlement.evm_rpc_url,
        &state.config.settlement.evm_settlement_operator_wallet_private_key,
        node_id_bytes,
        req.amount_internal,
    )
    .await
    {
        Ok(Some(tx_hash)) => {
            Json(SettlementResponse {
                success: true,
                tx_hash: Some(tx_hash),
                message: "provider withdraw confirmed".to_string(),
            })
        }
        Ok(None) => {
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: "provider withdraw skipped (graceful degradation)".to_string(),
            })
        }
        Err(e) => {
            error!(error = %e, "withdraw_provider_dot failed");
            Json(SettlementResponse {
                success: false,
                tx_hash: None,
                message: format!("provider withdraw failed: {e}"),
            })
        }
    }
}

// ===========================================================================
// Non-evm-settlement stubs (501 Not Implemented)
// ===========================================================================

#[cfg(not(feature = "evm-settlement"))]
pub async fn deposit_dot() -> Json<SettlementResponse> {
    Json(SettlementResponse {
        success: false,
        tx_hash: None,
        message: "evm-settlement feature not enabled".to_string(),
    })
}

#[cfg(not(feature = "evm-settlement"))]
pub async fn deposit_usdc_as_dot() -> Json<SettlementResponse> {
    Json(SettlementResponse {
        success: false,
        tx_hash: None,
        message: "evm-settlement feature not enabled".to_string(),
    })
}

#[cfg(not(feature = "evm-settlement"))]
pub async fn withdraw_dot() -> Json<SettlementResponse> {
    Json(SettlementResponse {
        success: false,
        tx_hash: None,
        message: "evm-settlement feature not enabled".to_string(),
    })
}

#[cfg(not(feature = "evm-settlement"))]
pub async fn withdraw_provider() -> Json<SettlementResponse> {
    Json(SettlementResponse {
        success: false,
        tx_hash: None,
        message: "evm-settlement feature not enabled".to_string(),
    })
}
