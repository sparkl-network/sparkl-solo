#!/usr/bin/env bash
# Deploy local contracts to Anvil and sync addresses into sparkl component env files.
#
# Usage:
#   ./scripts/deploy-local-sync-env.sh              # deploy + sync (Anvil must be up)
#   ./scripts/deploy-local-sync-env.sh --start-anvil # start Anvil if missing, then deploy
#   ./scripts/deploy-local-sync-env.sh --sync-only   # read deployments/local.json only
#   ./scripts/deploy-local-sync-env.sh --no-seed     # skip ModelPriceOracle defaultPrice seed
#   ./scripts/deploy-local-sync-env.sh --reset-chain # wipe Anvil state file (restart Anvil after)
#
# Requires: forge, cast, jq, python3
# Optional: anvil (with --start-anvil)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/local-deploy-common.sh
source "${ROOT}/scripts/lib/local-deploy-common.sh"

CONTRACTS="${ROOT}/contracts"
DEPLOY_JSON="${CONTRACTS}/deployments/local.json"
NETWORK_ROOT="$(cd "${ROOT}/.." && pwd)"
SOLO_ROOT="${ROOT}"
PORTAL_ROOT="${NETWORK_ROOT}/sparkl-portal"
ROUTER_ROOT="${NETWORK_ROOT}/sparkl-router"
ORACLE_RATES_ROOT="${NETWORK_ROOT}/sparkl-oracle-rates"
ORACLE_MODEL_ROOT="${NETWORK_ROOT}/sparkl-oracle-model-price"

ANVIL_HOST="${ANVIL_HOST:-127.0.0.1}"
ANVIL_PORT="${ANVIL_PORT:-8545}"
ANVIL_RPC_LOCAL="http://127.0.0.1:${ANVIL_PORT}"
ANVIL_RPC="${ANVIL_RPC:-${ANVIL_RPC_LOCAL}}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
ANVIL_STATE="${ROOT}/.launch/anvil-state.json"
ANVIL_LOG="${ROOT}/.launch/anvil.log"

ROUTER_HOST="${ROUTER_HOST:-127.0.0.1}"
ROUTER_PORT="${ROUTER_PORT:-3001}"
ROUTER_BIND="${ROUTER_BIND:-${ROUTER_HOST}:${ROUTER_PORT}}"
ROUTER_URL="${ROUTER_URL:-http://${ROUTER_HOST}:${ROUTER_PORT}}"
ROUTER_ADMIN_TOKEN="${ROUTER_ADMIN_TOKEN:-dev-admin-token-change-me}"

DEFAULT_USDC_PER_DOT="${ORACLE_USDC_PER_DOT:-1340000}"

ANVIL_DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ANVIL_ORACLE_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
ANVIL_DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
ANVIL_ORACLE_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
# Anvil mnemonic index 8 (64-byte hex); must match `cast wallet private-key --mnemonic "test test test test test test test test test test test junk" --mnemonic-index 8`
ANVIL_RECORD_USAGE_PK="0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97"
ANVIL_RECORD_USAGE_ADDR="0x23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f"
ANVIL_PROTOCOL_TREASURY_ADDR="0xa0Ee7A142d267C1b36734E279844e1E4fed538508"

START_ANVIL=0
SYNC_ONLY=0
NO_SEED=0
RESET_CHAIN=0
ANVIL_PID=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --start-anvil) START_ANVIL=1 ;;
    --sync-only) SYNC_ONLY=1 ;;
    --no-seed) NO_SEED=1 ;;
    --reset-chain) RESET_CHAIN=1 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
  shift
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

cleanup_anvil() {
  if [[ -n "${ANVIL_PID}" ]] && kill -0 "${ANVIL_PID}" 2>/dev/null; then
    kill "${ANVIL_PID}" 2>/dev/null || true
  fi
}

require_cmd forge
require_cmd cast
require_cmd jq
require_cmd python3

if [[ "${START_ANVIL}" -eq 1 ]]; then
  require_cmd anvil
fi

mkdir -p "${ROOT}/.launch" "${ROOT}/dev-config"

if [[ "${RESET_CHAIN}" -eq 1 ]]; then
  if [[ -f "${ANVIL_STATE}" ]]; then
    rm -f "${ANVIL_STATE}"
    echo "Removed Anvil state: ${ANVIL_STATE}"
  fi
  echo "Stop any running Anvil on port ${ANVIL_PORT}, then rerun with --start-anvil or start Anvil manually."
  if [[ "${SYNC_ONLY}" -eq 1 ]]; then
    exit 0
  fi
fi

if [[ "${SYNC_ONLY}" -eq 0 ]]; then
  if ! local_deploy_wait_for_rpc 2>/dev/null; then
    if [[ "${START_ANVIL}" -eq 1 ]]; then
      echo "Starting Anvil at ${ANVIL_RPC}..."
      anvil --host "${ANVIL_HOST}" --port "${ANVIL_PORT}" --chain-id "${ANVIL_CHAIN_ID}" \
        --state "${ANVIL_STATE}" >"${ANVIL_LOG}" 2>&1 &
      ANVIL_PID=$!
      trap cleanup_anvil EXIT INT TERM
      local_deploy_wait_for_rpc
    else
      echo "Anvil is not reachable at ${ANVIL_RPC}." >&2
      echo "  Start it: cd ${CONTRACTS} && anvil --state ${ANVIL_STATE}" >&2
      echo "  Or rerun with: ./scripts/deploy-local-sync-env.sh --start-anvil" >&2
      exit 1
    fi
  fi

  export PRIVATE_KEY="${PRIVATE_KEY:-${ANVIL_DEPLOYER_PK}}"
  if [[ -z "${ORACLE_UPDATER_ADDRESS:-}" ]]; then
    export ORACLE_UPDATER_ADDRESS="${ANVIL_ORACLE_ADDR}"
  fi
  export ORACLE_MAX_STALENESS="${ORACLE_MAX_STALENESS:-3600}"

  echo "Building contracts..."
  (cd "${CONTRACTS}" && forge build --quiet)

  if [[ -x "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" ]]; then
    echo "Syncing contract ABIs to portal and solo..."
    "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" --no-build
  else
    echo "Warning: ${NETWORK_ROOT}/scripts/sync-contract-abis.sh not found; ABIs may be stale." >&2
  fi

  echo "Deploying contracts (DeployLocal) to ${ANVIL_RPC}..."
  if ! (
    cd "${CONTRACTS}"
    forge script script/DeployLocal.s.sol:DeployLocal \
      --rpc-url "${ANVIL_RPC}" \
      --broadcast
  ); then
    echo "Deploy failed." >&2
    echo "  Stale mixed chain state? Stop Anvil, then: ./scripts/deploy-local-sync-env.sh --reset-chain --start-anvil" >&2
    exit 1
  fi
fi

if [[ ! -f "${DEPLOY_JSON}" ]]; then
  echo "Expected deployments file missing: ${DEPLOY_JSON}" >&2
  exit 1
fi

RATE_SETTER="$(local_deploy_json_field '.rateSetter // .priceOracle')"
MODEL_PRICE_ORACLE="$(local_deploy_json_field '.modelPriceOracle // empty')"
REGISTRY="$(local_deploy_json_field '.providerRegistry')"
ESCROW="$(local_deploy_json_field '.settlementEscrow')"
NET_CFG="$(local_deploy_json_field '.sparklNetworkConfig')"
ORACLE_UPDATER="$(local_deploy_json_field '.oracleUpdater')"
CHAIN_ID="$(local_deploy_json_field '.chainId')"

if [[ "${SYNC_ONLY}" -eq 0 && "${NO_SEED}" -eq 0 ]]; then
  if local_deploy_wait_for_rpc 2>/dev/null; then
    local_deploy_seed_model_default_price_if_needed "${MODEL_PRICE_ORACLE}" "${RATE_SETTER}" || true
  else
    echo "Skipping on-chain seed (RPC not reachable)."
  fi
fi

local_deploy_sync_component_envs

echo ""
echo "Done. Contract addresses:"
echo "  RPC:                 ${ANVIL_RPC}"
echo "  Chain ID:            ${CHAIN_ID}"
echo "  ProviderRegistry:    ${REGISTRY}"
echo "  SettlementEscrow:    ${ESCROW}"
echo "  ModelPriceOracle:    ${MODEL_PRICE_ORACLE}"
echo "  RateSetter:          ${RATE_SETTER}"
echo "  SparklNetworkConfig: ${NET_CFG}"
echo "  Oracle updater:      ${ORACLE_UPDATER}"
echo ""
echo "Next steps:"
echo "  1. Restart sparkl-portal after NEXT_PUBLIC_* changes: cd ${PORTAL_ROOT} && yarn dev"
echo "  2. Restart sparkl-router if running: cargo run -- ${SOLO_ROOT}/dev-config/router-launch.toml"
echo "  3. Register nodes at /node/register (router needs on-chain registration when chain.enabled = true)"
echo ""
echo "Deployments JSON: ${DEPLOY_JSON}"
