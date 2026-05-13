# TEE attestation stub (pre–real SGX / SEV / Nitro)

HTTP service that:

1. Issues a short‑lived challenge (nonce).
2. Verifies the **node operator** signed that challenge (no TEE quote parsing yet). The signature must recover to **`ProviderRegistry.nodeOperator(nodeId)`**.
3. Calls on-chain **`ProviderRegistry.setTEEProof(nodeId, keccak256(report))`** using the **registry `attestationService` key** (`ADMIN_PRIVATE_KEY` must be that address).

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

Returns a fresh challenge for the **node operator** wallet to sign:

```json
{ "challengeId": "0xabc...", "message": "...", "expiresAt": 1234567890000 }
```

`message` is the UTF‑8 string passed to `ethers.Wallet.signMessage(message)`.

### `POST /v1/attest`

```json
{
  "nodeId": "0x0123abcd...",
  "report": "0x....",
  "challengeId": "0x...",
  "signature": "0x..."
}
```

- **`nodeId`**: **`bytes32`** as hex (`0x` + 64 hex chars), exactly as registered with **`registerNode`**.
- **`report`**: opaque attestation blob as hex (stub: any bytes; later replace with DCAP/SEV‑SNP/Nitro payloads).
- **`signature`**: ECDSA over the EIP‑191 personal message for `message` from the challenge step (same as `signMessage`). Must recover to **`nodeOperator(nodeId)`**.

On success: `{ "ok": true, "teeReportHash": "0x...", "txHash": "0x..." }`

**Legacy:** `providerAddress` is no longer accepted as the first on-chain argument (the contract takes **`nodeId`**). The stub still reads `providerAddress` from the JSON body only as a **deprecated alias for `nodeId`** when `nodeId` is omitted; pass **`nodeId`** explicitly.

## Client sketch (ethers v6)

```js
const challenge = await fetch("http://localhost:8787/v1/challenge").then((r) => r.json());
const sig = await operatorSigner.signMessage(challenge.message);
await fetch("http://localhost:8787/v1/attest", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    nodeId: registeredNodeIdBytes32, // same bytes32 as registerNode
    report: "0xbeef", // stub TEE report bytes
    challengeId: challenge.challengeId,
    signature: sig,
  }),
});
```

## Security

- Stub only: anyone who can steal the operator key can pass the check. Real TEE verification replaces `report` handling.
- **`ADMIN_PRIVATE_KEY`** is high value; keep off disk in production (HSM, KMS, separate vault).
- Consider mTLS / API keys in front of this service before exposing beyond localhost.
