//! Bootstrap contract reads for Hub EVM addresses (`SparklNetworkConfig`).
#![cfg(feature = "evm-settlement")]

use alloy::{
    primitives::Address,
    providers::ProviderBuilder,
};
use anyhow::{Context, Result};

use crate::config::{RegistryConfig, SettlementConfig};

/// Hardcoded `SparklNetworkConfig` deployment (CREATE2). **Ceremony:** after the deploy script runs,
/// replace this with the address written to `contracts/deployments/paseo.json` (`sparklNetworkConfig`)
/// and rebuild. Template fields live in `contracts/deployments/paseo.example.json`. The salt hex in
/// that file is `keccak256(bytes("sparkl.network.config.v1"))` (matches `DeploySparklBase.NETWORK_CONFIG_SALT`).
///
/// Until non-zero: nodes rely on `[registry].registry_contract_address` and `[settlement].escrow_contract`
/// from TOML/CLI (see `resolve_with_overrides`).
pub const SPARKL_NETWORK_CONFIG_ADDRESS: &str =
    "0x0000000000000000000000000000000000000000";

alloy::sol!(
    #[sol(rpc)]
    SparklNetworkConfig,
    concat!(env!("CARGO_MANIFEST_DIR"), "/abi/SparklNetworkConfig.json")
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedHubAddresses {
    pub provider_registry: Address,
    pub settlement_escrow: Address,
    pub price_oracle: Address,
    pub version: u64,
}

/// `None` when the constant is absent, invalid hex, or the zero address (dev / not yet configured).
pub fn network_config_bootstrap_address() -> Option<Address> {
    let s = SPARKL_NETWORK_CONFIG_ADDRESS.trim();
    if s.is_empty() {
        return None;
    }
    let addr: Address = s.parse().ok()?;
    if addr == Address::ZERO {
        return None;
    }
    Some(addr)
}

pub fn parse_evm_address_field(raw: &str, field: &'static str) -> Result<Address> {
    let t = raw.trim();
    if t.is_empty() {
        anyhow::bail!("{field} is empty");
    }
    t.parse::<Address>()
        .map_err(|e| anyhow::anyhow!("invalid {field} `{t}`: {e}"))
}

/// Read registry / escrow / oracle / config version from the bootstrap contract via JSON-RPC.
pub async fn resolve_from_bootstrap(rpc_url: &str, bootstrap: Address) -> Result<ResolvedHubAddresses> {
    let url = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid RPC URL for bootstrap resolution")?;
    let provider = ProviderBuilder::new().connect_http(url);
    let cfg = SparklNetworkConfig::new(bootstrap, &provider);

    let provider_registry = cfg
        .providerRegistry()
        .call()
        .await
        .context("SparklNetworkConfig.providerRegistry eth_call failed")?;
    let settlement_escrow = cfg
        .settlementEscrow()
        .call()
        .await
        .context("SparklNetworkConfig.settlementEscrow eth_call failed")?;
    let price_oracle = cfg
        .priceOracle()
        .call()
        .await
        .context("SparklNetworkConfig.priceOracle eth_call failed")?;
    let version = cfg
        .version()
        .call()
        .await
        .context("SparklNetworkConfig.version eth_call failed")?;

    Ok(ResolvedHubAddresses {
        provider_registry,
        settlement_escrow,
        price_oracle,
        version: version as u64,
    })
}

fn fallback_from_config(registry: &RegistryConfig, settlement: &SettlementConfig) -> Result<ResolvedHubAddresses> {
    Ok(ResolvedHubAddresses {
        provider_registry: parse_evm_address_field(&registry.registry_contract_address, "registry.registry_contract_address")?,
        settlement_escrow: parse_evm_address_field(&settlement.escrow_contract, "settlement.escrow_contract")?,
        price_oracle: Address::ZERO,
        version: 0,
    })
}

/// Prefer on-chain bootstrap when [`SPARKL_NETWORK_CONFIG_ADDRESS`] is set and non-zero; otherwise
/// (or on `eth_call` failure) use registry + escrow from config.
pub async fn resolve_with_overrides(
    rpc_url: &str,
    registry: &RegistryConfig,
    settlement: &SettlementConfig,
) -> Result<ResolvedHubAddresses> {
    let Some(bootstrap) = network_config_bootstrap_address() else {
        return fallback_from_config(registry, settlement);
    };

    match resolve_from_bootstrap(rpc_url, bootstrap).await {
        Ok(h) => Ok(h),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "SparklNetworkConfig bootstrap resolve failed; falling back to registry/settlement config fields"
            );
            fallback_from_config(registry, settlement)
        }
    }
}

pub fn format_address_cfg(addr: Address) -> String {
    format!("{addr:#x}")
}
