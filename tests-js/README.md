# tests-js

JavaScript/TypeScript operational tests for `sparkl-solo`.

## Setup

```bash
cd tests-js
yarn install
```

## Defaults

- `NODE1_URL=http://127.0.0.1:19944`
- `NODE2_URL=http://127.0.0.1:19945`
- `MODEL=qwen/qwen3.5-9b`

Override with env vars as needed.

## Commands

```bash
# status + peers snapshot
yarn status

# attestation challenge on node1
yarn attestation

# encrypted request + SSE stream + receipt visibility
yarn encrypted

# full TPM-related operational suite (reads dev-config ports)
yarn tpm:suite
```

## TPM Suite Notes

- Uses config files directly:
  - `../dev-config/node1.toml`
  - `../dev-config/node2.toml`
- Checks:
  - TPM socket file exists (`/tmp/sparkl-tpm.sock`)
  - `/status/detail` identity fields + distinct peer IDs
  - `/attestation/challenge`
  - `/v1/models`
  - cross-node `/receipts/verify` using a receipt from node1 stream
- Optional env vars:
  - `NODE1_CONFIG`, `NODE2_CONFIG`
  - `TPM_SOCKET`
  - `MODEL`
  - `EXPECT_CERT_TYPE` (default `mock-software`, set to `any` to skip strict check)
