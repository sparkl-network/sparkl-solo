# Sparkl's trust model

...is built on three layered proofs. 

**PAOW:** Proven Attestation of Work, a proof of work that is used to verify:

_(assuming TEE is used for the node's execution environment)_

- **PoR** — Proof of Registration establishes that a node's operator genuinely controls the node's private key: at registration time, the node signs a challenge containing the operator's address, chain ID, and registry contract address, and ECDSA.recover on-chain verifies the signer matches the declared nodeId — no third party can hijack a node's identity without its key. 

- **PoA** — Proof of Attestation elevates a node to TEE_VERIFIED tier: the independent attestation service submits a teeReportHash via setTEEProof, cryptographically binding the node's execution environment to a known, auditable enclave — users can trust that the node is running unmodified inference software inside a Trusted Execution Environment. 

- **PoU** — Proof of Usage closes the settlement loop: rather than trusting a provider's self-reported billing, the node continuously calls recordUsage(sessionId, usageDelta) to commit metered consumption on-chain, and the settlement operator enforces that toProvider in any settle call never exceeds usageRecorded — a provider cannot claim payment beyond what has been cryptographically committed. 

Together, **PoR** → **PoA** → **PoU** form a chain of custody from "this operator owns this node" through "this node runs trusted software" to "this much work was verifiably done" — making Sparkl's marketplace trustless end-to-end without requiring users to take any provider's word for it.