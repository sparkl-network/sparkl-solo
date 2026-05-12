# TEE attestation stub (pre–real SGX / SEV / Nitro)

HTTP service that:

1. Issues a short‑lived challenge (nonce).
2. Verifies the **provider wallet** signed that challenge (no TEE quote parsing yet).
3. Calls on‑chain `ProviderRegistry.setTEEProof(provider, keccak256(report))` using the **registry `attestationService` key** (`ADMIN_PRIVATE_KEY` must be that address).

## Prereqs

- Node 18+
- A deployed `ProviderRegistry` whose `attestationService` is the address derived from `ADMIN_PRIVATE_KEY`
- `yarn` (repo convention)

## Setup

```bash
cd services/tee-attestation-stub
yarn install
cp .env.example .env
# edit .env — critical: PROVIDER_REGISTRY_ADDRESS, RPC_URL, ADMIN_PRIVATE_KEY matching on-chain attestationService
```

## Run

```bash
yarn start
```

## API

### `GET /health`

`{ "ok": true }`

### `GET /v1/challenge`

Returns a fresh challenge for the provider wallet to sign:

```json
{ "challengeId": "0xabc...", "message": "...", "expiresAt": 1234567890000 }
```

`message` is the UTF‑8 string passed to `ethers.Wallet.signMessage(message)`.

### `POST /v1/attest`

```json
{
  "providerAddress": "0x...",
  "report": "0x....",
  "challengeId": "0x...",
  "signature": "0x..."
}
```

- **`report`**: opaque attestation blob as hex (stub: any bytes; later replace with DCAP/SEV‑SNP/Nitro payloads).
- **`signature`**: ECDSA over the EIP‑191 personal message for `message` from the challenge step (same as `signMessage`).

On success: `{ "ok": true, "teeReportHash": "0x...", "txHash": "0x..." }`

## Client sketch (ethers v6)

```js
const challenge = await fetch("http://localhost:8787/v1/challenge").then((r) => r.json());
const sig = await signer.signMessage(challenge.message);
await fetch("http://localhost:8787/v1/attest", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    providerAddress: await signer.getAddress(),
    report: "0xbeef", // stub TEE report bytes
    challengeId: challenge.challengeId,
    signature: sig,
  }),
});
```

## Security

- Stub only: anyone who can steal the provider key can pass the check. Real TEE verification replaces `report` handling.
- **`ADMIN_PRIVATE_KEY`** is high value; keep off disk in production (HSM, KMS, separate vault).
- Consider mTLS / API keys in front of this service before exposing beyond localhost.
