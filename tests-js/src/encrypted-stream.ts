import nacl from "tweetnacl";

import { MODEL, NODE1_URL, getStatus, hexToBytes } from "./common.js";

type ChatChunk = {
  choices?: Array<{ delta?: { content?: string; reasoning_content?: string } }>;
  sparkl?: { seq?: number; receipt?: string };
};

function splitLines(buffer: string): [string[], string] {
  const lines = buffer.split("\n");
  const tail = lines.pop() ?? "";
  return [lines, tail];
}

async function main() {
  const status = await getStatus(NODE1_URL);
  const providerPubkey = hexToBytes(status.identity.x25519_pubkey);
  if (providerPubkey.length !== nacl.box.publicKeyLength) {
    throw new Error("provider x25519 pubkey length mismatch");
  }

  const ephemeral = nacl.box.keyPair();
  const nonce = nacl.randomBytes(nacl.box.nonceLength);
  const plaintext = JSON.stringify({
    model: MODEL,
    messages: [{ role: "user", content: "Hello encrypted from tests-js" }],
    stream: true,
  });

  const ciphertext = nacl.box(
    new TextEncoder().encode(plaintext),
    nonce,
    providerPubkey,
    ephemeral.secretKey,
  );
  const wire = new Uint8Array(nonce.length + ciphertext.length);
  wire.set(nonce, 0);
  wire.set(ciphertext, nonce.length);

  const resp = await fetch(`${NODE1_URL}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      encrypted: true,
      epk: Buffer.from(ephemeral.publicKey).toString("base64"),
      ciphertext: Buffer.from(wire).toString("base64"),
    }),
  });

  if (!resp.ok || !resp.body) {
    throw new Error(`request failed (${resp.status})`);
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let tail = "";
  let receipts = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    tail += decoder.decode(value, { stream: true });
    const [lines, nextTail] = splitLines(tail);
    tail = nextTail;

    for (const line of lines) {
      if (!line.startsWith("data: ")) continue;
      const payload = line.slice(6).trim();
      if (payload === "[DONE]") {
        console.log(`\n[DONE] receipts=${receipts}`);
        return;
      }
      const data = JSON.parse(payload) as ChatChunk;
      if (data.sparkl?.receipt) {
        receipts += 1;
        process.stdout.write(`\n[receipt seq=${data.sparkl.seq}] `);
      }
      const content =
        data.choices?.[0]?.delta?.content ??
        data.choices?.[0]?.delta?.reasoning_content;
      if (content) process.stdout.write(content);
    }
  }

  console.log(`\n[stream ended] receipts=${receipts}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
