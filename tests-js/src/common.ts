export type NodeStatus = {
  peer_id: string;
  identity: {
    x25519_pubkey: string;
    ed25519_pubkey: string;
  };
  peers_known?: number;
  peers?: string[];
};

export const NODE1_URL = process.env.NODE1_URL ?? "http://127.0.0.1:19944";
export const NODE2_URL = process.env.NODE2_URL ?? "http://127.0.0.1:19945";
export const MODEL = process.env.MODEL ?? "qwen/qwen3.5-9b";

export async function getStatus(baseUrl: string): Promise<NodeStatus> {
  const resp = await fetch(`${baseUrl}/status`);
  if (!resp.ok) {
    throw new Error(`status failed (${resp.status}) for ${baseUrl}`);
  }
  return (await resp.json()) as NodeStatus;
}

export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) {
    throw new Error("invalid hex length");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}
