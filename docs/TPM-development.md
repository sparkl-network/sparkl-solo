# TPM Testing Options on Mac

## Important: `tpm2-tools` on macOS

- `swtpm` is available via Homebrew (`brew install swtpm`).
- `tpm2-tools` is currently not available in Homebrew core (`brew install tpm2-tools` returns "No available formula").
- In this repo, running with `--features tpm` on macOS still works:
  - when `TCTI`/`TPM2TOOLS_TCTI` is set **and** `tpm2_getrandom` exists, identity reports `cert_type: "swtpm"`.
  - when `tpm2_getrandom` is not installed, identity falls back to software and reports `cert_type: "mock-software"`.
- Use Linux (or a container/another machine with `tpm2-tools`) for full TPM2 CLI verification commands in this document.

## Option 1: Software TPM (swtpm) — Recommended for Dev

swtpm is a free, open-source software TPM 2.0 emulator. It implements the full TPM2 spec in software — your Rust tss-esapi bindings talk to it over a Unix socket exactly as they would to real hardware. No DGX required.

bash

# macOS install via Homebrew

brew install swtpm

# Start a software TPM instance

mkdir -p /tmp/sparkl-tpm  
swtpm socket   
  --tpmstate dir=/tmp/sparkl-tpm   
  --ctrl type=unixio,path=/tmp/sparkl-tpm.ctrl   
  --server type=unixio,path=/tmp/sparkl-tpm.sock   
  --flags not-need-init,startup-clear   
  --tpm2

# Point tss-esapi at the socket

export TPM2TOOLS_TCTI="swtpm:path=/tmp/sparkl-tpm.sock"  
export TCTI="swtpm:path=/tmp/sparkl-tpm.sock"

cargo run --features tpm
swtpm supports all the operations Sparkle uses: key generation, signing, PCR reads, and quote generation. The only thing it cannot emulate is NVIDIA NRAS attestation (which requires real GPU hardware) — but that's gated behind nras_enabled = false in your config anyway.

Cost: free, open source (GPL).

## Option 2: IBM TPM2 Simulator

IBM ships a reference TPM2 simulator written in C — the same codebase used in many TPM compliance test suites. More faithful to hardware edge cases than swtpm.

bash

# Clone and build

git clone [https://github.com/kgoldman/ibmswtpm2](https://github.com/kgoldman/ibmswtpm2)
cd ibmswtpm2/src && make

# Run on localhost port 2321 (default TPM2 port)

./tpm_server &

# Tell tss-esapi to use it

export TCTI="mssim:host=localhost,port=2321"

cargo run --features tpm
Cost: free, BSD license.

## Option 3: macOS Secure Enclave as Stand-in

macOS has its own hardware security module — the Secure Enclave — accessible via the security framework and the CryptoKit Swift API. It does key generation, signing, and non-exportable key storage, just like TPM2.

However: the Rust TPM2 bindings (tss-esapi) do not talk to the Secure Enclave — they speak the TCG TPM2 spec, which is different. You'd need a shim or a separate SecureEnclave backend behind your identity.rs trait.

Verdict: more work than swtpm, saves nothing. Use swtpm instead.

## Option 4: Docker with swtpm

If you want a fully isolated, reproducible TPM test environment that mirrors the DGX's Ubuntu 24.04 DGX OS:

text

# Dockerfile.tpm-dev

FROM ubuntu:24.04
RUN apt-get update && apt-get install -y   
    swtpm tpm2-tools tpm2-abrmd   
    curl build-essential pkg-config   
    libssl-dev libtss2-dev
text

# docker-compose.yml

services:
  tpm:
    image: sparkle-tpm-dev
    volumes:
      - ./:/workspace
      - tpm-state:/tmp/sparkle-tpm
    environment:
      - TCTI=swtpm:path=/tmp/sparkle-tpm.sock
    command: >
      sh -c "swtpm socket --tpmstate dir=/tmp/sparkle-tpm
             --tpm2 --flags not-need-init,startup-clear
             --server type=unixio,path=/tmp/sparkle-tpm.sock
             --ctrl type=unixio,path=/tmp/sparkle-tpm.ctrl &
             sleep 1 && cargo run --features tpm"
This gives you an Ubuntu environment with a software TPM — identical to what sparkle-node1 will see on the DGX, minus the NRAS step.

What swtpm Cannot Test
Feature	swtpm	Real DGX TPM2
Key generation (non-exportable)	✅ emulated	✅ hardware
Ed25519 / ECDSA signing	✅ emulated	✅ hardware
PCR reads and quotes	✅ emulated	✅ hardware
Sealed storage (PCR-bound keys)	✅ emulated	✅ hardware
NRAS attestation submission	❌ no real GPU	✅ real cert from NVIDIA
Physical tamper resistance	❌ software only	✅ hardware enforced
TPM2 PT_MANUFACTURER = NVIDIA	❌ IBM/generic	✅ NVIDIA-specific
For everything except NRAS, swtpm is sufficient. NRAS stays behind nras_enabled = false until you're ready to test on the actual DGX.

Recommended Dev Setup
text

# ~/.sparkle/config.toml (dev on Mac)

[attestation]
nras_enabled = false      # stays false until DGX
cert_ttl_days = 7

[node]

# runs against swtpm via TCTI env var

bash

# Terminal 1: start software TPM

swtpm socket --tpmstate dir=/tmp/sparkle-tpm --tpm2   
  --flags not-need-init,startup-clear   
  --server type=unixio,path=/tmp/sparkle-tpm.sock   
  --ctrl type=unixio,path=/tmp/sparkle-tpm.ctrl

# Terminal 2: run node with real TPM feature against swtpm

export TCTI="swtpm:path=/tmp/sparkle-tpm.sock"
cargo run --features tpm
Total cost: £0. When you're ready to test NRAS, you ssh to your DGX, set nras_enabled = true, and run with the same binary compiled for aarch64. The TPM2 code path is identical — only the hardware underneath changes.





## Testing swtpm on Mac: Step by Step

## 1. Verify Both swtpm Instances Are Running

```
bash
```

`# Check both TPM sockets exist`  
`ls -la /tmp/sparkle-tpm-*.sock 2>/dev/null || ls -la /tmp/sparkle-tpm*.sock`  
  
`# Or however you named them — check your config`  
`# Node 1 TPM socket`  
`tpm2_getcap properties-fixed --tcti="swtpm:path=/tmp/sparkle-tpm-1.sock" 2>/dev/null | grep TPM2_PT_MANUFACTURER`  
  
`# Node 2 TPM socket`  
`tpm2_getcap properties-fixed --tcti="swtpm:path=/tmp/sparkle-tpm-2.sock" 2>/dev/null | grep TPM2_PT_MANUFACTURER`

Both should respond — confirms the swtpm daemons are alive and accepting connections.

---

## 2. Check Each Node's Identity

```
bash
```

`# Node 1 (port 19944 from your earlier test)`  
`curl -s http://127.0.0.1:19944/status/detail | jq '{`  
  `peer_id,`  
  `attestation,`  
  `tpm_pubkey: .identity.pubkey`  
`}'`  
  
`# Node 2 (whatever port you configured — likely 29944 or 19945)`  
`curl -s http://127.0.0.1:29944/status/detail | jq '{`  
  `peer_id,`  
  `attestation,`  
  `tpm_pubkey: .identity.pubkey`  
`}'`

The two `peer_id` values must be **different** — each swtpm instance generated its own keypair. If they're the same, both nodes are sharing one swtpm socket (config issue).

---

## 3. Confirm DHT Peer Discovery

```
bash
```

`# Node 1's known peers — should include Node 2's peer_id`  
`curl -s http://127.0.0.1:19944/status/detail | jq '.peers_known, .peers'`  
  
`# Node 2 should know Node 1`  
`curl -s http://127.0.0.1:29944/status/detail | jq '.peers_known, .peers'`

You already confirmed this works from your earlier test — just verify the peer_ids match what `/status/detail` reports for each node.

---

## 4. Test Encrypted Request (E2E Crypto)

This is the key TPM test — send a NaCl Box encrypted request to Node 1, verify it decrypts and proxies correctly. Write a small TypeScript test client:

```
typescript
```

`// test-encrypted.ts`  
`import { box, randomBytes } from 'tweetnacl';`  
`import { encodeBase64 } from 'tweetnacl-util';`  
  
`// 1. Fetch Node 1's X25519 pubkey from /status/detail`  
`const status = await fetch('http://127.0.0.1:19944/status/detail').then(r => r.json());`  
`const providerPubkey = Uint8Array.from(Buffer.from(status.identity.x25519_pubkey, 'hex'));`  
  
`// 2. Generate consumer ephemeral keypair`  
`const ephemeral = box.keyPair();`  
  
`// 3. Derive shared secret + encrypt request`  
`const nonce = randomBytes(box.nonceLength);`  
`const plaintext = JSON.stringify({`  
  `model: 'qwen/qwen3.5-9b',   // use whatever your backend has loaded`  
  `messages: [{ role: 'user', content: 'Hello encrypted from Sparkle test' }],`  
  `stream: true`  
`});`  
  
`const ciphertext = box(`  
  `Buffer.from(plaintext),`  
  `nonce,`  
  `providerPubkey,`  
  `ephemeral.secretKey`  
`);`  
  
`// 4. Send encrypted request`  
`const body = JSON.stringify({`  
  `encrypted: true,`  
  `epk: encodeBase64(ephemeral.publicKey),`  
  `nonce: encodeBase64(nonce),`  
  `ciphertext: encodeBase64(ciphertext)`  
`});`  
  
`const resp = await fetch('http://127.0.0.1:19944/v1/chat/completions', {`  
  `method: 'POST',`  
  `headers: { 'Content-Type': 'application/json' },`  
  `body,`  
`});`  
  
`// 5. Stream and verify receipts`  
`for await (const chunk of resp.body) {`  
  `const lines = Buffer.from(chunk).toString().split('\n');`  
  `for (const line of lines) {`  
    `if (!line.startsWith('data: ') || line === 'data: [DONE]') continue;`  
    `const data = JSON.parse(line.slice(6));`  
    `if (data.sparkl?.receipt) {`  
      `const receipt = JSON.parse(Buffer.from(data.sparkl.receipt, 'base64').toString());`  
      `console.logseq=${receipt.seq} tokens=${receipt.token_count} sig=${receipt.provider_sig.slice(0,4)}...);`  
    `}`  
    `const content = data.choices?.[0]?.delta?.content`  
                 `|| data.choices?.[0]?.delta?.reasoning_content;`  
    `if (content) process.stdout.write(content);`  
  `}`  
`}`

```
bash
```

`# Run it`  
`npx tsx test-encrypted.ts`

If the node decrypts with its swtpm-held key and streams back plaintext chunks — the TPM E2E path is confirmed working.

---

## 5. Test TPM Challenge-Response Directly

Verify the swtpm key is actually being used for signing (not falling back to software):

```
bash
```

`# Hit Node 1's attestation challenge endpoint directly`  
`curl -s -X POST http://127.0.0.1:19944/attestation/challenge \`  
  `-H "Content-Type: application/json" \`  
  `-d '{"nonce": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}' \`  
  `| jq '{`  
      `provider_id,`  
      `nonce,`  
      `signature,`  
      `cert_type: .attestation.cert_type   # should be "swtpm" not "mock-software"`  
    `}'`

The response should show `cert_type: "swtpm"` (or `"tpm"`) rather than `"mock-software"`. If you see `"mock-software"`, the `tpm` feature compiled in but TCTI env var isn't being picked up — check that `TCTI` or `TPM2TOOLS_TCTI` is set in the process environment where the node started.

---

## 6. Test Node-to-Node Receipt Verification

This verifies Node 2 can verify a receipt signed by Node 1's swtpm key:

```
bash
```

`# 1. Get a receipt from Node 1 by making a request (as above)`  
`RECEIPT=$(curl -sN http://127.0.0.1:19944/v1/chat/completions \`  
  `-H "Content-Type: application/json" \`  
  `-d '{"model":"qwen/qwen3.5-9b","messages":[{"role":"user","content":"hi"}],"stream":true}' \`  
  `| grep -m1 '"receipt"' \`  
  `| python3 -c "import sys,json,re; d=json.loads(re.search(r'data: (.+)', sys.stdin.read()).group(1)); print(d['sparkl']['receipt'])")`  
  
`echo "Receipt: $RECEIPT"`  
  
`# 2. Ask Node 2 to verify it (cross-node verification endpoint)`  
`curl -s -X POST http://127.0.0.1:29944/receipts/verify \`  
  `-H "Content-Type: application/json" \`  
  `-d "{`  
    `\"receipt\": \"$RECEIPT\",`  
    `\"provider_pubkey\": \"$(curl -s http://127.0.0.1:19944/status/detail | jq -r .identity.ed25519_pubkey)\"`  
  `}" \`  
  `| jq '{valid, reason}'`

Expected: `{"valid": true, "reason": "signature_ok"}`. This proves the Ed25519 signing from Node 1's swtpm is verifiable by an independent third party using only the public key — no trust in the signing node required.

---

## 7. Smoke Test: What Passing Looks Like

```
text
```

`✅ Both /status/detail endpoints respond with different peer_ids`  
`✅ peers_known > 0 on both nodes (DHT working)`  
`✅ /v1/models returns backend model list on both nodes`  
`✅ Unencrypted completions stream with sparkl receipts (you already have this)`  
`✅ Encrypted completions decrypt and stream correctly (test 4)`  
`✅ Attestation challenge returns cert_type: "swtpm" not "mock-software" (test 5)`  
`✅ Receipt from Node 1 verifies on Node 2 using only public key (test 6)`

Once all seven pass, the TPM path is fully validated on Mac. The only delta when moving to the DGX is: `nras_enabled = true`, `TCTI` points at the hardware TPM device (`/dev/tpm0` or `tabrmd`socket), and `cert_type` changes from `"swtpm"` to `"nras"`.

---

## JavaScript test harness

This repository now includes a runnable JS harness in `tests-js/` for repeated TPM/runtime checks:

```bash
cd tests-js
yarn install
yarn status
yarn attestation
yarn encrypted
yarn tpm:suite
```

`yarn tpm:suite` reads `dev-config/node1.toml` and `dev-config/node2.toml`, validates `/status/detail`, `/attestation/challenge`, `/v1/models`, and cross-node `/receipts/verify`.