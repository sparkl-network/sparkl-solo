#!/usr/bin/env bash
# Local dev stack: Anvil + contract deploy + sparkl-solo tests + solo node.
# Prints env hints for sparkl-oracle-rates (../sparkl-oracle-rates).
#
# Usage:
#   ./scripts/launch-local.sh              # full stack; node runs until Ctrl+C
#   ./scripts/launch-local.sh --skip-node  # deploy + tests; leaves Anvil running
#   ./scripts/launch-local.sh --skip-tests # faster iteration
#   ./scripts/launch-local.sh --keep-anvil # do not stop Anvil on exit
#
# Requires: forge, anvil, cast, cargo, jq

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTRACTS="${ROOT}/contracts"
DEPLOY_JSON="${CONTRACTS}/deployments/local.json"
DEV_CONFIG="${ROOT}/dev-config/launch.toml"
ANVIL_HOST="${ANVIL_HOST:-127.0.0.1}"
ANVIL_PORT="${ANVIL_PORT:-8545}"
ANVIL_RPC="http://${ANVIL_HOST}:${ANVIL_PORT}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
ANVIL_LOG="${ROOT}/.launch/anvil.log"
NODE_LOG="${ROOT}/.launch/sparkl-solo.log"
PID_DIR="${ROOT}/.launch"

# Anvil well-known keys (public; local only)
ANVIL_DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# Anvil mnemonic index 1 (see: cast wallet private-key --mnemonic "test test ... junk" --mnemonic-index 1)
ANVIL_ORACLE_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
ANVIL_DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
ANVIL_ORACLE_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

SKIP_TESTS=0
SKIP_NODE=0
KEEP_ANVIL=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-tests) SKIP_TESTS=1 ;;
    --skip-node) SKIP_NODE=1; KEEP_ANVIL=1 ;;
    --keep-anvil) KEEP_ANVIL=1 ;;
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

ANVIL_PID=""
NODE_PID=""

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

cleanup() {
  local code=$?
  if [[ -n "${NODE_PID}" ]] && kill -0 "${NODE_PID}" 2>/dev/null; then
    kill "${NODE_PID}" 2>/dev/null || true
  fi
  if [[ "${KEEP_ANVIL}" -eq 0 ]] && [[ -n "${ANVIL_PID}" ]] && kill -0 "${ANVIL_PID}" 2>/dev/null; then
    kill "${ANVIL_PID}" 2>/dev/null || true
  fi
  if [[ $code -ne 0 ]]; then
    echo "launch-local.sh exited with status ${code}" >&2
  fi
}
trap cleanup EXIT INT TERM

wait_for_rpc() {
  local tries=40
  while (( tries > 0 )); do
    if cast chain-id --rpc-url "${ANVIL_RPC}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
    tries=$((tries - 1))
  done
  echo "Anvil RPC not ready at ${ANVIL_RPC}" >&2
  exit 1
}

json_field() {
  jq -r "$1" "${DEPLOY_JSON}"
}

print_banner() {
  echo ""
  echo "================================================================"
  echo " $1"
  echo "================================================================"
}

require_cmd forge
require_cmd anvil
require_cmd cast
require_cmd cargo
require_cmd jq

mkdir -p "${PID_DIR}" "${ROOT}/dev-config" "${ROOT}/dev-data/launch"

print_banner "Starting Anvil"
if curl -s -o /dev/null -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
  "${ANVIL_RPC}" 2>/dev/null; then
  echo "Reusing existing Anvil at ${ANVIL_RPC}"
else
  anvil \
    --host "${ANVIL_HOST}" \
    --port "${ANVIL_PORT}" \
    --chain-id "${ANVIL_CHAIN_ID}" \
    >"${ANVIL_LOG}" 2>&1 &
  ANVIL_PID=$!
  echo "Anvil pid=${ANVIL_PID} log=${ANVIL_LOG}"
  wait_for_rpc
fi

print_banner "Deploying contracts (DeployLocal)"
export PRIVATE_KEY="${PRIVATE_KEY:-${ANVIL_DEPLOYER_PK}}"
if [[ -z "${ORACLE_UPDATER_ADDRESS:-}" ]]; then
  export ORACLE_UPDATER_ADDRESS="${ANVIL_ORACLE_ADDR}"
fi
export ORACLE_MAX_STALENESS="${ORACLE_MAX_STALENESS:-3600}"
(
  cd "${CONTRACTS}"
  forge build --quiet
  forge script script/DeployLocal.s.sol:DeployLocal \
    --rpc-url "${ANVIL_RPC}" \
    --broadcast
)

if [[ ! -f "${DEPLOY_JSON}" ]]; then
  echo "Expected deployments file missing: ${DEPLOY_JSON}" >&2
  exit 1
fi

RATE_SETTER="$(json_field '.rateSetter // .priceOracle')"
REGISTRY="$(json_field '.providerRegistry')"
ESCROW="$(json_field '.settlementEscrow')"
NET_CFG="$(json_field '.sparklNetworkConfig')"
MOCK_USDC="$(json_field '.mockUsdc')"
ORACLE_UPDATER="$(json_field '.oracleUpdater')"
CHAIN_ID="$(json_field '.chainId')"

print_banner "Contract addresses"
echo "RPC URL:              ${ANVIL_RPC}"
echo "Chain ID:             ${CHAIN_ID}"
echo "Deployer:             ${ANVIL_DEPLOYER_ADDR}"
echo "RateSetter:           ${RATE_SETTER}"
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

print_banner "Writing node config ${DEV_CONFIG}"
cat >"${DEV_CONFIG}" <<EOF
[node]
name = "launch-local"
data_dir = "./dev-data/launch"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50
include_models = []
exclude_models = []

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31010"]
inference_port = 19950
bootstrap_peers = []
public_addr = []
expose_status_detail = true
allow_non_globals_in_dht = true

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 120

[attestation]
nras_url = "https://nras.attestation.nvidia.com"
nras_enabled = false
cert_ttl_days = 7

[registry]
registry_contract_address = "${REGISTRY}"
evm_rpc_url = "${ANVIL_RPC}"
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
evm_rpc_url = "${ANVIL_RPC}"
escrow_contract = "${ESCROW}"
sparkl_network_config_address = "${NET_CFG}"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
EOF

if [[ "${SKIP_NODE}" -eq 0 ]]; then
  print_banner "Starting sparkl-solo node"
  (
    cd "${ROOT}"
    cargo build --features mock-tpm --quiet
    cargo run --features mock-tpm -- --config "${DEV_CONFIG}"
  ) >"${NODE_LOG}" 2>&1 &
  NODE_PID=$!
  echo "Node pid=${NODE_PID} log=${NODE_LOG}"
  echo "Health: curl http://127.0.0.1:19950/health"
  echo "Status: curl http://127.0.0.1:19950/status"

  # Wait until the node is reachable or the process exits.
  for _ in $(seq 1 40); do
    if ! kill -0 "${NODE_PID}" 2>/dev/null; then
      echo "sparkl-solo exited early; see ${NODE_LOG}" >&2
      tail -n 40 "${NODE_LOG}" >&2 || true
      exit 1
    fi
    if curl -sf "http://127.0.0.1:19950/health" >/dev/null 2>&1; then
      echo "Node health check OK"
      break
    fi
    sleep 0.5
  done
else
  echo "(skipped node: --skip-node)"
fi

print_banner "sparkl-oracle-rates configuration"
ORACLE_PK="${ORACLE_PRIVATE_KEY:-${ANVIL_ORACLE_PK}}"
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
  echo "Press Ctrl+C to stop Anvil and the solo node."
  wait "${NODE_PID}" || true
fi
