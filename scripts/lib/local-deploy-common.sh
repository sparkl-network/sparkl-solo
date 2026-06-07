# Shared helpers for local Anvil deploy + env sync (sourced by deploy-local-sync-env.sh).

local_deploy_json_field() {
  jq -r "$1" "${DEPLOY_JSON}"
}

local_deploy_wait_for_rpc() {
  local tries=40
  while (( tries > 0 )); do
    if cast chain-id --rpc-url "${ANVIL_RPC}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
    tries=$((tries - 1))
  done
  echo "Anvil RPC not ready at ${ANVIL_RPC}" >&2
  return 1
}

local_deploy_cast_tuple_field() {
  local line="$1"
  shift
  cast call "$@" --rpc-url "${ANVIL_RPC}" 2>/dev/null | awk -v n="${line}" 'NR == n { print $1; exit }'
}

local_deploy_oracle_signer_private_key() {
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

local_deploy_dot_per_usdc_from_usdc_per_dot() {
  local usdc_per_dot="$1"
  python3 -c "print(10**24 // int('${usdc_per_dot}'))"
}

local_deploy_model_price_internal_from_usd_per_1k_micro() {
  local usd_micro="$1"
  local dot_per_usdc="$2"
  python3 -c "print(int('${usd_micro}') * int('${dot_per_usdc}') // 1_000_000)"
}

local_deploy_seed_model_default_price_if_needed() {
  local model_oracle="$1"
  local rate_setter="$2"
  local updated_at dot_per_usdc usdc_per_dot input_default output_default
  local updater_pk updater

  updated_at="$(local_deploy_cast_tuple_field 3 "${model_oracle}" "defaultPrice()(uint256,uint256,uint64)" || echo "0")"
  if [[ "${updated_at}" != "0" && -n "${updated_at}" ]]; then
    echo "ModelPriceOracle defaultPrice already set (updatedAt=${updated_at})"
    return 0
  fi

  usdc_per_dot="${DEFAULT_USDC_PER_DOT}"
  local rate_updated
  rate_updated="$(local_deploy_cast_tuple_field 1 "${rate_setter}" "priceUpdatedAt()(uint256)" || echo "0")"
  if [[ -n "${rate_updated}" && "${rate_updated}" != "0" ]]; then
    usdc_per_dot="$(local_deploy_cast_tuple_field 1 "${rate_setter}" "getUsdcPerDot()(uint256)" || echo "${DEFAULT_USDC_PER_DOT}")"
  fi

  dot_per_usdc="$(local_deploy_dot_per_usdc_from_usdc_per_dot "${usdc_per_dot}")"
  input_default="$(local_deploy_model_price_internal_from_usd_per_1k_micro 100 "${dot_per_usdc}")"
  output_default="$(local_deploy_model_price_internal_from_usd_per_1k_micro 500 "${dot_per_usdc}")"

  updater="$(local_deploy_cast_tuple_field 1 "${model_oracle}" "updater()(address)" || true)"
  updater_pk="$(local_deploy_oracle_signer_private_key "${updater}")" || return 1

  echo "Seeding ModelPriceOracle defaultPrice (one-time, local MVP)..."
  cast send "${model_oracle}" "setDefaultPrice(uint256,uint256)" \
    "${input_default}" "${output_default}" \
    --rpc-url "${ANVIL_RPC}" \
    --private-key "${updater_pk}" \
    --quiet
  echo "  inputPer1k=${input_default} outputPer1k=${output_default}"
}

# Upsert KEY=value in a dotenv file (preserves comments and unrelated keys).
local_deploy_upsert_env_kv() {
  local file="$1" key="$2" value="$3"
  python3 - "$file" "$key" "$value" <<'PY'
import pathlib, re, sys

path = pathlib.Path(sys.argv[1])
key, value = sys.argv[2], sys.argv[3]
line = f"{key}={value}\n"
if path.exists():
    text = path.read_text()
    if not text.endswith("\n"):
        text += "\n"
else:
    text = ""
pattern = re.compile(rf"^{re.escape(key)}=.*$", re.MULTILINE)
if pattern.search(text):
    text = pattern.sub(f"{key}={value}", text)
else:
    if text and not text.endswith("\n"):
        text += "\n"
    text += line
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(text)
PY
}

# Replace a top-level TOML key (key = "value" or key = 123).
local_deploy_patch_toml_kv() {
  local file="$1" key="$2" value="$3"
  [[ -f "${file}" ]] || return 0
  python3 - "$file" "$key" "$value" <<'PY'
import pathlib, re, sys

path = pathlib.Path(sys.argv[1])
key, value = sys.argv[2], sys.argv[3]
text = path.read_text()
if value.isdigit():
    replacement = f'{key} = {value}'
else:
    replacement = f'{key} = "{value}"'
text = re.sub(rf"^{re.escape(key)}\s*=\s*.*$", replacement, text, flags=re.MULTILINE)
path.write_text(text)
PY
}

local_deploy_write_router_toml() {
  local file="$1"
  mkdir -p "$(dirname "${file}")"
  if [[ ! -f "${file}" ]]; then
    cat >"${file}" <<EOF
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

[settlement]
data_dir = "data"
enabled = true
# record_usage_private_key generated at router startup into data_dir/record-usage-key.json
registry_owner_private_key = "${ANVIL_DEPLOYER_PK}"
record_usage_enabled = true
record_usage_token_chunk = 10000
record_usage_flush_interval_secs = 60
enforce_session_budget = true
EOF
    return 0
  fi
  local_deploy_patch_toml_kv "${file}" rpc_url "${ANVIL_RPC}"
  local_deploy_patch_toml_kv "${file}" registry_contract "${REGISTRY}"
  local_deploy_patch_toml_kv "${file}" escrow_contract "${ESCROW}"
  local_deploy_patch_toml_kv "${file}" admin_token "${ROUTER_ADMIN_TOKEN}"
}

local_deploy_sync_component_envs() {
  local oracle_pk
  oracle_pk="$(local_deploy_oracle_signer_private_key "${ORACLE_UPDATER}")" || return 1

  echo ""
  echo "Updating component env files (from ${DEPLOY_JSON})..."

  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_CHAIN_ENV assethub-dev-stub
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_RPC_URL_ASSHUB_DEV_STUB "${ANVIL_RPC}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_CHAIN_ID_ASSHUB_DEV_STUB "${CHAIN_ID}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_OPERATOR_REGISTRY_ADDRESS_ASSHUB_DEV_STUB "${REGISTRY}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_SETTLEMENT_ESCROW_ADDRESS_ASSHUB_DEV_STUB "${ESCROW}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_MODEL_PRICE_ORACLE_ADDRESS_ASSHUB_DEV_STUB "${MODEL_PRICE_ORACLE}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    NEXT_PUBLIC_SPARKL_ROUTER_URL "${ROUTER_URL}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    SPARKL_ROUTER_URL "${ROUTER_URL}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    SPARKL_ROUTER_ADMIN_TOKEN "${ROUTER_ADMIN_TOKEN}"
  local_deploy_upsert_env_kv "${PORTAL_ROOT}/.env.local" \
    RPC_PROXY_TARGET "${ANVIL_RPC_LOCAL}"
  echo "  portal: ${PORTAL_ROOT}/.env.local"

  local_deploy_upsert_env_kv "${ORACLE_RATES_ROOT}/.env.local" \
    EVM_RPC_URL "${ANVIL_RPC}"
  local_deploy_upsert_env_kv "${ORACLE_RATES_ROOT}/.env.local" \
    RATE_SETTER_ADDRESS "${RATE_SETTER}"
  local_deploy_upsert_env_kv "${ORACLE_RATES_ROOT}/.env.local" \
    ORACLE_PRIVATE_KEY "${oracle_pk}"
  echo "  oracle-rates: ${ORACLE_RATES_ROOT}/.env.local"

  local_deploy_upsert_env_kv "${ORACLE_MODEL_ROOT}/.env.local" \
    EVM_RPC_URL "${ANVIL_RPC}"
  local_deploy_upsert_env_kv "${ORACLE_MODEL_ROOT}/.env.local" \
    RATE_SETTER_ADDRESS "${RATE_SETTER}"
  local_deploy_upsert_env_kv "${ORACLE_MODEL_ROOT}/.env.local" \
    MODEL_PRICE_ORACLE_ADDRESS "${MODEL_PRICE_ORACLE}"
  local_deploy_upsert_env_kv "${ORACLE_MODEL_ROOT}/.env.local" \
    ORACLE_PRIVATE_KEY "${oracle_pk}"
  echo "  oracle-model-price: ${ORACLE_MODEL_ROOT}/.env.local"

  local_deploy_write_router_toml "${SOLO_ROOT}/dev-config/router-launch.toml"
  echo "  solo router config: ${SOLO_ROOT}/dev-config/router-launch.toml"

  if [[ -f "${ROUTER_ROOT}/config.toml" ]]; then
    local_deploy_patch_toml_kv "${ROUTER_ROOT}/config.toml" rpc_url "${ANVIL_RPC}"
    local_deploy_patch_toml_kv "${ROUTER_ROOT}/config.toml" registry_contract "${REGISTRY}"
    local_deploy_patch_toml_kv "${ROUTER_ROOT}/config.toml" escrow_contract "${ESCROW}"
    local_deploy_patch_toml_kv "${ROUTER_ROOT}/config.toml" admin_token "${ROUTER_ADMIN_TOKEN}"
    echo "  router: ${ROUTER_ROOT}/config.toml"
  fi

  if [[ -f "${SOLO_ROOT}/dev-config/launch.toml" ]]; then
    local_deploy_patch_toml_kv "${SOLO_ROOT}/dev-config/launch.toml" \
      registry_contract_address "${REGISTRY}"
    local_deploy_patch_toml_kv "${SOLO_ROOT}/dev-config/launch.toml" \
      evm_rpc_url "${ANVIL_RPC}"
    local_deploy_patch_toml_kv "${SOLO_ROOT}/dev-config/launch.toml" \
      escrow_contract "${ESCROW}"
    local_deploy_patch_toml_kv "${SOLO_ROOT}/dev-config/launch.toml" \
      sparkl_network_config_address "${NET_CFG}"
    echo "  solo node config: ${SOLO_ROOT}/dev-config/launch.toml"
  fi
}
