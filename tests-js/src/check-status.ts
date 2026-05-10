import { NODE1_URL, NODE2_URL, getStatus } from "./common.js";

async function main() {
  const [s1, s2] = await Promise.all([getStatus(NODE1_URL), getStatus(NODE2_URL)]);
  console.log("node1", {
    url: NODE1_URL,
    peer_id: s1.peer_id,
    peers_known: s1.peers_known ?? 0,
    peers: s1.peers ?? [],
  });
  console.log("node2", {
    url: NODE2_URL,
    peer_id: s2.peer_id,
    peers_known: s2.peers_known ?? 0,
    peers: s2.peers ?? [],
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
