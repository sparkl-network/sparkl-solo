# DEVELOPER GUIDE

This guide covers local development for `sparkl-solo`, including running two local nodes with different config files so they can discover each other.

## Prerequisites

- Rust toolchain (stable)
- From this directory: `node1/`

## Build and test

- Build with mock TPM mode:
  - `cargo build --features mock-tpm`
- Run tests:
  - `cargo test --features mock-tpm`

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

## Notes

- `mock-tpm` is required for laptop/dev workflows.
- Registry and settlement are disabled in these local configs.
- If using `llama-swap` on `:8000`, set `backend.url = "http://127.0.0.1:8000"`.
