//! On-chain settlement via Hub EVM `SettlementEscrow`.
#![cfg(feature = "evm-settlement")]

use std::sync::Arc;

use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::config::SettlementConfig;
use crate::session::{SecurityTier, Session, SessionManager, SessionState};

alloy::sol!(
    #[sol(rpc)]
    SettlementEscrow,
    concat!(env!("CARGO_MANIFEST_DIR"), "/abi/SettlementEscrow.json")
);

alloy::sol!(
    #[sol(rpc)]
    ProviderRegistry,
    concat!(env!("CARGO_MANIFEST_DIR"), "/abi/ProviderRegistry.json")
);

pub(crate) async fn process_settlement_tick(
    settlement: &SettlementConfig,
    sessions: Arc<SessionManager>,
    tee_tick_now: bool,
    epoch_boundary: bool,
    tee_candidates: &[Session],
    tee_last_eligible_block: &mut Option<u64>,
) {
    if !epoch_boundary && !tee_tick_now {
        return;
    }

    let rpc_url = match settlement.evm_rpc_url.trim().parse::<reqwest::Url>() {
        Ok(u) => u,
        Err(e) => {
            warn!(error = %e, "invalid settlement.evm_rpc_url; skipping EVM settlement tick");
            return;
        }
    };

    let escrow_addr: Address = match settlement.escrow_contract.trim().parse() {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "invalid settlement.escrow_contract; skipping EVM settlement tick");
            return;
        }
    };

    let provider_pk = settlement.evm_provider_wallet_private_key.trim();
    let operator_pk = settlement.evm_settlement_operator_wallet_private_key.trim();
    if provider_pk.is_empty() || operator_pk.is_empty() {
        warn!("evm-settlement enabled but node-operator / settlement-operator wallet keys missing; skipping EVM settlement tick");
        return;
    }

    let node_operator_signer = match signer_from_hex(provider_pk) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "invalid settlement.evm_provider_wallet_private_key");
            return;
        }
    };

    let operator_signer = match signer_from_hex(operator_pk) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "invalid settlement.evm_settlement_operator_wallet_private_key");
            return;
        }
    };

    let read_provider = ProviderBuilder::new().connect_http(rpc_url.clone());
    let escrow_read = SettlementEscrow::new(escrow_addr, &read_provider);

    let operator_on_chain = match escrow_read.settlementOperator().call().await {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "settlementOperator() eth_call failed; skipping EVM settlement tick");
            return;
        }
    };

    if operator_on_chain == Address::ZERO {
        warn!("settlementOperator unset on escrow contract; skipping EVM settlement tick");
        return;
    }

    if operator_signer.address() != operator_on_chain {
        warn!(
            signer = %operator_signer.address(),
            on_chain = %operator_on_chain,
            "operator private key does not match escrow.settlementOperator; skipping EVM settlement tick"
        );
        return;
    }

    let node_operator_rpc = ProviderBuilder::new()
        .wallet(node_operator_signer.clone())
        .fetch_chain_id()
        .connect_http(rpc_url.clone());
    let escrow_node_operator = SettlementEscrow::new(escrow_addr, &node_operator_rpc);

    let operator_exec = ProviderBuilder::new()
        .wallet(operator_signer.clone())
        .fetch_chain_id()
        .connect_http(rpc_url);
    let escrow_operator = SettlementEscrow::new(escrow_addr, &operator_exec);

    let chain_head_opt =
        if tee_tick_now && !tee_candidates.is_empty() && settlement.tee_settle_every_n_blocks > 0 {
            match fetch_head_block_number(&read_provider).await {
                Ok(h) => Some(h),
                Err(e) => {
                    warn!(error = %e, "failed to fetch chain head for tee_settle_every_n_blocks gate");
                    None
                }
            }
        } else {
            None
        };

    let tee_gate_ok = tee_gate_open(settlement, chain_head_opt, tee_last_eligible_block.as_ref());

    if tee_tick_now && !tee_candidates.is_empty() {
        if settlement.tee_settle_every_n_blocks > 0 && !tee_gate_ok {
            warn!(
                head = ?chain_head_opt,
                last_eligible = ?tee_last_eligible_block,
                blocks = settlement.tee_settle_every_n_blocks,
                "tee settlement gated by block cadence; skipping this tee tick"
            );
        } else {
            let mut advanced_eligibility_anchor = false;
            for session in tee_candidates {
                let partial = matches!(session.state, SessionState::Active);
                let res = async {
                    let Some(chain_sid_u64) = session.evm_session_id else {
                        warn!(session_uuid = %session.id, "skip tee settlement: session.evm_session_id not set");
                        return Ok::<SettleOutcome, anyhow::Error>(SettleOutcome::Noop);
                    };

                    let session_id = U256::from(chain_sid_u64);

                    let chain_sess = escrow_read
                        .sessions(session_id)
                        .call()
                        .await
                        .context("sessions(sessionId) eth_call failed")?;

                    if chain_sess.settled {
                        sessions.close(session.id, SessionState::Settled);
                        return Ok(SettleOutcome::Noop);
                    }

                    if chain_sess.user == Address::ZERO {
                        anyhow::bail!("unknown on-chain session {}", chain_sid_u64);
                    }

                    let registry_addr = escrow_read
                        .registry()
                        .call()
                        .await
                        .context("registry() eth_call failed")?;
                    let registry_read = ProviderRegistry::new(registry_addr, &read_provider);
                    let node_operator = registry_read
                        .nodeOperator(chain_sess.nodeId)
                        .call()
                        .await
                        .context("nodeOperator eth_call failed")?;
                    if node_operator_signer.address() != node_operator {
                        anyhow::bail!(
                            "node operator signer {} is not on-chain nodeOperator {} for session nodeId {:?}",
                            node_operator_signer.address(),
                            node_operator,
                            chain_sess.nodeId
                        );
                    }

                    let local_total = bill_internal(
                        session.amount_micro_usd,
                        settlement.usage_internal_units_per_micro_usd,
                    )?;

                    let usage_pre = chain_sess.usageRecorded;
                    if !usage_within_tee_tolerance(
                        local_total,
                        usage_pre,
                        settlement.usage_tolerance_bps,
                    ) {
                        warn!(
                            session_uuid = %session.id,
                            chain_session_id = chain_sid_u64,
                            usage = %usage_pre,
                            local = %local_total,
                            bps = settlement.usage_tolerance_bps,
                            "tee settlement skipped: usageRecorded exceeds local bill beyond tolerance"
                        );
                        return Ok(SettleOutcome::Noop);
                    }

                    let delta_sync = local_total.saturating_sub(usage_pre);
                    if delta_sync > U256::ZERO {
                        let pending_tx = escrow_node_operator
                            .recordUsage(session_id, delta_sync)
                            .send()
                            .await
                            .context("recordUsage send failed")?;
                        let tx_hash = pending_tx
                            .with_required_confirmations(1)
                            .watch()
                            .await
                            .context("recordUsage confirmation failed")?;
                        info!(
                            session_uuid = %session.id,
                            chain_session_id = chain_sid_u64,
                            ?tx_hash,
                            "recordUsage confirmed"
                        );
                    }

                    let chain_sess = escrow_read
                        .sessions(session_id)
                        .call()
                        .await
                        .context("sessions(sessionId) refresh after recordUsage failed")?;

                    if chain_sess.settled {
                        sessions.close(session.id, SessionState::Settled);
                        return Ok(SettleOutcome::Noop);
                    }

                    let locked = chain_sess.lockedInternal;
                    if locked == U256::ZERO {
                        return Ok(SettleOutcome::Noop);
                    }

                    let usage = chain_sess.usageRecorded;
                    let paid = chain_sess.paidToProviderInternal;
                    let remaining_claim = usage.saturating_sub(paid);
                    let cap_local_remaining = local_total.saturating_sub(paid);

                    let (to_provider, to_user, full_drain, phase) = if partial {
                        let anchor = session.evm_tee_anchor_tokens;
                        let delta_tokens = session.tokens_output.saturating_sub(anchor);
                        let micro_slice_u128 = (session.amount_micro_usd as u128)
                            .saturating_mul(delta_tokens as u128)
                            / u128::from(session.tokens_output.max(1));
                        let micro_slice = u64::try_from(micro_slice_u128).unwrap_or(u64::MAX);

                        let slice_internal = bill_internal(
                            micro_slice,
                            settlement.usage_internal_units_per_micro_usd,
                        )?;

                        let fair_increment = slice_internal
                            .min(cap_local_remaining)
                            .min(remaining_claim)
                            .min(locked);

                        (fair_increment, U256::ZERO, false, "tee streaming partial")
                    } else {
                        let to_provider_full = remaining_claim.min(locked).min(cap_local_remaining);
                        let to_user_full = locked.saturating_sub(to_provider_full);
                        (
                            to_provider_full,
                            to_user_full,
                            true,
                            "tee completed full drain",
                        )
                    };

                    let out = to_provider.saturating_add(to_user);
                    if out == U256::ZERO {
                        return Ok(SettleOutcome::Noop);
                    }

                    let pending_tx = if full_drain {
                        escrow_operator
                            .settleByOperatorFull(session_id, to_provider, to_user)
                            .send()
                            .await
                            .context("settleByOperatorFull send failed")?
                    } else {
                        escrow_operator
                            .settleByOperatorPartial(session_id, to_provider, to_user)
                            .send()
                            .await
                            .context("settleByOperatorPartial send failed")?
                    };

                    let tx_hash = pending_tx
                        .with_required_confirmations(1)
                        .watch()
                        .await
                        .context("operator settle confirmation failed")?;

                    if partial {
                        sessions.update_evm_tee_anchor_tokens(session.id, session.tokens_output);
                        info!(
                            session_uuid = %session.id,
                            chain_session_id = chain_sid_u64,
                            ?tx_hash,
                            to_provider = %to_provider,
                            to_user = %to_user,
                            anchor_tokens = session.tokens_output,
                            phase,
                        );
                    } else {
                        sessions.close(session.id, SessionState::Settled);
                        info!(
                            session_uuid = %session.id,
                            chain_session_id = chain_sid_u64,
                            ?tx_hash,
                            to_provider = %to_provider,
                            to_user = %to_user,
                            phase,
                        );
                    }

                    Ok(SettleOutcome::DidMutateChain)
                }
                .await;

                match res {
                    Ok(SettleOutcome::DidMutateChain) => advanced_eligibility_anchor = true,
                    Ok(SettleOutcome::Noop) => {}
                    Err(e) => warn!(
                        session_uuid = %session.id,
                        error = %e,
                        "evm tee-verified settlement failed for session"
                    ),
                }
            }
            if advanced_eligibility_anchor {
                if let Some(h) = chain_head_opt {
                    *tee_last_eligible_block = Some(h);
                }
            }
        }
    }

    if epoch_boundary {
        let pending = sessions.pending_best_effort_completed_settlement();
        for session in pending {
            let res = async {
                let Some(chain_sid_u64) = session.evm_session_id else {
                    warn!(
                        session_uuid = %session.id,
                        "skip best-effort settlement: session.evm_session_id not set"
                    );
                    return Ok::<(), anyhow::Error>(());
                };

                let session_id = U256::from(chain_sid_u64);

                let chain_sess = escrow_read
                    .sessions(session_id)
                    .call()
                    .await
                    .context("sessions(sessionId) eth_call failed")?;

                if chain_sess.settled {
                    sessions.close(session.id, SessionState::Settled);
                    return Ok(());
                }

                if chain_sess.user == Address::ZERO {
                    anyhow::bail!("unknown on-chain session {}", chain_sid_u64);
                }

                let registry_addr = escrow_read
                    .registry()
                    .call()
                    .await
                    .context("registry() eth_call failed")?;
                let registry_read = ProviderRegistry::new(registry_addr, &read_provider);
                let node_operator = registry_read
                    .nodeOperator(chain_sess.nodeId)
                    .call()
                    .await
                    .context("nodeOperator eth_call failed")?;
                if node_operator_signer.address() != node_operator {
                    anyhow::bail!(
                        "node operator signer {} is not on-chain nodeOperator {} for session nodeId {:?}",
                        node_operator_signer.address(),
                        node_operator,
                        chain_sess.nodeId
                    );
                }

                let local_total = bill_internal(
                    session.amount_micro_usd,
                    settlement.usage_internal_units_per_micro_usd,
                )?;

                let usage_pre = chain_sess.usageRecorded;
                let delta_sync = local_total.saturating_sub(usage_pre);
                if delta_sync > U256::ZERO {
                    let pending_tx = escrow_node_operator
                        .recordUsage(session_id, delta_sync)
                        .send()
                        .await
                        .context("recordUsage send failed")?;
                    let tx_hash = pending_tx
                        .with_required_confirmations(1)
                        .watch()
                        .await
                        .context("recordUsage confirmation failed")?;
                    info!(
                        session_uuid = %session.id,
                        chain_session_id = chain_sid_u64,
                        ?tx_hash,
                        "recordUsage confirmed"
                    );
                }

                let chain_sess = escrow_read
                    .sessions(session_id)
                    .call()
                    .await
                    .context("sessions(sessionId) refresh after recordUsage failed")?;

                if chain_sess.settled {
                    sessions.close(session.id, SessionState::Settled);
                    return Ok(());
                }

                let locked = chain_sess.lockedInternal;
                if locked == U256::ZERO {
                    return Ok(());
                }

                let usage = chain_sess.usageRecorded;
                let paid = chain_sess.paidToProviderInternal;
                let remaining_claim = usage.saturating_sub(paid);
                let cap_local = local_total.saturating_sub(paid);
                let to_provider = remaining_claim.min(locked).min(cap_local);
                let to_user = locked.saturating_sub(to_provider);

                let pending_tx = escrow_operator
                    .settleByOperatorFull(session_id, to_provider, to_user)
                    .send()
                    .await
                    .context("settleByOperatorFull send failed")?;
                let tx_hash = pending_tx
                    .with_required_confirmations(1)
                    .watch()
                    .await
                    .context("settleByOperatorFull confirmation failed")?;

                sessions.close(session.id, SessionState::Settled);

                info!(
                    session_uuid = %session.id,
                    chain_session_id = chain_sid_u64,
                    ?tx_hash,
                    to_provider = %to_provider,
                    to_user = %to_user,
                    tier = ?SecurityTier::BestEffort,
                    "settleByOperatorFull confirmed (best-effort epoch)"
                );

                Ok(())
            }
            .await;

            if let Err(e) = res {
                warn!(
                    session_uuid = %session.id,
                    error = %e,
                    "evm best-effort settlement failed for session"
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettleOutcome {
    Noop,
    DidMutateChain,
}

fn tee_gate_open(
    settlement: &SettlementConfig,
    chain_head: Option<u64>,
    last_eligible: Option<&u64>,
) -> bool {
    if settlement.tee_settle_every_n_blocks == 0 {
        return true;
    }
    let Some(h) = chain_head else {
        return false;
    };
    match last_eligible {
        None => true,
        Some(last) => h >= last.saturating_add(settlement.tee_settle_every_n_blocks),
    }
}

async fn fetch_head_block_number<P: Provider>(provider: &P) -> Result<u64> {
    let n = provider
        .get_block_number()
        .await
        .context("get_block_number eth_call failed")?;
    Ok(n.into())
}

fn usage_within_tee_tolerance(local_total: U256, usage: U256, bps: u16) -> bool {
    let slack = local_total.saturating_mul(U256::from(bps)) / U256::from(10_000u64);
    usage <= local_total.saturating_add(slack)
}

/// Open a session on the Hub EVM `SettlementEscrow` contract.
///
/// Reads `nextSessionId()` before sending the tx so the caller knows the
/// session ID that will be assigned (the contract does `uint256 id = nextSessionId++`).
///
/// Returns `Ok(session_id)` on success, or `Err` if EVM settlement is not
/// configured / the call fails. When the provider wallet key is missing or
/// the escrow contract is unset, returns `Ok(None)` (graceful degradation).
pub async fn open_session_on_chain(
    escrow_addr_str: &str,
    rpc_url: &str,
    provider_pk: &str,
    node_id: [u8; 32],
    tier: SecurityTier,
    min_deposit: u64,
) -> Result<Option<u64>> {
    let escrow_addr: Address = escrow_addr_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid escrow_contract address: {e}"))?;

    if escrow_addr == Address::ZERO {
        warn!("escrow_contract is zero address; skipping on-chain session open");
        return Ok(None);
    }

    let rpc_url_parsed = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let pk = provider_pk.trim();
    if pk.is_empty() {
        warn!("settlement.evm_provider_wallet_private_key not configured; skipping on-chain session open");
        return Ok(None);
    }

    let signer = signer_from_hex(pk)?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .fetch_chain_id()
        .connect_http(rpc_url_parsed);

    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    // Read the next session ID before opening (contract does id = nextSessionId++)
    let next_id = escrow.nextSessionId().call().await.map_err(|e| {
        anyhow::anyhow!("failed to read nextSessionId: {e}")
    })?;

    let chain_sid: u64 = next_id.try_into().map_err(|_| {
        anyhow::anyhow!("nextSessionId overflowed u64")
    })?;

    let tier_u8 = match tier {
        SecurityTier::BestEffort => 0u8,
        SecurityTier::TeeVerified => 1u8,
    };

    let deposit_u256 = U256::from(min_deposit);

    info!(
        session_id = chain_sid,
        node_id = %hex::encode(node_id),
        tier = ?tier,
        deposit = %min_deposit,
        "opening session on SettlementEscrow"
    );

    let pending = escrow
        .openSession(node_id.into(), tier_u8, deposit_u256)
        .value(deposit_u256)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("openSession send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow::anyhow!("openSession confirmation failed: {e}"))?;

    info!(
        session_id = chain_sid,
        ?tx_hash,
        "session opened on-chain successfully"
    );

    Ok(Some(chain_sid))
}

fn signer_from_hex(key: &str) -> Result<PrivateKeySigner> {
    let hex_str = key.strip_prefix("0x").unwrap_or(key).trim();
    let bytes = hex::decode(hex_str).context("private key hex decode failed")?;
    PrivateKeySigner::from_slice(&bytes).map_err(|e| anyhow::anyhow!("invalid signing key: {e}"))
}

fn bill_internal(micro_usd: u64, units_per_micro_usd: u128) -> Result<U256> {
    let u = U256::from(micro_usd);
    let rate = U256::from(units_per_micro_usd);
    u.checked_mul(rate)
        .ok_or_else(|| anyhow::anyhow!("bill_internal overflow"))
}
