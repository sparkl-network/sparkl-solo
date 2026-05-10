import { access, readFile } from "node:fs/promises";

type StatusPayload = {
  peer_id: string;
  identity: {
    x25519_pubkey: string;
    ed25519_pubkey: string;
  };
  peers_known?: number;
  peers?: string[];
};

const NODE1_CONFIG = process.env.NODE1_CONFIG ?? "../dev-config/node1.toml";
const NODE2_CONFIG = process.env.NODE2_CONFIG ?? "../dev-config/node2.toml";
const TPM_SOCKET = process.env.TPM_SOCKET ?? "/tmp/sparkl-tpm.sock";
const MODEL = process.env.MODEL ?? "qwen/qwen3.5-9b";
const EXPECT_CERT_TYPE = process.env.EXPECT_CERT_TYPE ?? "mock-software";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function nodeUrlFromConfig(configPath: string): Promise<string> {
  const raw = await readFile(configPath, "utf8");
  const m = raw.match(/^\s*inference_port\s*=\s*(\d+)\s*$/m);
  assert(m, `inference_port not found in ${configPath}`);
  return `http://127.0.0.1:${m[1]}`;
}

async function getJson<T>(url: string): Promise<T> {
  const resp = await fetch(url);
  assert(resp.ok, `${url} failed with status ${resp.status}`);
  return (await resp.json()) as T;
}

function isHex(value: string, len: number): boolean {
  return value.length === len && /^[0-9a-f]+$/i.test(value);
}

async function extractFirstReceipt(nodeUrl: string): Promise<string> {
  const resp = await fetch(`${nodeUrl}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model: MODEL,
      messages: [{ role: "user", content: "Return 120 short numbered items." }],
      stream: true,
    }),
  });
  assert(resp.ok, `stream request failed (${resp.status})`);
  assert(resp.body, "stream body missing");

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let tail = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    tail += decoder.decode(value, { stream: true });
    const lines = tail.split("\n");
    tail = lines.pop() ?? "";

    for (const line of lines) {
      if (!line.startsWith("data: ")) continue;
      const payload = line.slice(6).trim();
      if (payload === "[DONE]") continue;
      try {
        const obj = JSON.parse(payload) as {
          sparkl?: { receipt?: string };
        };
        const receipt = obj.sparkl?.receipt;
        if (receipt) return receipt;
      } catch {
        // ignore malformed/non-json chunks
      }
    }
  }
  throw new Error("no receipt found in streamed response");
}

async function main() {
  await access(TPM_SOCKET);
  console.log(`TPM socket found: ${TPM_SOCKET}`);

  const [node1Url, node2Url] = await Promise.all([
    nodeUrlFromConfig(NODE1_CONFIG),
    nodeUrlFromConfig(NODE2_CONFIG),
  ]);
  console.log(`node1 URL from config: ${node1Url}`);
  console.log(`node2 URL from config: ${node2Url}`);

  const [s1, s2] = await Promise.all([
    getJson<StatusPayload>(`${node1Url}/status`),
    getJson<StatusPayload>(`${node2Url}/status`),
  ]);
  assert(s1.peer_id !== s2.peer_id, "node1 and node2 peer_id must be different");
  assert(isHex(s1.identity.x25519_pubkey, 64), "node1 x25519 pubkey must be 32-byte hex");
  assert(isHex(s1.identity.ed25519_pubkey, 64), "node1 ed25519 pubkey must be 32-byte hex");
  assert(isHex(s2.identity.x25519_pubkey, 64), "node2 x25519 pubkey must be 32-byte hex");
  assert(isHex(s2.identity.ed25519_pubkey, 64), "node2 ed25519 pubkey must be 32-byte hex");

  const nonce = "deadbeef".repeat(8);
  const challengeResp = await fetch(`${node1Url}/attestation/challenge`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ nonce }),
  });
  assert(challengeResp.ok, `attestation challenge failed (${challengeResp.status})`);
  const challenge = (await challengeResp.json()) as {
    provider_id: string;
    signature: string;
    attestation?: { cert_type?: string };
  };
  assert(challenge.provider_id === s1.peer_id, "challenge provider_id must match node1 peer_id");
  assert(isHex(challenge.signature, 128), "challenge signature must be 64-byte hex");
  if (EXPECT_CERT_TYPE !== "any") {
    assert(
      challenge.attestation?.cert_type === EXPECT_CERT_TYPE,
      `expected cert_type=${EXPECT_CERT_TYPE}, got ${challenge.attestation?.cert_type}`,
    );
  }

  const [m1, m2] = await Promise.all([
    getJson<{ data?: Array<{ id: string }> }>(`${node1Url}/v1/models`),
    getJson<{ data?: Array<{ id: string }> }>(`${node2Url}/v1/models`),
  ]);
  assert((m1.data?.length ?? 0) > 0, "node1 /v1/models returned no models");
  assert((m2.data?.length ?? 0) > 0, "node2 /v1/models returned no models");

  const receipt = await extractFirstReceipt(node1Url);
  const verifyResp = await fetch(`${node2Url}/receipts/verify`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      receipt,
      provider_pubkey: s1.identity.ed25519_pubkey,
    }),
  });
  assert(verifyResp.ok, `receipt verify failed (${verifyResp.status})`);
  const verify = (await verifyResp.json()) as { valid: boolean; reason: string };
  assert(verify.valid, `receipt verification invalid: ${verify.reason}`);

  console.log("TPM suite passed", {
    node1_peer_id: s1.peer_id,
    node2_peer_id: s2.peer_id,
    node1_peers_known: s1.peers_known ?? 0,
    node2_peers_known: s2.peers_known ?? 0,
    models_node1: m1.data?.length ?? 0,
    models_node2: m2.data?.length ?? 0,
    receipt_verify: verify.reason,
    cert_type: challenge.attestation?.cert_type ?? "unknown",
  });
}

main().catch((err) => {
  console.error("TPM suite failed:", err.message);
  process.exit(1);
});
