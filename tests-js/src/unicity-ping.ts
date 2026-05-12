// Smoke-test reachability against a Unicity JSON-RPC gateway.

export {};

const url =
  process.env.UNICITY_AGGREGATOR_URL?.trim() ||
  "https://goggregator-test.unicity.network/";
const stateIdForProof = process.env.UNICITY_SMOKE_STATE_ID?.trim();

/** Public testnet exposes get_block_height per shardId (required on goggregator-test). */
const blockHeightShardId = Number(process.env.UNICITY_SMOKE_SHARD_ID ?? "2");

async function pingBlockHeight() {
  if (!Number.isInteger(blockHeightShardId) || blockHeightShardId < 0) {
    throw new Error(
      `UNICITY_SMOKE_SHARD_ID must be a non-negative integer, got "${process.env.UNICITY_SMOKE_SHARD_ID}"`,
    );
  }

  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      method: "get_block_height",
      params: { shardId: blockHeightShardId },
      id: 1,
    }),
  });

  const raw = await res.text();
  if (!res.ok) {
    throw new Error(`Unicity ping failed with HTTP ${res.status}: ${raw.slice(0, 200)}`);
  }

  let json: { result?: { blockNumber?: string }; error?: unknown };
  try {
    json = JSON.parse(raw) as { result?: { blockNumber?: string }; error?: unknown };
  } catch (error) {
    throw new Error(
      `Unicity ping returned non-JSON body: ${raw.slice(0, 200)} (${String(error)})`,
    );
  }

  if (json.error != null) {
    throw new Error(`Unicity get_block_height RPC error: ${JSON.stringify(json.error)}`);
  }

  console.assert(typeof json.result?.blockNumber === "string", "expected blockNumber string");
  console.log("Unicity block height:", json.result?.blockNumber);
}

async function pingInclusionProofV2(stateId: string) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      method: "get_inclusion_proof.v2",
      params: { stateId },
      id: 2,
    }),
  });

  const raw = await res.text();
  if (!res.ok) {
    throw new Error(`Unicity ping failed with HTTP ${res.status}: ${raw.slice(0, 200)}`);
  }

  let json: { result?: unknown; error?: unknown };
  try {
    json = JSON.parse(raw) as { result?: unknown; error?: unknown };
  } catch (error) {
    throw new Error(
      `Unicity ping returned non-JSON body: ${raw.slice(0, 200)} (${String(error)})`,
    );
  }

  if (json.error != null) {
    throw new Error(`Unicity get_inclusion_proof.v2 RPC error: ${JSON.stringify(json.error)}`);
  }

  console.assert(
    typeof json.result === "string" && json.result.length > 0,
    "expected non-empty hex string result",
  );
  console.log("Unicity inclusion proof v2 (hex prefix):", String(json.result).slice(0, 64) + "…");
}

if (stateIdForProof) {
  await pingInclusionProofV2(stateIdForProof);
} else {
  await pingBlockHeight();
}
