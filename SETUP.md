# Production Node Setup (`sparkl-solo`)

This guide explains how to run a `sparkl-solo` node in a production-like environment, including config options, network exposure, and port forwarding.

`sparkl-solo` is still a prototype. Use this as an operational baseline, then harden further for your environment.

## 1) Prerequisites

- Linux host (recommended) with a stable public IP (or static DNS + port forwarding).
- Rust toolchain and build dependencies for this repo.
- A reachable model backend (`llama-swap`, `Ollama`, or equivalent OpenAI-compatible backend).
- TPM path:
  - real hardware TPM/attestation stack for production hardware, or
  - `swtpm` for pre-production validation.
- Firewall control (cloud security group + host firewall).

## 2) Build the binary

From repo root:

```bash
cargo build --release --features tpm
```

Binary path:

```bash
./target/release/sparkl-solo
```

## 3) Create a production config

Create `config/prod.toml`:

```toml
[node]
name = "prod-node-1"
data_dir = "/var/lib/sparkl-solo/node1"
log_level = "info"
mode = "solo"
receipt_cadence_tokens = 50
include_models = []
exclude_models = []

[network]
listen_addrs = ["/ip4/0.0.0.0/udp/30333/quic-v1", "/ip4/0.0.0.0/tcp/30333"]
inference_port = 9944
bootstrap_peers = []
public_addr = [
  "/ip4/YOUR.PUBLIC.IP/tcp/30333",
  "/ip4/YOUR.PUBLIC.IP/tcp/443/ws",
  "/ip4/YOUR.PUBLIC.IP/tcp/30333/p2p/YOUR_PEER_ID"
]
allow_non_globals_in_dht = false
external_ip = "YOUR.PUBLIC.IP.ADDR"

[backend]
url = "http://127.0.0.1:11434"
health_path = "/health"
models_path = "/v1/models"
timeout_secs = 120

[attestation]
nras_url = "https://nras.attestation.nvidia.com"
nras_enabled = true
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

## 4) Config options reference

- `node.name`: operator-friendly node name for logs/ops.
- `node.data_dir`: persistent state (identity keys, local DB); back this up.
- `node.log_level`: typically `info` in production.
- `node.mode`: currently `solo`.
- `node.receipt_cadence_tokens`: receipt emission interval.
- `node.include_models`: allow-list. If empty, all backend models are eligible.
- `node.exclude_models`: deny-list applied after include filtering.
- `network.listen_addrs`: libp2p listen transports (TCP + QUIC recommended).
- `network.inference_port`: HTTP API port (`/health`, `/status`, `/status/detail`, `/v1/*`).
- `network.bootstrap_peers`: initial peers to dial.
- `network.public_addr`: internet-facing multiaddrs (after port-forwarding) to advertise and inject into DHT.
  - both forms are valid:
    - without peer id: `/ip4/<ext-ip>/tcp/<ext-port>` or `/ip4/<ext-ip>/tcp/<ext-port>/ws`
    - with peer id: `/ip4/<ext-ip>/tcp/<ext-port>/p2p/<peer-id>`
  - if peer id is omitted, node uses its local peer id for DHT insertion.
- `network.expose_status_detail`: exposes `/status/detail` diagnostics endpoint.
  - default `false`: `/status/detail` returns `404 Not Found`.
  - set `true` for operator tooling and local dev diagnostics.
- `network.allow_non_globals_in_dht`: Substrate-style DHT privacy control.
  - default `false`: only global addresses are inserted into DHT.
  - set `true` for local/dev environments that rely on private or loopback ranges.
- `network.external_ip`: advertised public IP (recommended behind NAT).
- `backend.url`: upstream inference backend base URL.
- `backend.health_path`: backend health endpoint.
- `backend.models_path`: backend model-list endpoint.
- `backend.timeout_secs`: backend request timeout.
- `attestation.*`: attestation endpoint and mode toggle.
- `registry.*`: Unicity registration/heartbeat controls.
- `settlement.*`: settlement/escrow controls.
- `pricing.*`: local pricing values used by settlement/accounting logic.

## 5) Model exposure policy (important)

Filtering order is:

1. `include_models` first.
2. If `include_models` is empty, start with all backend models.
3. Apply `exclude_models` to remove blocked models.

Examples:

- Expose only two models:
  - `include_models = ["qwen/qwen3.5-9b", "meta-llama/llama-3.1-8b-instruct"]`
  - `exclude_models = []`
- Expose all except one private model:
  - `include_models = []`
  - `exclude_models = ["internal/research-model"]`

These rules affect both:

- `/v1/models` visibility
- `/v1/chat/completions` model validation

## 6) CLI overrides

Run with config:

```bash
./target/release/sparkl-solo --config config/prod.toml
```

Available CLI overrides (grouped by config section):

- `node.*`
  - `--name`
  - `--data-dir`
  - `--log-level`
  - `--mode` (`solo|farm`)
  - `--receipt-cadence`
  - `--include-models` (comma-separated)
  - `--exclude-models` (comma-separated)
- `network.*`
  - `--listen-addrs` (comma-separated)
  - `--inference-port`
  - `--external-ip`
  - `--bootstrap-peers` (comma-separated)
  - `--public-addr` (comma-separated)
  - `--expose-status-detail` (`true|false`)
  - `--allow-non-globals-in-dht` (`true|false`)
- `backend.*`
  - `--backend-url`
  - `--backend-health-path`
  - `--backend-models-path`
  - `--backend-timeout-secs`
- `attestation.*`
  - `--nras-url`
  - `--nras-enabled` (`true|false`)
  - `--cert-ttl-days`
- `registry.*`
  - `--registry-url`
  - `--registry-heartbeat-secs`
  - `--registry-enabled` (`true|false`)
- `settlement.*`
  - `--settlement-epoch-secs`
  - `--evm-rpc-url`
  - `--escrow-contract`
  - `--settlement-enabled` (`true|false`)
- `pricing.*`
  - `--price-input-micro-usd-per-m`
  - `--price-output-micro-usd-per-m`

If provided, CLI values override config file values.

### Environment variable overrides

Environment variables are also supported through `config` crate loading:

- Prefix: `SPARKLE__`
- Path separator: double underscore `__`
- Mapping example:
  - `SPARKLE__NETWORK__INFERENCE_PORT=19944`
  - `SPARKLE__BACKEND__URL=http://127.0.0.1:1234`
  - `SPARKLE__NETWORK__EXPOSE_STATUS_DETAIL=true`
  - `SPARKLE__NETWORK__ALLOW_NON_GLOBALS_IN_DHT=false`
  - `SPARKLE__REGISTRY__UNICITY_AGGREGATOR_URL=https://goggregator-test.unicity.network/` (Unicity JSON-RPC base URL when using `--features unicity`; same `SPARKLE__` + `__` mapping as other keys — no separate env integration)

For list values, use JSON arrays:

```bash
export SPARKLE__NETWORK__PUBLIC_ADDR='["/ip4/203.0.113.10/tcp/30333","/ip4/203.0.113.10/tcp/443/ws"]'
export SPARKLE__NETWORK__BOOTSTRAP_PEERS='["/dns/boot.sparkl.network/tcp/51993/p2p/12D3KooW..."]'
```

## 7) Port forwarding and firewall

Minimum inbound ports to forward/open:

- `30333/tcp` (libp2p TCP)
- `30333/udp` (libp2p QUIC)

Optional inbound:

- `9944/tcp` (HTTP inference API). Expose only if intended for external clients.

Recommended outbound:

- Backend access (`backend.url`)
- Bootstrap peers
- Registry/settlement endpoints (if enabled)
- NRAS endpoint (`attestation.nras_url`) when using NRAS

NAT/forwarding checklist:

- Router/cloud firewall forwards external `30333/tcp` and `30333/udp` to this host.
- Host firewall allows same ports.
- `network.external_ip` is set to public IP advertised to peers.
- At least one `bootstrap_peers` entry is configured for initial peer discovery.

If you do not want to expose raw inference API publicly, keep `inference_port` private and put a reverse proxy/auth gateway in front.

## 8) Run as a service (systemd)

Example unit file `/etc/systemd/system/sparkl-solo.service`:

```ini
[Unit]
Description=Sparkl Solo Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=sparkl
WorkingDirectory=/opt/sparkl-solo
ExecStart=/opt/sparkl-solo/target/release/sparkl-solo --config /etc/sparkl-solo/prod.toml
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

Enable/start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now sparkl-solo
sudo systemctl status sparkl-solo
```

## 9) Validation checklist

- `curl -s http://127.0.0.1:9944/health`
- `curl -s http://127.0.0.1:9944/status` (minimal public readiness)
- `curl -s http://127.0.0.1:9944/status/detail` (operator diagnostics)
- `curl -s http://127.0.0.1:9944/v1/models`
- Confirm expected model filtering from include/exclude config.
- Confirm peers appear in `/status/detail` after bootstrap and discovery.
- Confirm inbound connectivity on `30333/tcp` and `30333/udp`.

## 10) Security baseline

- Keep `node.data_dir` on persistent encrypted storage.
- Restrict filesystem permissions on key/state directories.
- Do not expose `inference_port` publicly without auth/rate limits.
- Pin bootstrap peers you trust.
- Monitor logs for repeated connection failures or unexpected model requests.
