//! On-chain settlement via Hub EVM `SettlementEscrow`.
#![cfg(feature = "evm-settlement")]

use std::sync::Arc;

use alloy::{
    primitives::{Address, FixedBytes, U256},
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

/// On-chain `SettlementEscrow.sessions(sessionId)` view (subset used for HTTP auth).
#[derive(Debug, Clone)]
pub struct ChainSession {
    pub user: Address,
    pub node_id: FixedBytes<32>,
    pub settled: bool,
}

/// Read-only `sessions(sessionId)` eth_call.
pub async fn fetch_chain_session(
    escrow_addr: &str,
    rpc_url: &str,
    session_id: u64,
) -> Result<ChainSession> {
    let escrow_addr: Address = escrow_addr
        .trim()
        .parse()
        .context("invalid settlement.escrow_contract")?;
    if escrow_addr == Address::ZERO {
        anyhow::bail!("settlement.escrow_contract is zero address");
    }

    let rpc_url = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    let chain_sess = escrow
        .sessions(U256::from(session_id))
        .call()
        .await
        .context("sessions(sessionId) eth_call failed")?;

    Ok(ChainSession {
        user: chain_sess.user,
        node_id: chain_sess.nodeId,
        settled: chain_sess.settled,
    })
}

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

    let chain_head_opt = if tee_tick_now
        && !tee_candidates.is_empty()
        && settlement.tee_settle_every_n_blocks > 0
    {
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

                    sync_token_usage_on_chain(
                        &escrow_node_operator,
                        sessions.clone(),
                        session_id,
                        &session,
                        chain_sid_u64,
                    )
                    .await?;

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

                    let (to_provider, to_user, full_drain, phase) = if partial {
                        let fair_increment = remaining_claim.min(locked);
                        (fair_increment, U256::ZERO, false, "tee streaming partial")
                    } else {
                        let to_provider_full = remaining_claim.min(locked);
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

                sync_token_usage_on_chain(
                    &escrow_node_operator,
                    sessions.clone(),
                    session_id,
                    &session,
                    chain_sid_u64,
                )
                .await?;

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
                let to_provider = remaining_claim.min(locked);
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
    Ok(n)
}

async fn sync_token_usage_on_chain<P>(
    escrow: &SettlementEscrow::SettlementEscrowInstance<P>,
    sessions: Arc<SessionManager>,
    session_id: U256,
    session: &Session,
    chain_sid_u64: u64,
) -> Result<()>
where
    P: Provider + Clone,
{
    let input_delta = session
        .tokens_input
        .saturating_sub(session.evm_input_tokens_synced);
    let output_delta = session
        .tokens_output
        .saturating_sub(session.evm_output_tokens_synced);
    if input_delta == 0 && output_delta == 0 {
        return Ok(());
    }

    let pending_tx = escrow
        .recordUsage(
            session_id,
            U256::from(input_delta),
            U256::from(output_delta),
        )
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
        input_delta,
        output_delta,
        ?tx_hash,
        "recordUsage confirmed"
    );
    sessions.update_evm_tokens_synced(session.id, session.tokens_input, session.tokens_output);
    Ok(())
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
    model_name: &str,
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
    let model_id = FixedBytes::<32>::from(alloy::primitives::keccak256(model_name.as_bytes()));

    info!(
        session_id = chain_sid,
        node_id = %hex::encode(node_id),
        model = %model_name,
        tier = ?tier,
        deposit = %min_deposit,
        "opening session on SettlementEscrow"
    );

    let pending = escrow
        .openSession(node_id.into(), tier_u8, model_id, deposit_u256)
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

// ---------------------------------------------------------------------------
// Deposit / Withdraw helpers (issues #7, #8, #9)
// ---------------------------------------------------------------------------

/// Deposit native DOT into the escrow contract on behalf of the node operator.
pub async fn deposit_dot(
    escrow_addr_str: &str,
    rpc_url: &str,
    provider_pk: &str,
    amount_native: u64,
) -> Result<Option<String>> {
    if amount_native == 0 {
        warn!("deposit_dot called with zero amount; skipping");
        return Ok(None);
    }

    let escrow_addr: Address = escrow_addr_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid escrow_contract address: {e}"))?;

    if escrow_addr == Address::ZERO {
        warn!("escrow_contract is zero address; skipping deposit");
        return Ok(None);
    }

    let pk = provider_pk.trim();
    if pk.is_empty() {
        warn!("settlement.evm_provider_wallet_private_key not configured; skipping deposit");
        return Ok(None);
    }

    let signer = signer_from_hex(pk)?;

    let rpc_url_parsed = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .fetch_chain_id()
        .connect_http(rpc_url_parsed);

    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    let amount_u256 = U256::from(amount_native);

    info!(
        amount_native = %amount_native,
        "depositing native DOT into SettlementEscrow"
    );

    let pending = escrow
        .depositDot()
        .value(amount_u256)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("depositDot send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow::anyhow!("depositDot confirmation failed: {e}"))?;

    info!(?tx_hash, "deposit_dot confirmed");

    Ok(Some(tx_hash.to_string()))
}

/// Deposit USDC as DOT by calling `depositUsdcAsDot` on the escrow contract.
///
/// The escrow contract handles the USDC transfer + oracle conversion internally.
/// `min_dot_internal_out` and `max_oracle_age_secs` control slippage and staleness.
///
/// Returns the transaction hash on success, or `Ok(None)` on graceful degradation.
pub async fn deposit_usdc_as_dot(
    escrow_addr_str: &str,
    rpc_url: &str,
    provider_pk: &str,
    usdc_amount: u64,
    min_dot_internal_out: u64,
    max_oracle_age_secs: u64,
) -> Result<Option<String>> {
    if usdc_amount == 0 {
        warn!("deposit_usdc_as_dot called with zero amount; skipping");
        return Ok(None);
    }

    let escrow_addr: Address = escrow_addr_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid escrow_contract address: {e}"))?;

    if escrow_addr == Address::ZERO {
        warn!("escrow_contract is zero address; skipping USDC deposit");
        return Ok(None);
    }

    let pk = provider_pk.trim();
    if pk.is_empty() {
        warn!("settlement.evm_provider_wallet_private_key not configured; skipping USDC deposit");
        return Ok(None);
    }

    let signer = signer_from_hex(pk)?;

    let rpc_url_parsed = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .fetch_chain_id()
        .connect_http(rpc_url_parsed);

    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    let usdc_u256 = U256::from(usdc_amount);
    let min_dot_u256 = U256::from(min_dot_internal_out);
    let max_age_u256 = U256::from(max_oracle_age_secs);

    info!(
        usdc_amount = %usdc_amount,
        min_dot_out = %min_dot_internal_out,
        "depositing USDC as DOT via SettlementEscrow"
    );

    let pending = escrow
        .depositUsdcAsDot_1(usdc_u256, min_dot_u256, max_age_u256)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("depositUsdcAsDot_1 send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow::anyhow!("depositUsdcAsDot confirmation failed: {e}"))?;

    info!(?tx_hash, "deposit_usdc_as_dot confirmed");

    Ok(Some(tx_hash.to_string()))
}

/// Withdraw native DOT from the escrow contract.
///
/// Burns `amount_internal` from the caller's internal DOT balance and sends
/// the corresponding native amount to the caller's address.
///
/// Returns the transaction hash on success, or `Ok(None)` on graceful degradation.
pub async fn withdraw_dot(
    escrow_addr_str: &str,
    rpc_url: &str,
    provider_pk: &str,
    amount_internal: u64,
) -> Result<Option<String>> {
    if amount_internal == 0 {
        warn!("withdraw_dot called with zero amount; skipping");
        return Ok(None);
    }

    let escrow_addr: Address = escrow_addr_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid escrow_contract address: {e}"))?;

    if escrow_addr == Address::ZERO {
        warn!("escrow_contract is zero address; skipping withdraw");
        return Ok(None);
    }

    let pk = provider_pk.trim();
    if pk.is_empty() {
        warn!("settlement.evm_provider_wallet_private_key not configured; skipping withdraw");
        return Ok(None);
    }

    let signer = signer_from_hex(pk)?;

    let rpc_url_parsed = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .fetch_chain_id()
        .connect_http(rpc_url_parsed);

    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    let amount_u256 = U256::from(amount_internal);

    info!(
        amount_internal = %amount_internal,
        "withdrawing native DOT from SettlementEscrow"
    );

    let pending = escrow
        .withdrawDot(amount_u256)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("withdrawDot send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow::anyhow!("withdrawDot confirmation failed: {e}"))?;

    info!(?tx_hash, "withdraw_dot confirmed");

    Ok(Some(tx_hash.to_string()))
}

/// Withdraw provider earnings from the escrow contract.
///
/// Only callable by the registry owner (or settlement operator if configured).
/// Burns `amount_internal` from the provider's internal DOT balance and sends
/// the corresponding native amount to the provider's EVM address.
///
/// Returns the transaction hash on success, or `Ok(None)` on graceful degradation.
pub async fn withdraw_provider_dot(
    escrow_addr_str: &str,
    rpc_url: &str,
    operator_pk: &str,
    node_id: [u8; 32],
    amount_internal: u64,
) -> Result<Option<String>> {
    if amount_internal == 0 {
        warn!("withdraw_provider_dot called with zero amount; skipping");
        return Ok(None);
    }

    let escrow_addr: Address = escrow_addr_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid escrow_contract address: {e}"))?;

    if escrow_addr == Address::ZERO {
        warn!("escrow_contract is zero address; skipping provider withdraw");
        return Ok(None);
    }

    let pk = operator_pk.trim();
    if pk.is_empty() {
        warn!("settlement.evm_settlement_operator_wallet_private_key not configured; skipping provider withdraw");
        return Ok(None);
    }

    let signer = signer_from_hex(pk)?;

    let rpc_url_parsed = rpc_url
        .trim()
        .parse::<reqwest::Url>()
        .context("invalid settlement.evm_rpc_url")?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .fetch_chain_id()
        .connect_http(rpc_url_parsed);

    let escrow = SettlementEscrow::new(escrow_addr, &provider);

    let amount_u256 = U256::from(amount_internal);

    info!(
        node_id = %hex::encode(node_id),
        amount_internal = %amount_internal,
        "withdrawing provider earnings from SettlementEscrow"
    );

    let pending = escrow
        .withdrawProviderDot(node_id.into(), amount_u256)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("withdrawProviderDot send failed: {e}"))?;

    let tx_hash = pending
        .with_required_confirmations(1)
        .watch()
        .await
        .map_err(|e| anyhow::anyhow!("withdrawProviderDot confirmation failed: {e}"))?;

    info!(?tx_hash, "withdraw_provider_dot confirmed");

    Ok(Some(tx_hash.to_string()))
}
