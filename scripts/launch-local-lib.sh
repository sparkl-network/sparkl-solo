# Shared helpers for launch-local.sh and launch-grid.sh (source only).
[[ -n "${LAUNCH_LOCAL_LIB_LOADED:-}" ]] && return 0
LAUNCH_LOCAL_LIB_LOADED=1

launch_local_init_paths() {
  local root="${1:?root required}"
  ROOT="${root}"
  CONTRACTS="${ROOT}/contracts"
  DEPLOY_JSON="${CONTRACTS}/deployments/local.json"
  DEPLOY_FINGERPRINT="${ROOT}/.launch/deploy-fingerprint"
  DEV_CONFIG="${ROOT}/dev-config/launch.toml"
  ANVIL_HOST="${ANVIL_HOST:-0.0.0.0}"
  ANVIL_PORT="${ANVIL_PORT:-8545}"
  ANVIL_RPC_LOCAL="http://127.0.0.1:${ANVIL_PORT}"
  ANVIL_RPC="${ANVIL_RPC:-${ANVIL_RPC_LOCAL}}"
  ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
  ANVIL_STATE="${ROOT}/.launch/anvil-state.json"
  ANVIL_LOG="${ROOT}/.launch/anvil.log"
  NODE_LOG="${ROOT}/.launch/sparkl-solo.log"
  ROUTER_CONFIG="${ROOT}/dev-config/router-launch.toml"
  ROUTER_LOG="${ROOT}/.launch/sparkl-router.log"
  ROUTER_HOST="${ROUTER_HOST:-127.0.0.1}"
  ROUTER_PORT="${ROUTER_PORT:-3001}"
  ROUTER_BIND="${ROUTER_BIND:-${ROUTER_HOST}:${ROUTER_PORT}}"
  ROUTER_URL="${ROUTER_URL:-http://${ROUTER_HOST}:${ROUTER_PORT}}"
  ROUTER_WS_URL="${ROUTER_WS_URL:-ws://${ROUTER_HOST}:${ROUTER_PORT}/node/connect}"
  ROUTER_ADMIN_TOKEN="${ROUTER_ADMIN_TOKEN:-dev-admin-token-change-me}"
  PID_DIR="${ROOT}/.launch"
  GRID_DIR="${PID_DIR}/grid"
  GRID_ENV="${GRID_DIR}/grid-env.sh"
  TMUX_SESSION="${TMUX_GRID_SESSION:-sparkl-grid}"
  SOLO_INFERENCE_PORT="${SOLO_INFERENCE_PORT:-19950}"
  PORTAL_DEV_PORT="${PORTAL_DEV_PORT:-3000}"
  DEFAULT_USDC_PER_DOT="${ORACLE_USDC_PER_DOT:-1340000}"
  ANVIL_DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
  ANVIL_ORACLE_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
  ANVIL_DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
  ANVIL_ORACLE_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
  NETWORK_ROOT="$(cd "${ROOT}/.." && pwd)"
  ROUTER_ROOT="${NETWORK_ROOT}/sparkl-router"
  PORTAL_ROOT="${NETWORK_ROOT}/sparkl-portal"
  PORTAL_ENV_LOCAL="${PORTAL_ROOT}/.env.local"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

print_banner() {
  echo ""
  echo "================================================================"
  echo " $1"
  echo "================================================================"
}

wait_for_rpc() {
  local tries=80
  while (( tries > 0 )); do
    if cast chain-id --rpc-url "${ANVIL_RPC}" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${ANVIL_PID:-}" ]] && ! kill -0 "${ANVIL_PID}" 2>/dev/null; then
      echo "Anvil exited before RPC was ready; see ${ANVIL_LOG}" >&2
      tail -n 30 "${ANVIL_LOG}" >&2 || true
      exit 1
    fi
    sleep 0.25
    tries=$((tries - 1))
  done
  echo "Anvil RPC not ready at ${ANVIL_RPC}" >&2
  if [[ -f "${ANVIL_LOG}" ]]; then
    tail -n 30 "${ANVIL_LOG}" >&2 || true
  fi
  exit 1
}

wait_for_router() {
  local tries=40
  while (( tries > 0 )); do
    if curl -sf "${ROUTER_URL}/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
    tries=$((tries - 1))
  done
  echo "sparkl-router not ready at ${ROUTER_URL}/health" >&2
  exit 1
}

anvil_rpc_ready() {
  curl -s -o /dev/null -X POST -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
    "${ANVIL_RPC}" 2>/dev/null
}

json_field() {
  jq -r "$1" "${DEPLOY_JSON}"
}

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

cast_tuple_field() {
  local line="$1"
  shift
  cast call "$@" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk -v n="${line}" 'NR == n { print $1; exit }'
}

oracle_signer_private_key() {
  local updater="${1,,}"
  if [[ -n "${ORACLE_PRIVATE_KEY:-}" ]]; then
    printf '%s' "${ORACLE_PRIVATE_KEY}"
    return 0
  fi
  if [[ "${updater}" == "${ANVIL_DEPLOYER_ADDR,,}" ]]; then
    printf '%s' "${ANVIL_DEPLOYER_PK}"
    return 0
  fi
  if [[ "${updater}" == "${ANVIL_ORACLE_ADDR,,}" ]]; then
    printf '%s' "${ANVIL_ORACLE_PK}"
    return 0
  fi
  echo "ERROR: oracleUpdater ${updater} is not a known Anvil dev key." >&2
  echo "  Set ORACLE_PRIVATE_KEY to the updater wallet, or redeploy with ORACLE_UPDATER_ADDRESS=${ANVIL_ORACLE_ADDR}." >&2
  return 1
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
  local updater_pk updater

  updated_at="$(cast_tuple_field 3 "${model_oracle}" "defaultPrice()(uint256,uint256,uint64)" || echo "0")"
  if [[ "${updated_at}" != "0" && -n "${updated_at}" ]]; then
    echo "ModelPriceOracle defaultPrice already set (updatedAt=${updated_at})"
    return 0
  fi

  usdc_per_dot="${DEFAULT_USDC_PER_DOT}"
  local rate_updated
  rate_updated="$(cast_tuple_field 1 "${rate_setter}" "priceUpdatedAt()(uint256)" || echo "0")"
  if [[ -n "${rate_updated}" && "${rate_updated}" != "0" ]]; then
    usdc_per_dot="$(cast_tuple_field 1 "${rate_setter}" "getUsdcPerDot()(uint256)" || echo "${DEFAULT_USDC_PER_DOT}")"
  fi

  dot_per_usdc="$(dot_per_usdc_from_usdc_per_dot "${usdc_per_dot}")"
  input_default="$(model_price_internal_from_usd_per_1k_micro 100 "${dot_per_usdc}")"
  output_default="$(model_price_internal_from_usd_per_1k_micro 500 "${dot_per_usdc}")"

  updater="$(cast_tuple_field 1 "${model_oracle}" "updater()(address)" || true)"
  updater_pk="$(oracle_signer_private_key "${updater}")" || return 1

  echo "Seeding ModelPriceOracle defaultPrice (one-time, local MVP)..."
  cast send "${model_oracle}" "setDefaultPrice(uint256,uint256)" \
    "${input_default}" "${output_default}" \
    --rpc-url "${ANVIL_RPC}" \
    --private-key "${updater_pk}" \
    --quiet
  echo "  inputPer1k=${input_default} outputPer1k=${output_default}"
}

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

anvil_cmd_args() {
  ANVIL_CMD_ARGS=(--host "${ANVIL_HOST}" --port "${ANVIL_PORT}" --chain-id "${ANVIL_CHAIN_ID}")
  if [[ "${ANVIL_USE_STATE}" -eq 1 ]]; then
    ANVIL_CMD_ARGS+=(--state "${ANVIL_STATE}")
  fi
}

start_anvil_background() {
  anvil_cmd_args
  anvil "${ANVIL_CMD_ARGS[@]}" >"${ANVIL_LOG}" 2>&1 &
  ANVIL_PID=$!
  echo "Anvil pid=${ANVIL_PID} log=${ANVIL_LOG}"
}

stop_anvil() {
  if [[ -n "${ANVIL_PID:-}" ]] && kill -0 "${ANVIL_PID}" 2>/dev/null; then
    kill "${ANVIL_PID}" 2>/dev/null || true
    wait "${ANVIL_PID}" 2>/dev/null || true
    ANVIL_PID=""
    return 0
  fi
  kill_listeners_on_port "${ANVIL_PORT}"
}

kill_listeners_on_port() {
  local port="$1"
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "${port}/tcp" 2>/dev/null || true
    sleep 0.35
    return 0
  fi
  local pid
  pid="$(ss -ltnp "sport = :${port}" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1 || true)"
  if [[ -n "${pid}" ]]; then
    kill "${pid}" 2>/dev/null || true
    sleep 0.35
  fi
}

load_deploy_addresses() {
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
}

read_existing_backend_url() {
  local url=""
  if [[ -f "${DEV_CONFIG}" ]]; then
    url="$(grep -E '^\s*url\s*=' "${DEV_CONFIG}" 2>/dev/null | head -1 | sed -E 's/^[[:space:]]*url[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/' || true)"
  fi
  if [[ -n "${url}" ]]; then
    printf '%s' "${url}"
  else
    printf '%s' "http://127.0.0.1:11434"
  fi
}

read_existing_walletconnect_id() {
  local id=""
  if [[ -f "${PORTAL_ENV_LOCAL}" ]]; then
    id="$(grep -E '^NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID=' "${PORTAL_ENV_LOCAL}" 2>/dev/null | cut -d= -f2- | tr -d '"' || true)"
  fi
  printf '%s' "${id}"
}

write_router_launch_config() {
  cat >"${ROUTER_CONFIG}" <<EOF
[server]
bind = "${ROUTER_BIND}"
router_url = "${ROUTER_URL}"
upstream_timeout_secs = 120

[chain]
rpc_url = "${ANVIL_RPC}"
registry_contract = "${REGISTRY}"
escrow_contract = "${ESCROW}"
session_cache_ttl_secs = 12
enabled = true

[node_auth]
ping_interval_secs = 30
pong_timeout_secs = 10

[metrics]
bind = "127.0.0.1:9091"

[portal]
admin_token = "${ROUTER_ADMIN_TOKEN}"
stale_threshold_secs = 40
models_refresh_on_pong_secs = 60
EOF
}

write_solo_launch_config() {
  local router_enabled="${1:-true}"
  local backend_url
  backend_url="$(read_existing_backend_url)"
  cat >"${DEV_CONFIG}" <<EOF
[node]
moniker = "launch-grid"
data_dir = "./dev-data/launch"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50
include_models = []
exclude_models = []

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31010"]
inference_port = ${SOLO_INFERENCE_PORT}
bootstrap_peers = []
public_addr = []
expose_status_detail = true
allow_non_globals_in_dht = true

[backend]
url = "${backend_url}"
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

[router]
enabled = ${router_enabled}
url = "${ROUTER_WS_URL}"
reconnect_min_secs = 1
reconnect_max_secs = 60
local_inference_base = ""
EOF
}

write_portal_env_local() {
  local wc_id
  wc_id="$(read_existing_walletconnect_id)"
  mkdir -p "$(dirname "${PORTAL_ENV_LOCAL}")"
  cat >"${PORTAL_ENV_LOCAL}" <<EOF
# Generated by launch-grid.sh — restart yarn dev after changes
NEXT_PUBLIC_CHAIN_ENV=assethub-dev-stub
SPARKL_ROUTER_URL=${ROUTER_URL}
SPARKL_ROUTER_ADMIN_TOKEN=${ROUTER_ADMIN_TOKEN}
EOF
  if [[ -n "${wc_id}" ]]; then
    echo "NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID=${wc_id}" >>"${PORTAL_ENV_LOCAL}"
  else
    echo "# NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID=" >>"${PORTAL_ENV_LOCAL}"
  fi
  cat >>"${PORTAL_ENV_LOCAL}" <<EOF

NEXT_PUBLIC_RPC_URL_ASSHUB_DEV_STUB=${ANVIL_RPC}
NEXT_PUBLIC_CHAIN_ID_ASSHUB_DEV_STUB=${CHAIN_ID}
NEXT_PUBLIC_CHAIN_NAME_ASSHUB_DEV_STUB=Sparkl local
NEXT_PUBLIC_RPC_USE_SAME_ORIGIN_PROXY=1
RPC_PROXY_TARGET=http://127.0.0.1:${ANVIL_PORT}
NEXT_PUBLIC_OPERATOR_REGISTRY_ADDRESS_ASSHUB_DEV_STUB=${REGISTRY}
NEXT_PUBLIC_SETTLEMENT_ESCROW_ADDRESS_ASSHUB_DEV_STUB=${ESCROW}
NEXT_PUBLIC_MODEL_PRICE_ORACLE_ADDRESS_ASSHUB_DEV_STUB=${MODEL_PRICE_ORACLE}
NEXT_PUBLIC_SPARKL_ROUTER_URL=${ROUTER_URL}
EOF
}

run_forge_build() {
  (cd "${CONTRACTS}" && forge build --quiet)
}

run_abi_sync() {
  if [[ -x "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" ]]; then
    "${NETWORK_ROOT}/scripts/sync-contract-abis.sh" --no-build
  else
    echo "Warning: ${NETWORK_ROOT}/scripts/sync-contract-abis.sh not found; portal/solo ABIs may be stale." >&2
  fi
}

run_deploy_if_needed() {
  if should_skip_deploy; then
    print_banner "Skipping deploy (contracts match fingerprint and chain)"
    return 0
  fi
  print_banner "Deploying contracts (DeployLocal)"
  if ! (
    cd "${CONTRACTS}"
    forge script script/DeployLocal.s.sol:DeployLocal \
      --rpc-url "${ANVIL_RPC}" \
      --broadcast
  ); then
    echo "Deploy failed." >&2
    echo "  If SparklNetworkConfig CREATE2 already exists on this chain, reset local state:" >&2
    echo "    rm -f ${ANVIL_STATE} && ./scripts/launch-grid.sh  # from sparkl-network/" >&2
    echo "  Or use --no-state for an ephemeral chain." >&2
    exit 1
  fi
}

launch_grid_prep() {
  print_banner "Prep: stop services on dev ports"
  kill_listeners_on_port "${ROUTER_PORT}"
  kill_listeners_on_port "${SOLO_INFERENCE_PORT}"
  stop_anvil
  kill_listeners_on_port "${ANVIL_PORT}"

  print_banner "Prep: start Anvil for deploy"
  mkdir -p "${PID_DIR}" "${ROOT}/dev-config" "${ROOT}/dev-data/launch"
  start_anvil_background
  wait_for_rpc

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

  print_banner "Writing configs (router, solo, portal)"
  write_router_launch_config
  write_solo_launch_config "true"
  write_portal_env_local

  print_banner "Contract addresses"
  echo "RPC URL:              ${ANVIL_RPC}"
  echo "Chain ID:             ${CHAIN_ID}"
  echo "ProviderRegistry:     ${REGISTRY}"
  echo "SettlementEscrow:     ${ESCROW}"
  echo "ModelPriceOracle:     ${MODEL_PRICE_ORACLE}"
  echo "SparklNetworkConfig:  ${NET_CFG}"
  echo "Portal .env.local:    ${PORTAL_ENV_LOCAL}"
  echo "Solo config:          ${DEV_CONFIG}"
  echo "Router config:        ${ROUTER_CONFIG}"

  print_banner "Prep: stop Anvil (tmux pane will start fresh)"
  stop_anvil
  kill_listeners_on_port "${ANVIL_PORT}"

  if [[ ! -f "${ROUTER_ROOT}/Cargo.toml" ]]; then
    echo "ERROR: sparkl-router not found at ${ROUTER_ROOT}" >&2
    exit 1
  fi
  if [[ ! -f "${PORTAL_ROOT}/package.json" ]]; then
    echo "ERROR: sparkl-portal not found at ${PORTAL_ROOT}" >&2
    exit 1
  fi

  print_banner "Building router and solo (once)"
  (cd "${ROUTER_ROOT}" && cargo build --quiet)
  (cd "${ROOT}" && cargo build --features mock-tpm --quiet)
}

write_grid_env() {
  mkdir -p "${GRID_DIR}"
  cat >"${GRID_ENV}" <<EOF
# Generated by launch-grid.sh — sourced by tmux pane scripts
ROOT="${ROOT}"
ANVIL_RPC="${ANVIL_RPC}"
ANVIL_HOST="${ANVIL_HOST}"
ANVIL_PORT="${ANVIL_PORT}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID}"
ANVIL_STATE="${ANVIL_STATE}"
ANVIL_USE_STATE="${ANVIL_USE_STATE}"
ROUTER_ROOT="${ROUTER_ROOT}"
ROUTER_CONFIG="${ROUTER_CONFIG}"
ROUTER_URL="${ROUTER_URL}"
DEV_CONFIG="${DEV_CONFIG}"
PORTAL_ROOT="${PORTAL_ROOT}"
SOLO_INFERENCE_PORT="${SOLO_INFERENCE_PORT}"
EOF
}

write_grid_pane_scripts() {
  mkdir -p "${GRID_DIR}"
  cat >"${GRID_DIR}/pane-anvil.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/grid-env.sh"
anvil_args=(anvil --host "${ANVIL_HOST}" --port "${ANVIL_PORT}" --chain-id "${ANVIL_CHAIN_ID}")
if [[ "${ANVIL_USE_STATE}" == "1" ]]; then
  anvil_args+=(--state "${ANVIL_STATE}")
fi
echo "[anvil] ${ANVIL_HOST}:${ANVIL_PORT} (RPC ${ANVIL_RPC})"
exec "${anvil_args[@]}"
EOF

  cat >"${GRID_DIR}/pane-router.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/grid-env.sh"
echo "[router] waiting for Anvil RPC at ${ANVIL_RPC}..."
until cast chain-id --rpc-url "${ANVIL_RPC}" >/dev/null 2>&1; do sleep 0.25; done
echo "[router] starting sparkl-router (${ROUTER_CONFIG})"
cd "${ROUTER_ROOT}"
exec cargo run --quiet -- "${ROUTER_CONFIG}"
EOF

  cat >"${GRID_DIR}/pane-solo.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/grid-env.sh"
echo "[solo] waiting for router ${ROUTER_URL}/health..."
until curl -sf "${ROUTER_URL}/health" >/dev/null 2>&1; do sleep 0.5; done
echo "[solo] starting sparkl-solo (${DEV_CONFIG})"
cd "${ROOT}"
exec cargo run --features mock-tpm --quiet -- --config "${DEV_CONFIG}"
EOF

  cat >"${GRID_DIR}/pane-portal.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/grid-env.sh"
echo "[portal] waiting for router ${ROUTER_URL}/health..."
until curl -sf "${ROUTER_URL}/health" >/dev/null 2>&1; do sleep 0.5; done
echo "[portal] yarn dev in ${PORTAL_ROOT}"
cd "${PORTAL_ROOT}"
exec yarn dev
EOF

  chmod +x "${GRID_DIR}/pane-"*.sh
}

launch_grid_tmux() {
  local session="${TMUX_SESSION}"
  tmux has-session -t "${session}" 2>/dev/null && tmux kill-session -t "${session}"

  # 2x2 layout (tmux renumbers panes after each split):
  #   split -h → left | right
  #   split -v on left → top-left / bottom-left  (right stays one pane, index 2)
  #   split -v on right (pane 2) → top-right / bottom-right
  tmux new-session -d -s "${session}" -n dev -c "${ROOT}"
  tmux split-window -h -t "${session}:0"
  tmux select-pane -t "${session}:0.0"
  tmux split-window -v
  tmux select-pane -t "${session}:0.2"
  tmux split-window -v

  # pane index → quadrant: 0=TL 2=TR / 1=BL 3=BR
  tmux select-pane -t "${session}:0.0" -T anvil
  tmux select-pane -t "${session}:0.2" -T router
  tmux select-pane -t "${session}:0.1" -T solo
  tmux select-pane -t "${session}:0.3" -T portal

  tmux send-keys -t "${session}:0.0" "${GRID_DIR}/pane-anvil.sh" Enter
  sleep 0.5
  tmux send-keys -t "${session}:0.2" "${GRID_DIR}/pane-router.sh" Enter
  sleep 0.3
  tmux send-keys -t "${session}:0.1" "${GRID_DIR}/pane-solo.sh" Enter
  sleep 0.3
  tmux send-keys -t "${session}:0.3" "${GRID_DIR}/pane-portal.sh" Enter
}
