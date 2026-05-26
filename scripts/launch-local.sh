#!/usr/bin/env bash
# Local dev stack: Anvil + contract deploy + sparkl-solo tests + solo node.
# Prints env hints for sparkl-portal and sparkl-oracle-rates (DOT/USD).
#
# Usage:
#   ./scripts/launch-local.sh              # full stack; node runs until Ctrl+C
#   ./scripts/launch-local.sh --skip-node  # deploy + tests; leaves Anvil running
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
# Requires: forge, anvil, cast, cargo, jq

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTRACTS="${ROOT}/contracts"
DEPLOY_JSON="${CONTRACTS}/deployments/local.json"
DEPLOY_FINGERPRINT="${ROOT}/.launch/deploy-fingerprint"
DEV_CONFIG="${ROOT}/dev-config/launch.toml"
ANVIL_HOST="${ANVIL_HOST:-0.0.0.0}"
ANVIL_PORT="${ANVIL_PORT:-8545}"
# Local forge/cast/solo always dial loopback; --host only controls who can connect in.
ANVIL_RPC_LOCAL="http://127.0.0.1:${ANVIL_PORT}"
ANVIL_RPC="${ANVIL_RPC:-${ANVIL_RPC_LOCAL}}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
ANVIL_STATE="${ROOT}/.launch/anvil-state.json"
ANVIL_LOG="${ROOT}/.launch/anvil.log"
NODE_LOG="${ROOT}/.launch/sparkl-solo.log"
PID_DIR="${ROOT}/.launch"

# MVP flat model default: 10¢ / 50¢ per 1M tokens (USD per 1k micro-units 100 / 500)
DEFAULT_USDC_PER_DOT="${ORACLE_USDC_PER_DOT:-1340000}"

# Anvil well-known keys (public; local only)
ANVIL_DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
# Anvil mnemonic index 1 (see: cast wallet private-key --mnemonic "test test ... junk" --mnemonic-index 1)
ANVIL_ORACLE_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
ANVIL_DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
ANVIL_ORACLE_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

SKIP_TESTS=0
SKIP_NODE=0
KEEP_ANVIL=0
ANVIL_USE_STATE=1
FORCE_DEPLOY=0
SKIP_DEPLOY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-tests) SKIP_TESTS=1 ;;
    --skip-node) SKIP_NODE=1; KEEP_ANVIL=1 ;;
    --keep-anvil) KEEP_ANVIL=1 ;;
    --no-state) ANVIL_USE_STATE=0 ;;
    --force-deploy) FORCE_DEPLOY=1 ;;
    --skip-deploy) SKIP_DEPLOY=1 ;;
    -h|--help)
      sed -n '2,14p' "$0"
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
REUSED_ANVIL=0

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

# DeployLocal artifact bytecode fingerprint (runtime deployedBytecode).
# Fingerprint excludes SettlementEscrow (runtime bytecode embeds deployed oracle addresses).
DEPLOY_ARTIFACTS=(
  "RateSetter.sol/RateSetter.json"
  "ModelPriceOracle.sol/ModelPriceOracle.json"
  "ProviderRegistry.sol/ProviderRegistry.json"
  "SparklNetworkConfig.sol/SparklNetworkConfig.json"
  "MockERC20.sol/MockERC20.json"
)

compute_artifact_fingerprint() {
  local combined=""
  local path rel
  for rel in "${DEPLOY_ARTIFACTS[@]}"; do
    path="${CONTRACTS}/out/${rel}"
    if [[ ! -f "${path}" ]]; then
      echo ""
      return 1
    fi
    combined+="$(jq -r '.deployedBytecode.object // empty' "${path}")"
  done
  if [[ -z "${combined}" ]]; then
    echo ""
    return 1
  fi
  printf '%s' "${combined}" | md5sum | awk '{print $1}'
}

artifact_runtime_bytecode() {
  local rel="$1"
  local path="${CONTRACTS}/out/${rel}"
  jq -r '.deployedBytecode.object // empty' "${path}" | tr 'A-F' 'a-f'
}

normalize_bytecode_hex() {
  local raw="${1#0x}"
  raw="${raw,,}"
  if [[ -z "${raw}" || "${raw}" == "0" ]]; then
    echo ""
    return 0
  fi
  if (( ${#raw} % 2 != 0 )); then
    raw="0${raw}"
  fi
  printf '0x%s' "${raw}"
}

on_chain_runtime_bytecode() {
  local addr="$1"
  cast code "${addr}" --rpc-url "${ANVIL_RPC}" 2>/dev/null | tr 'A-F' 'a-f'
}

bytecode_matches_artifact() {
  local addr="$1"
  local rel="$2"
  local expected actual
  expected="$(artifact_runtime_bytecode "${rel}")"
  if [[ -z "${expected}" ]]; then
    return 1
  fi
  actual="$(on_chain_runtime_bytecode "${addr}")"
  expected="$(normalize_bytecode_hex "${expected}")"
  actual="$(normalize_bytecode_hex "${actual}")"
  [[ -n "${actual}" && "${expected}" == "${actual}" ]]
}

address_has_code() {
  local addr="$1"
  local code
  code="$(on_chain_runtime_bytecode "${addr}")"
  [[ -n "${code}" && "${code}" != "0x" ]]
}

verify_deployments_on_chain() {
  local rate_setter model_oracle registry escrow net_cfg mock_usdc oracle_updater
  if [[ ! -f "${DEPLOY_JSON}" ]]; then
    return 1
  fi
  rate_setter="$(json_field '.rateSetter // .priceOracle')"
  model_oracle="$(json_field '.modelPriceOracle // empty')"
  registry="$(json_field '.providerRegistry')"
  escrow="$(json_field '.settlementEscrow')"
  net_cfg="$(json_field '.sparklNetworkConfig')"
  mock_usdc="$(json_field '.mockUsdc')"
  oracle_updater="$(json_field '.oracleUpdater')"

  for addr in "${rate_setter}" "${model_oracle}" "${registry}" "${escrow}" "${net_cfg}" "${mock_usdc}"; do
    if [[ -z "${addr}" || "${addr}" == "null" ]]; then
      return 1
    fi
    if ! address_has_code "${addr}"; then
      return 1
    fi
  done

  bytecode_matches_artifact "${rate_setter}" "RateSetter.sol/RateSetter.json" || return 1
  bytecode_matches_artifact "${model_oracle}" "ModelPriceOracle.sol/ModelPriceOracle.json" || return 1
  bytecode_matches_artifact "${registry}" "ProviderRegistry.sol/ProviderRegistry.json" || return 1
  bytecode_matches_artifact "${net_cfg}" "SparklNetworkConfig.sol/SparklNetworkConfig.json" || return 1
  bytecode_matches_artifact "${mock_usdc}" "MockERC20.sol/MockERC20.json" || return 1

  local on_registry on_rate on_model on_usdc
  on_registry="$(cast call "${escrow}" "registry()(address)" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr 'A-Z' 'a-f')"
  on_rate="$(cast call "${escrow}" "priceOracle()(address)" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr 'A-Z' 'a-f')"
  on_model="$(cast call "${escrow}" "modelPriceOracle()(address)" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr 'A-Z' 'a-f')"
  on_usdc="$(cast call "${escrow}" "usdc()(address)" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr 'A-Z' 'a-f')"
  registry="${registry,,}"
  rate_setter="${rate_setter,,}"
  model_oracle="${model_oracle,,}"
  mock_usdc="${mock_usdc,,}"
  [[ "${on_registry}" == "${registry}" && "${on_rate}" == "${rate_setter}" && "${on_model}" == "${model_oracle}" && "${on_usdc}" == "${mock_usdc}" ]] || return 1

  local on_chain_updater
  on_chain_updater="$(cast call "${rate_setter}" "updater()(address)" --rpc-url "${ANVIL_RPC}" 2>/dev/null || true)"
  on_chain_updater="${on_chain_updater,,}"
  oracle_updater="${oracle_updater,,}"
  [[ "${on_chain_updater}" == "${oracle_updater}" ]]
}

should_skip_deploy() {
  local current stored
  if [[ "${FORCE_DEPLOY}" -eq 1 ]]; then
    return 1
  fi
  if [[ "${SKIP_DEPLOY}" -eq 1 ]]; then
    if verify_deployments_on_chain; then
      return 0
    fi
    echo "ERROR: --skip-deploy requested but on-chain deployments are missing or stale." >&2
    echo "  Run without --skip-deploy, or use --force-deploy after contract changes." >&2
    exit 1
  fi
  if [[ ! -f "${DEPLOY_JSON}" ]] || [[ ! -f "${DEPLOY_FINGERPRINT}" ]]; then
    return 1
  fi
  current="$(compute_artifact_fingerprint)" || return 1
  stored="$(grep -E '^fingerprint=' "${DEPLOY_FINGERPRINT}" 2>/dev/null | cut -d= -f2- || true)"
  if [[ -z "${current}" || "${current}" != "${stored}" ]]; then
    if [[ -n "${stored}" && "${current}" != "${stored}" ]]; then
      echo "Contract artifacts changed (fingerprint ${stored} -> ${current}); redeploying." >&2
    fi
    return 1
  fi
  if verify_deployments_on_chain; then
    return 0
  fi
  echo "Saved deployments do not match chain bytecode; redeploying." >&2
  return 1
}

write_deploy_fingerprint() {
  local fp
  fp="$(compute_artifact_fingerprint)" || return 1
  mkdir -p "${PID_DIR}"
  cat >"${DEPLOY_FINGERPRINT}" <<EOF
fingerprint=${fp}
chain_id=${CHAIN_ID:-${ANVIL_CHAIN_ID}}
deployments=${DEPLOY_JSON}
EOF
}

dot_per_usdc_from_usdc_per_dot() {
  local usdc_per_dot="$1"
  python3 -c "print(10**24 // int('${usdc_per_dot}'))"
}

model_price_internal_from_usd_per_1k_micro() {
  local usd_micro="$1"
  local dot_per_usdc="$2"
  python3 -c "print(int('${usd_micro}') * int('${dot_per_usdc}') // 1_000_000)"
}

seed_model_default_price_if_needed() {
  local model_oracle="$1"
  local rate_setter="$2"
  local updated_at dot_per_usdc usdc_per_dot input_default output_default

  updated_at="$(cast call "${model_oracle}" "defaultPrice()(uint256,uint256,uint64)" \
    --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $3}' || echo "0")"
  if [[ "${updated_at}" != "0" && -n "${updated_at}" ]]; then
    echo "ModelPriceOracle defaultPrice already set (updatedAt=${updated_at})"
    return 0
  fi

  usdc_per_dot="${DEFAULT_USDC_PER_DOT}"
  local rate_updated
  rate_updated="$(cast call "${rate_setter}" "priceUpdatedAt()(uint256)" \
    --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr -d '[],' || echo "0")"
  if [[ -n "${rate_updated}" && "${rate_updated}" != "0" ]]; then
    usdc_per_dot="$(cast call "${rate_setter}" "getUsdcPerDot()(uint256)" \
      --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk '{print $1}' | tr -d '[],' || echo "${DEFAULT_USDC_PER_DOT}")"
  fi

  dot_per_usdc="$(dot_per_usdc_from_usdc_per_dot "${usdc_per_dot}")"
  input_default="$(model_price_internal_from_usd_per_1k_micro 100 "${dot_per_usdc}")"
  output_default="$(model_price_internal_from_usd_per_1k_micro 500 "${dot_per_usdc}")"

  echo "Seeding ModelPriceOracle defaultPrice (one-time, local MVP)..."
  cast send "${model_oracle}" "setDefaultPrice(uint256,uint256)" \
    "${input_default}" "${output_default}" \
    --rpc-url "${ANVIL_RPC}" \
    --private-key "${ORACLE_PRIVATE_KEY:-${ANVIL_ORACLE_PK}}" \
    --quiet
  echo "  inputPer1k=${input_default} outputPer1k=${output_default}"
}

require_cmd forge
require_cmd anvil
require_cmd cast
require_cmd cargo
require_cmd jq
require_cmd python3

# Best-effort LAN IPv4 for printed remote URLs (portal / wallet on another device).
lan_ipv4() {
  if command -v ip >/dev/null 2>&1; then
    ip -4 route get 1.1.1.1 2>/dev/null \
      | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}'
    return 0
  fi
  if command -v hostname >/dev/null 2>&1; then
    hostname -I 2>/dev/null | awk '{print $1}'
    return 0
  fi
  return 1
}

mkdir -p "${PID_DIR}" "${ROOT}/dev-config" "${ROOT}/dev-data/launch"

print_banner "Starting Anvil"
if curl -s -o /dev/null -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
  "${ANVIL_RPC}" 2>/dev/null; then
  REUSED_ANVIL=1
  echo "Reusing existing Anvil at ${ANVIL_RPC} (not started by this script)"
else
  anvil_args=(--host "${ANVIL_HOST}" --port "${ANVIL_PORT}" --chain-id "${ANVIL_CHAIN_ID}")
  if [[ "${ANVIL_USE_STATE}" -eq 1 ]]; then
    anvil_args+=(--state "${ANVIL_STATE}")
    echo "Anvil state: ${ANVIL_STATE}"
  else
    echo "Anvil: ephemeral (--no-state)"
  fi
  anvil "${anvil_args[@]}" >"${ANVIL_LOG}" 2>&1 &
  ANVIL_PID=$!
  echo "Anvil pid=${ANVIL_PID} log=${ANVIL_LOG}"
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

(
  cd "${CONTRACTS}"
  forge build --quiet
)

NETWORK_ROOT="$(cd "${ROOT}/.." && pwd)"
if [[ -x "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" ]]; then
  "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" --no-build
else
  echo "Warning: ${NETWORK_ROOT}/scripts/sync-contract-abis.sh not found; portal/solo ABIs may be stale." >&2
fi

if should_skip_deploy; then
  print_banner "Skipping deploy (contracts match fingerprint and chain)"
else
  print_banner "Deploying contracts (DeployLocal)"
  if ! (
    cd "${CONTRACTS}"
    forge script script/DeployLocal.s.sol:DeployLocal \
      --rpc-url "${ANVIL_RPC}" \
      --broadcast
  ); then
    echo "Deploy failed." >&2
    echo "  If SparklNetworkConfig CREATE2 already exists on this chain, reset local state:" >&2
    echo "    rm -f ${ANVIL_STATE} && ./scripts/launch-local.sh" >&2
    echo "  Or use --no-state for an ephemeral chain." >&2
    exit 1
  fi
fi

if [[ ! -f "${DEPLOY_JSON}" ]]; then
  echo "Expected deployments file missing: ${DEPLOY_JSON}" >&2
  exit 1
fi

RATE_SETTER="$(json_field '.rateSetter // .priceOracle')"
MODEL_PRICE_ORACLE="$(json_field '.modelPriceOracle // empty')"
REGISTRY="$(json_field '.providerRegistry')"
ESCROW="$(json_field '.settlementEscrow')"
NET_CFG="$(json_field '.sparklNetworkConfig')"
MOCK_USDC="$(json_field '.mockUsdc')"
ORACLE_UPDATER="$(json_field '.oracleUpdater')"
CHAIN_ID="$(json_field '.chainId')"

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

print_banner "sparkl-portal .env (assethub-dev-stub)"
cat <<EOF

Copy into sparkl-portal/.env or .env.local (restart yarn dev after changing NEXT_PUBLIC_*):

NEXT_PUBLIC_CHAIN_ENV=assethub-dev-stub
NEXT_PUBLIC_RPC_URL_ASSHUB_DEV_STUB=${ANVIL_RPC}
NEXT_PUBLIC_CHAIN_ID_ASSHUB_DEV_STUB=${CHAIN_ID}
NEXT_PUBLIC_OPERATOR_REGISTRY_ADDRESS_ASSHUB_DEV_STUB=${REGISTRY}
NEXT_PUBLIC_SETTLEMENT_ESCROW_ADDRESS_ASSHUB_DEV_STUB=${ESCROW}
NEXT_PUBLIC_MODEL_PRICE_ORACLE_ADDRESS_ASSHUB_DEV_STUB=${MODEL_PRICE_ORACLE}

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
  if [[ "${REUSED_ANVIL}" -eq 1 ]]; then
    echo "Press Ctrl+C to stop the solo node (Anvil on ${ANVIL_RPC} was already running and is left running)."
  elif [[ "${KEEP_ANVIL}" -eq 1 ]]; then
    echo "Press Ctrl+C to stop the solo node (Anvil left running: --keep-anvil or --skip-node)."
  elif [[ "${ANVIL_USE_STATE}" -eq 0 ]]; then
    echo "Press Ctrl+C to stop the solo node and Anvil (ephemeral: --no-state)."
  else
    echo "Press Ctrl+C to stop the solo node and Anvil (state saved to ${ANVIL_STATE})."
  fi
  wait "${NODE_PID}" || true
fi
