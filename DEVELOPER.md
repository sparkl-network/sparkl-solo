# DEVELOPER GUIDE

This guide covers local development for `sparkl-solo`, including running two local nodes with different config files so they can discover each other.

## Prerequisites

- Rust toolchain (stable)
- From this directory: `sparkl-solo/`

## Build and test

- Build with mock TPM mode:
  - `cargo build --features mock-tpm`
- Build with TPM feature:
  - `cargo build --features tpm`
- Run tests:
  - `cargo test --features mock-tpm`
  - `cargo test --features tpm`

## macOS TPM2 tooling note

- `swtpm` is available on macOS via Homebrew:
  - `brew install swtpm`
- `tpm2-tools` is currently not available in Homebrew core (`brew install tpm2-tools` fails with "No available formula").
- Current node behavior with `--features tpm` on macOS:
  - if `TCTI`/`TPM2TOOLS_TCTI` is set and `tpm2_getrandom` exists, identity is marked TPM-backed (`cert_type: "swtpm"`).
  - if `tpm2_getrandom` is missing, node automatically falls back to software identity (`cert_type: "mock-software"`).
- For full TPM2 CLI validation (`tpm2_getcap`, `tpm2_getrandom`, etc.), use a Linux machine/container with `tpm2-tools` installed.

## Create local config files

Create a `dev-config/` folder:

```bash
mkdir -p dev-config
```

Create `dev-config/node1.toml`:

```toml
[node]
name = "local-node-1"
data_dir = "./dev-data/node1"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31001"]
inference_port = 9944
bootstrap_peers = []

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
unicity_aggregator_url = "https://aggregator.unicity.network"
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
evm_rpc_url = "https://mainnet.base.org"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
```

Start node 1 once so it prints/logs its libp2p peer ID. You will use that in node 2 config.

Create `dev-config/node2.toml` (replace `<NODE1_PEER_ID>`):

```toml
[node]
name = "local-node-2"
data_dir = "./dev-data/node2"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50

[network]
listen_addrs = ["/ip4/127.0.0.1/tcp/31002"]
inference_port = 9945
bootstrap_peers = ["/ip4/127.0.0.1/tcp/31001/p2p/<NODE1_PEER_ID>"]

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
unicity_aggregator_url = "https://aggregator.unicity.network"
heartbeat_secs = 30
enabled = false

[settlement]
epoch_secs = 600
evm_rpc_url = "https://mainnet.base.org"
escrow_contract = "0x0000000000000000000000000000000000000000"
enabled = false

[pricing]
micro_usd_per_m_input_tokens = 100
micro_usd_per_m_output_tokens = 780
```

## Run two local nodes

In terminal 1:

```bash
cargo run --features mock-tpm -- --config dev-config/node1.toml
```

In terminal 2:

```bash
cargo run --features mock-tpm -- --config dev-config/node2.toml
```

## Verify local connectivity

- Watch logs in both terminals for:
  - new listen address events
  - connection established events
  - identify/kademlia events
- Hit health/status:
  - `curl http://127.0.0.1:9944/health`
  - `curl http://127.0.0.1:9945/health`
  - `curl http://127.0.0.1:9944/status`
  - `curl http://127.0.0.1:9945/status`

## Verify with automated test

Run the two-node integration test:

```bash
cargo test --features mock-tpm two_nodes_discover_each_other_with_separate_configs -- --nocapture
```

## Verify with JavaScript tests

Run operational checks from the `tests-js/` folder:

```bash
cd tests-js
yarn install
yarn status
yarn attestation
yarn encrypted
yarn tpm:suite
```

## Notes

- `mock-tpm` is required for laptop/dev workflows.
- Registry and settlement are disabled in these local configs.
- If using `llama-swap` on `:8000`, set `backend.url = "http://127.0.0.1:8000"`.
- libp2p peer identity is persisted under `<data_dir>/network/`:
  - `secret_ed25519` (private key, generated once if missing)
  - `peer_id` (derived PeerId text)
- Receipt cadence can be tuned per node:
  - config: `node.receipt_cadence_tokens = 50`
  - CLI override: `--receipt-cadence NUM_TOKS`
- Model exposure can be filtered per node:
  - config allow-list: `node.include_models = ["model/a", "model/b"]`
  - config block-list: `node.exclude_models = ["model/private"]`
  - CLI overrides: `--include-models model/a,model/b` and `--exclude-models model/private`
  - filtering order: include first (or all models when include is empty), then exclude
