// Smoke-test: can we reach the Unicity testnet?
export {};

const res = await fetch("https://gateway-test.unicity.network/", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    jsonrpc: "2.0",
    method: "get_block_height",
    params: {},
    id: 1,
  }),
});

const raw = await res.text();
if (!res.ok) {
  throw new Error(`Unicity ping failed with HTTP ${res.status}: ${raw.slice(0, 200)}`);
}

let json: { result?: { blockNumber?: string } };
try {
  json = JSON.parse(raw) as { result?: { blockNumber?: string } };
} catch (error) {
  throw new Error(
    `Unicity ping returned non-JSON body: ${raw.slice(0, 200)} (${String(error)})`,
  );
}

console.assert(typeof json.result?.blockNumber === "string", "expected blockNumber string");
console.log("Unicity testnet block height:", json.result?.blockNumber);
