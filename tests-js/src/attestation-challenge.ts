import { NODE1_URL } from "./common.js";

const nonce =
  process.env.NONCE ??
  "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

async function main() {
  const resp = await fetch(`${NODE1_URL}/attestation/challenge`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ nonce }),
  });
  const body = await resp.json();
  console.log({ status: resp.status, body });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
