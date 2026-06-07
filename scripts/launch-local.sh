#!/usr/bin/env bash
# Local dev stack: Anvil + contract deploy + sparkl-solo tests + sparkl-router + solo node.
# Prints env hints for sparkl-portal and sparkl-oracle-rates (DOT/USD).
#
# Usage:
#   ./scripts/launch-local.sh              # full stack; node runs until Ctrl+C
#   ./scripts/launch-local.sh --skip-node  # deploy + tests; leaves Anvil running
#   ./scripts/launch-local.sh --skip-router # do not start sparkl-router
#   ./scripts/launch-local.sh --skip-tests # faster iteration
#   ./scripts/launch-local.sh --keep-anvil # do not stop Anvil on exit
#   ./scripts/launch-local.sh --no-state   # ephemeral Anvil (no state file)
#   ./scripts/launch-local.sh --force-deploy
#   ./scripts/launch-local.sh --skip-deploy
#
# Anvil listen address (default all interfaces for LAN / remote RPC):
#   ANVIL_HOST=0.0.0.0 ./scripts/launch-local.sh   # remote can use http://<LAN-IP>:8545
#   ANVIL_HOST=127.0.0.1 ./scripts/launch-local.sh # localhost only
#
# Interactive tmux grid: ../scripts/launch-grid.sh (from sparkl-network/)
#
# Requires: forge, anvil, cast, cargo, jq

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=launch-local-lib.sh
source "${SCRIPT_DIR}/launch-local-lib.sh"
launch_local_init_paths "${ROOT}"

SKIP_TESTS=0
SKIP_NODE=0
SKIP_ROUTER=0
KEEP_ANVIL=0
ANVIL_USE_STATE=1
FORCE_DEPLOY=0
SKIP_DEPLOY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-tests) SKIP_TESTS=1 ;;
    --skip-node) SKIP_NODE=1; KEEP_ANVIL=1 ;;
    --skip-router) SKIP_ROUTER=1 ;;
    --keep-anvil) KEEP_ANVIL=1 ;;
    --no-state) ANVIL_USE_STATE=0 ;;
    --force-deploy) FORCE_DEPLOY=1 ;;
    --skip-deploy) SKIP_DEPLOY=1 ;;
    -h|--help)
      sed -n '2,16p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
  shift
done

ANVIL_PID=""
NODE_PID=""
ROUTER_PID=""
REUSED_ANVIL=0
REUSED_ROUTER=0

cleanup() {
  local code=$?
  if [[ -n "${NODE_PID}" ]] && kill -0 "${NODE_PID}" 2>/dev/null; then
    kill "${NODE_PID}" 2>/dev/null || true
  fi
  if [[ "${REUSED_ROUTER}" -eq 0 ]] && [[ -n "${ROUTER_PID}" ]] && kill -0 "${ROUTER_PID}" 2>/dev/null; then
    kill "${ROUTER_PID}" 2>/dev/null || true
  fi
  if [[ "${KEEP_ANVIL}" -eq 0 ]] && [[ -n "${ANVIL_PID}" ]] && kill -0 "${ANVIL_PID}" 2>/dev/null; then
    kill "${ANVIL_PID}" 2>/dev/null || true
  fi
  if [[ $code -ne 0 ]]; then
    echo "launch-local.sh exited with status ${code}" >&2
  fi
}
trap cleanup EXIT INT TERM

require_cmd forge
require_cmd anvil
require_cmd cast
require_cmd cargo
require_cmd jq
require_cmd python3

mkdir -p "${PID_DIR}" "${ROOT}/dev-config" "${ROOT}/dev-data/launch"

print_banner "Starting Anvil"
if anvil_rpc_ready; then
  REUSED_ANVIL=1
  echo "Reusing existing Anvil at ${ANVIL_RPC} (not started by this script)"
else
  if [[ "${ANVIL_USE_STATE}" -eq 1 ]]; then
    echo "Anvil state: ${ANVIL_STATE}"
  else
    echo "Anvil: ephemeral (--no-state)"
  fi
  start_anvil_background
  echo "Anvil listen: ${ANVIL_HOST}:${ANVIL_PORT} (local RPC: ${ANVIL_RPC})"
  if [[ "${ANVIL_HOST}" != "127.0.0.1" && "${ANVIL_HOST}" != "localhost" ]]; then
    lan_ip="$(lan_ipv4 || true)"
    if [[ -n "${lan_ip}" ]]; then
      echo "Anvil LAN RPC: http://${lan_ip}:${ANVIL_PORT}"
    fi
    echo "Ensure firewall allows inbound TCP ${ANVIL_PORT} if remote machines cannot connect."
  fi
  wait_for_rpc
fi

if [[ "${REUSED_ANVIL:-0}" -eq 1 && "${ANVIL_HOST}" != "127.0.0.1" ]]; then
  echo "Note: reusing Anvil — remote RPC needs it started with: anvil --host 0.0.0.0 --port ${ANVIL_PORT}"
fi

export PRIVATE_KEY="${PRIVATE_KEY:-${ANVIL_DEPLOYER_PK}}"
if [[ -z "${ORACLE_UPDATER_ADDRESS:-}" ]]; then
  export ORACLE_UPDATER_ADDRESS="${ANVIL_ORACLE_ADDR}"
fi
export ORACLE_MAX_STALENESS="${ORACLE_MAX_STALENESS:-3600}"

run_forge_build
run_abi_sync
run_deploy_if_needed
load_deploy_addresses
write_deploy_fingerprint || true

print_banner "Seeding on-chain defaults (if needed)"
seed_model_default_price_if_needed "${MODEL_PRICE_ORACLE}" "${RATE_SETTER}"

print_banner "Contract addresses"
echo "RPC URL:              ${ANVIL_RPC}"
echo "Chain ID:             ${CHAIN_ID}"
echo "Deployer:             ${ANVIL_DEPLOYER_ADDR}"
echo "RateSetter:           ${RATE_SETTER}"
echo "ModelPriceOracle:     ${MODEL_PRICE_ORACLE}"
echo "ProviderRegistry:     ${REGISTRY}"
echo "SettlementEscrow:     ${ESCROW}"
echo "SparklNetworkConfig:  ${NET_CFG}"
echo "Mock USDC:            ${MOCK_USDC}"
echo "Oracle updater:       ${ORACLE_UPDATER}"

if [[ "${SKIP_TESTS}" -eq 0 ]]; then
  print_banner "Forge contract tests"
  (cd "${CONTRACTS}" && forge test)

  print_banner "sparkl-solo Rust tests (mock-tpm)"
  (cd "${ROOT}" && cargo test --features mock-tpm)
else
  echo "(skipped tests: --skip-tests)"
fi

if [[ "${SKIP_ROUTER}" -eq 0 ]]; then
  if [[ ! -f "${ROUTER_ROOT}/Cargo.toml" ]]; then
    echo "ERROR: sparkl-router not found at ${ROUTER_ROOT}" >&2
    echo "  Expected sibling checkout: sparkl-network/sparkl-router" >&2
    exit 1
  fi

  print_banner "Writing router config ${ROUTER_CONFIG}"
  write_router_launch_config

  print_banner "Starting sparkl-router"
  if curl -sf "${ROUTER_URL}/health" >/dev/null 2>&1; then
    REUSED_ROUTER=1
    echo "Reusing existing sparkl-router at ${ROUTER_URL} (not started by this script)"
  else
    (
      cd "${ROUTER_ROOT}"
      cargo build --quiet
      cargo run --quiet -- "${ROUTER_CONFIG}"
    ) >"${ROUTER_LOG}" 2>&1 &
    ROUTER_PID=$!
    echo "Router pid=${ROUTER_PID} log=${ROUTER_LOG}"
    echo "Health: curl ${ROUTER_URL}/health"

    router_ready=0
    for _ in $(seq 1 40); do
      if ! kill -0 "${ROUTER_PID}" 2>/dev/null; then
        echo "sparkl-router exited early; see ${ROUTER_LOG}" >&2
        tail -n 40 "${ROUTER_LOG}" >&2 || true
        exit 1
      fi
      if curl -sf "${ROUTER_URL}/health" >/dev/null 2>&1; then
        echo "Router health check OK"
        router_ready=1
        break
      fi
      sleep 0.5
    done
    if [[ "${router_ready}" -eq 0 ]]; then
      echo "sparkl-router health check failed; see ${ROUTER_LOG}" >&2
      tail -n 40 "${ROUTER_LOG}" >&2 || true
      exit 1
    fi
  fi
else
  echo "(skipped router: --skip-router)"
fi

ROUTER_ENABLED="true"
if [[ "${SKIP_ROUTER}" -eq 1 ]]; then
  ROUTER_ENABLED="false"
fi

print_banner "Writing node config ${DEV_CONFIG}"
write_solo_launch_config "${ROUTER_ENABLED}"

if [[ "${SKIP_NODE}" -eq 0 ]]; then
  print_banner "Starting sparkl-solo node"
  (
    cd "${ROOT}"
    cargo build --features mock-tpm --quiet
    cargo run --features mock-tpm -- --config "${DEV_CONFIG}"
  ) >"${NODE_LOG}" 2>&1 &
  NODE_PID=$!
  echo "Node pid=${NODE_PID} log=${NODE_LOG}"
  echo "Health: curl http://127.0.0.1:${SOLO_INFERENCE_PORT}/health"
  echo "Status: curl http://127.0.0.1:${SOLO_INFERENCE_PORT}/status"

  for _ in $(seq 1 40); do
    if ! kill -0 "${NODE_PID}" 2>/dev/null; then
      echo "sparkl-solo exited early; see ${NODE_LOG}" >&2
      tail -n 40 "${NODE_LOG}" >&2 || true
      exit 1
    fi
    if curl -sf "http://127.0.0.1:${SOLO_INFERENCE_PORT}/health" >/dev/null 2>&1; then
      echo "Node health check OK"
      break
    fi
    sleep 0.5
  done

  if [[ "${SKIP_ROUTER}" -eq 0 ]]; then
    echo ""
    echo "Router WSS subscription requires commercial registration on portal /node/register"
    echo "  (router chain.enabled = true — unregistered nodes are rejected at connect)."
    echo "  Node identity: curl http://127.0.0.1:${SOLO_INFERENCE_PORT}/identity"
    echo "  Use node_id (0x + 64 hex) when registering; operator EOA e.g. ${ANVIL_DEPLOYER_ADDR}"
  fi
else
  echo "(skipped node: --skip-node)"
fi

if [[ "${SKIP_ROUTER}" -eq 0 ]]; then
  print_banner "sparkl-router"
  cat <<EOF

Health:  curl ${ROUTER_URL}/health
Models:  curl ${ROUTER_URL}/v1/models   (after portal registerNode + WSS tunnel online)
Status:  curl -H "Authorization: Bearer ${ROUTER_ADMIN_TOKEN}" ${ROUTER_URL}/status/nodes
Config:  ${ROUTER_CONFIG}
Log:     ${ROUTER_LOG}
EOF
fi

print_banner "sparkl-portal .env (assethub-dev-stub)"
cat <<EOF

Copy into sparkl-portal/.env or .env.local (restart yarn dev after changing NEXT_PUBLIC_*):

NEXT_PUBLIC_CHAIN_ENV=assethub-dev-stub
NEXT_PUBLIC_RPC_URL_ASSHUB_DEV_STUB=${ANVIL_RPC}
NEXT_PUBLIC_CHAIN_ID_ASSHUB_DEV_STUB=${CHAIN_ID}
NEXT_PUBLIC_OPERATOR_REGISTRY_ADDRESS_ASSHUB_DEV_STUB=${REGISTRY}
NEXT_PUBLIC_SETTLEMENT_ESCROW_ADDRESS_ASSHUB_DEV_STUB=${ESCROW}
NEXT_PUBLIC_MODEL_PRICE_ORACLE_ADDRESS_ASSHUB_DEV_STUB=${MODEL_PRICE_ORACLE}
NEXT_PUBLIC_SPARKL_ROUTER_URL=${ROUTER_URL}
SPARKL_ROUTER_URL=${ROUTER_URL}
SPARKL_ROUTER_ADMIN_TOKEN=${ROUTER_ADMIN_TOKEN}

# Remote browser on another device: open http://<LAN-IP>:3000 and use Switch network (proxy → /api/rpc).
# Run portal with: cd sparkl-portal && yarn dev:lan
# Keep RPC_PROXY_TARGET on loopback (Next and Anvil on same host):
# NEXT_PUBLIC_RPC_USE_SAME_ORIGIN_PROXY=1
# RPC_PROXY_TARGET=http://127.0.0.1:8545
# Direct wallet RPC to Anvil (no proxy): NEXT_PUBLIC_RPC_URL_ASSHUB_DEV_STUB=http://<LAN-IP>:8545

# WalletConnect (set your own project id):
# NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID=

Deployments JSON: ${DEPLOY_JSON}
EOF

print_banner "sparkl-oracle-rates configuration"
ORACLE_PK="$(oracle_signer_private_key "${ORACLE_UPDATER}")"
cat <<EOF

Copy into sparkl-oracle-rates/.env (or export in your shell):

EVM_RPC_URL=${ANVIL_RPC}
RATE_SETTER_ADDRESS=${RATE_SETTER}
ORACLE_PRIVATE_KEY=${ORACLE_PK}

UPDATE_INTERVAL_MS=300000
DEVIATION_THRESHOLD=0.005
MAX_STALENESS_MS=3600000
PRICE_SOURCES=coingecko,binance
HEALTH_PORT=8090

Oracle wallet must match RateSetter.updater on-chain:
  expected updater: ${ORACLE_UPDATER}
  key address:      $(cast wallet address --private-key "${ORACLE_PK}" 2>/dev/null || echo "(cast failed)")

Fund is not required on Anvil (accounts are pre-funded). On testnet, fund the oracle
address with native DOT for gas before yarn start.

Verify updater:
  cast call ${RATE_SETTER} "updater()(address)" --rpc-url ${ANVIL_RPC}

Start oracle service:
  cd ${ROOT}/../sparkl-oracle-rates && cp .env.example .env
  # edit .env with the values above, then: yarn install && yarn start

Optional sparkl-solo EVM integration (rebuild with evm-settlement, set wallet keys):
  registry.enabled = true
  settlement.enabled = true
  settlement.evm_provider_wallet_private_key = <operator key matching nodeOperator>
  sparkl_network_config_address = ${NET_CFG}

Deployments JSON: ${DEPLOY_JSON}
EOF

if [[ "${SKIP_NODE}" -eq 0 ]]; then
  echo ""
  router_note=""
  if [[ "${SKIP_ROUTER}" -eq 0 ]]; then
    if [[ "${REUSED_ROUTER}" -eq 1 ]]; then
      router_note=" (sparkl-router on ${ROUTER_URL} was already running and is left running)"
    else
      router_note=" and sparkl-router"
    fi
  fi
  if [[ "${REUSED_ANVIL}" -eq 1 ]]; then
    echo "Press Ctrl+C to stop the solo node${router_note} (Anvil on ${ANVIL_RPC} was already running and is left running)."
  elif [[ "${KEEP_ANVIL}" -eq 1 ]]; then
    echo "Press Ctrl+C to stop the solo node${router_note} (Anvil left running: --keep-anvil or --skip-node)."
  elif [[ "${ANVIL_USE_STATE}" -eq 0 ]]; then
    if [[ "${SKIP_ROUTER}" -eq 0 && "${REUSED_ROUTER}" -eq 0 ]]; then
      echo "Press Ctrl+C to stop the solo node, sparkl-router, and Anvil (ephemeral: --no-state)."
    else
      echo "Press Ctrl+C to stop the solo node and Anvil (ephemeral: --no-state)${router_note}."
    fi
  else
    if [[ "${SKIP_ROUTER}" -eq 0 && "${REUSED_ROUTER}" -eq 0 ]]; then
      echo "Press Ctrl+C to stop the solo node, sparkl-router, and Anvil (state saved to ${ANVIL_STATE})."
    else
      echo "Press Ctrl+C to stop the solo node and Anvil (state saved to ${ANVIL_STATE})${router_note}."
    fi
  fi
  wait "${NODE_PID}" || true
fi
