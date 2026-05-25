# Verifiable AI Inference: Proving the Model You Paid For

## Summary

Survey of verifiable AI inference as of May 2026. API contracts typically do not bind the model billed to the weights executed; the sections below survey mechanisms that address it.

- zkML (ZK proofs): Cryptographic binding, no honest-party assumption. Systems include zkLLM, NANOZK, TensorCommitments, EZKL, DeepProve, zkPyTorch/Expander, JSTprove. zkLLM: ~15 min proving for 13B (2024); NANOZK: 24 ms layer verification, ~70× smaller proofs than EZKL at depth 128 (2026).
- TEEs: Remote attestation rooted in hardware (Anthropic/Irregular whitepaper, June 2025). NVIDIA H100/Blackwell GPU-CC is the practical deployment path; Tinfoil Modelwrap binds weight commitments to enclave measurement.
- opML: Optimistic fraud proofs (ORA); bisection dispute games from rollup designs. Low cost at scale; ~7-day dispute window excludes interactive latency targets.
- SVIP: Statistical checks on hidden activations; sub-10 ms overhead, <5% error in reported evaluations, requires API access to internals.
- Verde / Gensyn: Refereed delegation with RepOps for bitwise-reproducible ops; requires ≥1 honest replica, negligible proving overhead. On Gensyn mainnet (April 2026).
- OTR: TEE attestation plus optimistic challengers and stochastic ZK spot-checks (DGrid); targets the Verifiability Trilemma with sub-second TEE finality.

Decentralised integration: see §11 (EigenLayer AVS, Gensyn).

## Executive Summary

When a client pays for frontier-model inference through a commercial API, the protocol does not prevent the provider from substituting a cheaper or quantised model and returning outputs as if the billed model ran. Providers can do this with straightforward operational changes; standard terms of service do not supply a technical countermeasure.

Verifiable inference covers mechanisms that bind a response to execution of specified weights: zero-knowledge proofs, TEE remote attestation, optimistic fraud proofs, statistical checks on internal activations, and hybrids. This survey covers zkML, TEEs, opML, behavioural fingerprinting, and combined architectures, with emphasis on papers and deployed systems through mid-2026.

---

## 1. The Problem: The Model Substitution Gap

### 1.1 Trust Architecture of MLaaS

Machine Learning as a Service (MLaaS) follows a simple bilateral structure: a client sends an input (prompt, image, structured data) to an inference endpoint, and a server processes it using a pre-trained model and returns a result. The client pays for the capability level of a specific model. Nothing in the protocol binds the response to execution of the claimed model.[1][2]

A malicious provider can:

- Substitute a smaller, cheaper model (a 7B parameter model instead of a 70B one)
- Apply aggressive quantisation that degrades quality
- Return cached responses from a prior run
- Return outputs from a model that has been fine-tuned away from the claimed base weights

All of these attacks are undetectable to the client using standard API interactions. The 2026 paper NANOZK articulates this directly: "Service providers could substitute cheaper models, apply aggressive quantization, or return cached responses — all undetectable by users paying premium prices for frontier capabilities."[3][4][5]

### 1.2 The Verifiability Trilemma

Any system attempting to prove inference integrity faces a trilemma among three desirable properties:[6][7]

- Computational Integrity: The proof must be cryptographically sound, binding the output to the execution of specific weights
- Low Latency: The proof must be generated and verified without prohibitive delay (seconds, not hours)
- Low Cost: Proof generation must not cost orders of magnitude more than the inference itself

No existing scheme simultaneously satisfies all three. This tension organises the entire design space.[8][9]

---

## 2. Taxonomy of Approaches

Five approaches dominate the design space; they differ in trust assumptions, latency and cost overhead, deployment maturity, and what must change on the client relative to a standard OpenAI-style `POST /v1/chat/completions` integration.


| Approach | Trust assumption | Latency overhead | Cost overhead | Integrity | Client vs OpenAI Chat Completions |
| --- | --- | --- | --- | --- | --- |
| ZK proofs (SNARK/STARK) | None (cryptographic) | Very high (minutes–hours) | Very high | Cryptographic | Non-native: async proof delivery; client verifies `(vk, proof, public_inputs)` locally or on-chain. Request body may need input/output commitments; response carries proof metadata or a separate verify endpoint. |
| TEEs | Hardware vendor | Minimal (~0%) | Low | Hardware-rooted | Often drop-in body; connection layer changes: fetch/verify attestation (`/.well-known/…`, RA-TLS, or SDK preflight), pin measurement/model-root policy, optional E2EE (HPKE/EHBP) before `messages` are sent. |
| Optimistic / fraud proofs | ≥1 honest challenger | Dispute-window delay | Very low | Game-theoretic | Not chat HTTP: client is a smart contract calling `requestCallback(…)`; app reads finalized output from chain/events after the challenge period. Wallet/RPC/ABI required. |
| Behavioural fingerprinting | Statistical / proxy | Negligible | Negligible | Probabilistic | Depends on variant: SVIP needs non-standard response fields (hidden/proxy features); black-box needs `logprobs` + `top_logprobs`; local forensic needs full runtime control (see §2.1). |
| Hybrid (OTR, TEE+ZK) | Mixed | Sub-second (TEE tier) | Low | Layered | TEE client steps plus attestation/receipt on the response; optional on-chain subscription for disputes and sampled ZK checks. |

### 2.1 Client integration (relative to OpenAI Chat Completions)

The OpenAI-compatible surface is the de facto client contract: JSON with `model`, `messages`, sampling parameters, and a streaming or non-streaming completion in `choices[].message.content`. None of the verification approaches listed above are satisfied by that exchange alone; each adds obligations on the client, the provider, or both. The following assumes a client that today points an OpenAI SDK at `https://api.openai.com/v1` (or a compatible proxy).

#### ZK proofs (zkML)

The inference request can remain familiar, but verification is a second phase. The provider (or a prover service) runs the model, then generates a proof that the circuit for model `M` produced output `y` on committed input `x`. The client must:

- Obtain a verification key `vk` for the committed model (published out-of-band, on-chain registry, or IEEE-style model registry [71]).
- Receive `proof` and public inputs (hashes of prompt, output, and sometimes sampled tokens) either in an extended response envelope (non-standard fields such as `proof`, `public_inputs`, `vk_hash`) or via webhook/poll after async proving (typical for zkLLM-scale latencies).
- Run a verifier (local library, zkVerify, or L1/L2 `verify` precompile) before treating the completion as final.

Commitment schemes (TensorCommitments, NANOZK layerwise proofs) add cross-layer hash chaining: the client checks that each layer’s output commitment matches the next layer’s input commitment. There is no widely deployed OpenAI extension for this; Ritual-style stacks expose verification as a separate service path rather than inline in `chat/completions`.

#### Trusted execution environments

TEE-backed products usually keep the chat JSON schema and move trust to the transport and attestation layer.

- Tinfoil-style clients wrap the OpenAI SDK: before or during the first request, the client fetches `/.well-known/tinfoil-attestation`, validates the hardware quote, Sigstore predicate, and TLS/HPKE key binding, and checks the attested model Merkle/dm-verity root against a published commitment for the requested `model` name. Request bodies match OpenAI; EHBP or pinned TLS provides encrypted direct-to-enclave channels.
- Confidential-inference designs (Anthropic/Irregular, Azure AI confidential inferencing) require the client to verify KMS/TEE attestation and policy, obtain an HPKE public key, and encrypt the prompt (often via OHTTP) so only the attested enclave decrypts it. That is not a drop-in OpenAI SDK configuration: the client implements RA-TLS or attested-key HPKE setup before posting completions.
- For GPU CC (H100/Blackwell), the client’s job is policy verification (MRENCLAVE/RTMR allowlists, non-debug enclave) and, where offered, per-response receipts tying the completion to a measurement.

#### Optimistic verification (opML)

ORA-style opML is an on-chain oracle protocol, not an extended chat API. The integrating “client” is a smart contract that inherits `AIOracleCallbackReceiver`, calls `aiOracle.requestCallback(modelId, input, address(this), gasLimit, callbackData)`, and implements `aiOracleCallback(requestId, output, callbackData)` when the result is posted after the challenge window. An off-chain application that wants a chat UX must either (a) submit calldata derived from the user prompt to the contract and wait for the callback event, or (b) use an indexer that watches opML finalization—still fundamentally different from streaming `chat/completions`. Wallets, chain selection, and dispute-window latency are mandatory client concerns.

#### Behavioural fingerprinting

| Variant | Request changes | Response changes | Client verification |
| --- | --- | --- | --- |
| SVIP | Prompt as usual; provider must accept an embedded secret (from a setup phase) | Standard text plus `proxy_task_feature` (compressed hidden-state features), not `choices[].message` alone | User holds secret, labeling network, and proxy head; compares L2 distance to expected label vector (<0.01 s). Requires prior trusted setup to train the proxy on the target model. |
| Black-box fingerprinting | `logprobs: true`, `top_logprobs: n` (supported on OpenAI; often disabled or restricted on third-party APIs) | `choices[].logprobs.content[].top_logprobs` | Offline compatibility test on reconstructed logit vectors; no provider protocol beyond exposing logprobs. |
| Domain watermarking | Usually none if the provider serves a watermark-trained checkpoint | Standard completions | Statistical detection on outputs; client-side or auditor-side tests. |
| Local forensic | Full control of `seed`, runtime, and model artefact paths | N/A (local inference) | Re-run with hashed weights and compare bit-identical outputs. |

SVIP is the furthest from OpenAI compatibility: three-party setup (trusted trainer, provider, user), non-standard response fields, and fixed-length constraints in published evaluations.

#### Refereed delegation (Verde / Gensyn)

The client does not send chat messages to a single `/v1/chat/completions` endpoint. It submits the same ML program (inference job) to multiple compute providers through the Gensyn/Verde network API, with RepOps-enabled operators for bitwise-reproducible execution. The client compares outputs and checkpoint hashes, and initiates Verde dispute arbitration when replicas diverge. Integration is job-spec and SDK-centric (AXL/Chain/REE), not OpenAI message arrays.

#### Hybrid (OTR and similar)

Clients inherit TEE integration (attestation before connect, optional E2EE) and treat the inference response as provisionally final when accompanied by a TEE attestation `\sigma_{TEE}` binding trace, binary, and hardware. For full OTR security, infrastructure clients also run or subscribe to challenger (“Fishermen”) services and occasional ZK spot-check verification on sampled requests—typically operator/indexer roles rather than every end-user app. EigenAI narrows the gap for app developers: OpenAI-compatible `/v1/chat/completions` with extra response fields (`receipt.req_hash`, `receipt.out_hash`, `receipt.sig`, `eigendalink`, `system_fingerprint`, deterministic `seed`) and optional request signing/encryption to the operator key; verification replays the request against EigenDA with deterministic decoding metadata.

---



## 3. Zero-Knowledge Machine Learning (zkML)

### 3.1 Foundations

Zero-knowledge proofs (ZKPs) allow a prover  \mathcal{P}  to convince a verifier  \mathcal{V}  that a statement is true without revealing anything beyond the statement's validity. Applied to ML inference, the goal is to prove: *"I ran model  \mathcal{M}  with parameters  \theta  on input  x  and obtained output  y ,"* without disclosing  \theta ,  x , or any intermediate activations unless desired.[10][11]

Succinctness matters for deployment: verification of a proof must be significantly cheaper than re-running the computation. zk-SNARKs (Succinct Non-Interactive Arguments of Knowledge) and zk-STARKs (Scalable Transparent Arguments of Knowledge) are the two dominant proof systems, with SNARKs offering smaller proofs and STARKs offering transparency without a trusted setup.[12][11]

Neural network inference presents unique challenges for ZK circuits:

1. Non-arithmetic operations: Softmax, GELU, LayerNorm, and ReLU do not naturally encode as finite-field arithmetic circuits
2. Scale: A 70B parameter model involves billions of multiply-accumulate operations, all of which must be arithmetised
3. Floating-point: ZK circuits operate over finite fields; IEEE 754 floating-point must be simulated or replaced with fixed-point arithmetic[13]

### 3.2 Key Academic Papers

#### zkCNN (2021)

Liu, Xie, and Zhang's zkCNN (ACM CCS 2021) applied GKR interactive proofs to convolutional neural networks. The work introduced techniques to handle the sumcheck protocol for convolution operations and established the theoretical basis for subsequent LLM-scale work.[14]

#### zkLLM (2024)

Sun, Li, and Zhang (arXiv:2404.16109) present zkLLM, a ZKP scheme for LLMs. Main components:[15]

- tlookup: A parallelised lookup argument for non-arithmetic tensor operations in deep learning, providing a solution with no asymptotic overhead. It is designed for the GPU parallel execution model.[16]
- zkAttn: A specialised ZKP for the attention mechanism, balancing running time, memory usage, and accuracy.[17]

With a fully parallelised CUDA implementation, zkLLM can generate a correctness proof for a 13B parameter LLM inference in under 15 minutes, producing a proof smaller than 200 KB. This marked the transition from toy-scale models to transformer-scale verification, though 15 minutes per query remains far from real-time.[15][16]

#### NANOZK (2026)

Wang's NANOZK (arXiv:2603.18046, March 2026) proves inference layerwise: transformer forward passes decompose into independent layers, each with a constant-size proof (5.5 KB/layer: 2.1 KB attention, 3.5 KB MLP; 24 ms verification).[5][3]

Lookup tables approximate softmax, GELU, and LayerNorm without reported accuracy loss; Fisher-information-guided selective verification applies when full-layer proofs are too costly. At depth 128, NANOZK reports 70× smaller proofs and 5.7× faster proving than EZKL, with \epsilon < 10^{-37} soundness.[5]

#### TensorCommitments (2026)

A concurrent approach from Zeng et al. (arXiv:2602.12630, February 2026) proposes TensorCommitments (TCs), a tensor-native proof-of-inference scheme. TCs bind LLM inference to a commitment — an irreversible tag organised via multivariate Terkle Trees — that breaks under tampering. For LLaMA 2, TC adds only 0.97% prover overhead and 0.12% verifier time over baseline inference while improving robustness to tailored LLM attacks by up to 48% over prior work.[1]

#### Lightweight Cryptographic Proofs of Inference (2026)

Pal et al. (March 2026) from Ritual present a sampling-based framework that replaces full cryptographic proofs with statistical properties of neural network execution traces. The prover commits to the inference trace via Merkle-tree vector commitments and opens only a small number of entries along randomly sampled paths from output to input. Soundness is traded for cost: proving drops from minutes to milliseconds, useful when repeated queries raise detection probability.[18]

### 3.3 Circuit Frameworks and Tooling

#### EZKL

EZKL (zkonduit) is the most widely used zkML toolchain for ONNX → Halo2 circuits. It accepts ML models in ONNX format, automatically converts them into ZKP-compatible circuits using the Halo2 proof system, and generates proofs of correct model execution. The three core use cases are: public model on private data, private model on public data, and public model on public data (particularly for blockchain co-processors). As of 2025-2026, EZKL proves a linear regression in 0.118 seconds on commodity hardware; larger models on A100 GPUs cost USD 0.02–0.50 per proof.[19][20][12]

#### DeepProve (Lagrange Labs)

DeepProve (Lagrange Labs) uses sumcheck and logup GKR for sublinear proving on MLPs and CNNs in ONNX. Published benchmarks claim 54–158× faster proving and 671× faster verification than EZKL.[21][22][23]

#### zkPyTorch / Expander (Polyhedra Network)

Polyhedra's zkPyTorch compiler (March 2025) lowers PyTorch/ONNX models to ZK circuits. Expander, the backend, uses GKR with polynomial commitments at linear prover time. Benchmark performance on their architecture shows VGG-16 (15M parameters) at approximately 2.2 seconds per image proof; Llama-3 (8B parameters) at approximately 150 seconds per token on single-core CPU. The Expander system achieves up to 9,000 ZK proofs per second with CUDA 13.0 and GPU-accelerated KZG commitments.[24][25][26]

#### JSTprove (Inference Labs)

JSTprove (arXiv:2510.21024, October 2025) wraps Expander with CLI tooling and auditable artefacts for verifiable inference pipelines.[27][28]

#### JOLT Atlas (a16z / Kinic)

JOLT is a zkVM (zero-knowledge virtual machine) targeting the RISC-V instruction set, developed by a16z. The lookup-centric approach converts CPU instructions into lookups in large pre-defined tables via the Lasso lookup argument, rather than heavy algebraic constraint systems per operation. JOLT Atlas adapts this for ML operations, enabling arbitrary ML programs compiled to RISC-V to be proved — at the cost of universal VM overhead compared to circuit-specific systems.[29][30][13]

---

## 4. Trusted Execution Environments (TEEs)

### 4.1 Architecture and Attestation

A Trusted Execution Environment is a hardware-isolated enclave within a processor (or GPU) that provides three guarantees: data confidentiality, data integrity, and code integrity. The key mechanism enabling verifiable inference is remote attestation — a cryptographic protocol by which the TEE generates a signed report proving:[31]

1. The hardware is a genuine TEE (using manufacturer-embedded keys)
2. The TEE is running specific, unmodified code (measured as a hash of the loaded binary)[32][33]

For AI inference, this means a user can receive a cryptographic attestation proving: *"NVIDIA H100 hardware, running binary hash X, processed your query."* If binary X corresponds to the inference code for the claimed model weights, the user has hardware-rooted assurance of execution integrity.[34][31]

### 4.2 GPU-Level Confidential Computing

CPU-only TEEs are insufficient for AI inference because weights and activations live in GPU memory — outside the CPU enclave boundary. An attacker with GPU memory access can extract weights. The NVIDIA H100 GPU introduced the first GPU-level confidential computing support, extending the TEE boundary to include the accelerator itself. It encrypts GPU execution memory, register states, model weights, the KV cache, and outputs, providing hardware attestation for GPU workloads.[35][36][37][31]

The Anthropic / Irregular (Pattern Labs) joint whitepaper on Confidential Inference Systems (June 2025) formally describes the design principles and security considerations for confidential inference, noting that:

- The threat model must assume the service provider is adversarial with full machine control
- CPU-only TEEs are insufficient — the confidential boundary must extend to the GPU accelerator
- NVIDIA H100 and Blackwell GPUs ship with native TEE support, enabling encrypted weight loading, protected in-flight computation, and encrypted output egress[38][33]

Anthropic's architecture involves: encrypting the request before it reaches Anthropic servers; decrypting, processing, and re-encrypting within the trusted loader; and never releasing model weights from the enclave.[33]

### 4.3 TEE platforms


| Platform            | Vendor | GPU Support           | Status              |
| ------------------- | ------ | --------------------- | ------------------- |
| Intel SGX           | Intel  | No (CPU only)         | Production          |
| Intel TDX           | Intel  | Via NVIDIA CC pairing | Production          |
| AMD SEV-SNP         | AMD    | Via NVIDIA CC pairing | Production          |
| ARM TrustZone / CCA | ARM    | No (edge focus)       | Production/Research |
| NVIDIA H100 CC      | NVIDIA | Native                | Production          |
| NVIDIA Blackwell    | NVIDIA | Native (enhanced)     | Production          |


The 2026 survey paper "When Agents Handle Secrets" provides a unified taxonomy covering all six major TEE platforms, analysing deployment roles and performance trade-offs for agentic AI specifically.[39]

### 4.4 Modelwrap (Tinfoil)

Tinfoil's Modelwrap (February 2026) binds model identity to TEE measurement:[40][41]

1. Public Merkle root over weight files
2. Boot-time attestation (Linux `dm-verity` recomputes block hashes on read; mismatch fails I/O)
3. Client verification of attestation root against the published commitment

Each response can carry a receipt tying the attestation to the weight commitment.[41]

### 4.5 Limitations of TEEs

TEEs shift trust from the inference provider to the hardware manufacturer (Intel, AMD, NVIDIA). A compromised or backdoored chip manufacturer could theoretically defeat attestation. Additionally, side-channel attacks (power analysis, cache timing) remain a theoretical vector, with RAND noting that current GPU TEEs lack physical attack protections. The Confidential Computing Transparency Framework (arXiv:2409.03720) explicitly notes that attestation cannot guarantee the absence of vulnerabilities or backdoors in TEE firmware.[42][43][31]

---

## 5. Optimistic Verification (opML)

### 5.1 Protocol Overview

opML (Optimistic Machine Learning), introduced by Hyper Oracle (ORA) in 2023 (arXiv:2401.17555), adapts the optimistic rollup design pattern to ML inference. The protocol operates as follows:[44][45]

1. A submitter executes ML inference natively (with full hardware acceleration) and posts results on-chain
2. The result is assumed correct by default (optimistic assumption)
3. Validators check results; if a discrepancy is found, a bisection dispute game is initiated
4. The game binary-searches the computation trace to isolate a single disputed instruction
5. A smart contract arbitrates only that single instruction, making on-chain adjudication cheap[46][47]

The core components are a Fraud Proof Virtual Machine (FPVM) that can execute individual ML operations and expose them to on-chain verification, and a Machine Learning Engine (MLE) for efficient off-chain execution. Memory is managed as a Merkle tree, enabling efficient single-instruction state proofs.[47][45]

### 5.2 Advantages and Limitations

opML can run 7B LLaMA models on standard PCs without GPUs, requires no expensive proof generation per request, and is cost-effective for large model scales. However, the dispute window — analogous to the 7-day challenge period in Ethereum optimistic rollups — prevents real-time or sub-second finality. This makes opML unsuitable for interactive applications, DeFi, or real-time AI agents. Security also requires at least one honest challenger to be monitoring and willing to dispute.[48][7][44]

### 5.3 zk-OPML Hybrid

A 2026 Springer paper (zk-OPML) explores combining ZK proofs with the opML framework to reduce the challenge period by providing probabilistic ZK spot-checks during the dispute window. This trades marginal ZK proving cost for significantly shortened settlement times.[49]

---

## 6. Behavioural Fingerprinting and Statistical Verification

### 6.1 SVIP: Hidden Representation Verification

SVIP (arXiv:2410.22307, NeurIPS 2025; Secret-based Verifiable Inference Protocol) requires the provider to return generated text and selected hidden activations from internal transformer layers.[4][50]

A small proxy task model is trained exclusively on the hidden states of the specified model (e.g., Llama 3.1 70B). If the provider runs the correct model, the proxy task performs well on the returned hidden states; if a cheaper model was substituted, the proxy task fails to generalise to those states.[51][4]

The verifier holds undisclosed probe queries so the provider cannot tailor activations to pass a fixed check.[52][51]

Results across models from 13B to 70B parameters: false negative rate below 5%, false positive rate below 3%, and verification latency of less than 0.01 seconds per query. The primary limitation is that SVIP requires the provider to expose hidden representations, which may not be supported by all APIs, and is most robust for fixed-length inputs.[4][51]

### 6.2 LLM Fingerprinting

Black-box fingerprinting techniques (arXiv:2407.01235, 2024) exploit the fact that the output distribution of an LLM spans a unique vector space associated with each model. By reconstructing the vector space from API responses (using top-k probabilities or full vocabulary logits), an auditor can perform a compatibility test to determine whether a suspect model's outputs lie in the same space as the target model's outputs. This requires no model internals, only the probability distribution over output tokens.[53]

Domain-specific watermarking for model fingerprinting (ICML 2025) introduces the idea of training a model to embed output watermarks only within specified subdomains (particular languages, topics, or code styles). This provides strong statistical guarantees for model provenance, with controllable false positive rates and robustness to real-world deployment variability, fine-tuning, and post-processing.[54]

### 6.3 Deterministic Forensic Framework

A practical forensic approach for local LLM inference (IEEE 2026) achieves 100% bit-identical reproducibility across 40 controlled inference runs by recording cryptographic hashes of model artefacts, capturing environmental metadata, and logging inference-time seeds. A separate verification process instantiates a clean runtime, reloads the hashed model, and performs bit-for-bit comparison to produce forensic pass/fail reports. This approach is primarily applicable to open-weight models in controlled environments.[55]

---

## 7. Refereed Delegation (Gensyn Verde)

### 7.1 Protocol Design

Verde (arXiv:2502.19405, February 2025, developed by Gensyn) introduces the cryptographic notion of refereed delegation adapted to the ML setting. Rather than a single trusted provider, a computationally limited client delegates the same program to multiple untrusted compute providers. If at least one provider is honest, the client obtains the correct result.[56][57]

The two technical innovations enabling this are:

1. Dispute Arbitration (Verde Protocol): When providers disagree, a dispute game pinpoints the exact operator in the neural network's computational graph where results first diverge. The referee (which can be a smart contract or jury of verifiers) recomputes only that single disputed operator, not the entire computation. This reduces verification cost from  O(N)  to  O(1)  dispute adjudication.[58][59]
2. RepOps (Reproducible Operators): A library implementing bitwise-reproducible versions of ML operators by enforcing a fixed execution order of floating-point operations. This solves the inherent hardware non-determinism of GPU computation (CUDA kernel scheduling, floating-point associativity) that would otherwise prevent honest nodes from producing identical outputs on different hardware.[60][59][56]

Verde is the foundation of Gensyn's mainnet (launched April 22, 2026), supporting models up to 72B parameters with bitwise-consistent outputs across hardware.[60]

### 7.2 Comparison to zkML

Verde trades cryptographic soundness for practical efficiency. Unlike ZK proofs — which provide mathematical certainty with no honest-party assumption — Verde's guarantees hold only if at least one participating node is honest. Verde adds only checkpoint hashing on the compute path—no dedicated proving hardware.[59]

---

## 8. Hybrid Architectures

### 8.1 Optimistic TEE-Rollups (OTR)

DGrid AI's Optimistic TEE-Rollups (OTR; arXiv:2512.20176, December 2025) stack three verification layers:[7][6]

1. Provisional finality: TEE-signed attestation \sigma_{TEE} with MRENCLAVE (or equivalent) binding trace, binary, and hardware; sub-second.[9]
2. Optimistic fallback: decentralised challengers ("Fishermen") dispute attestations.[9]
3. Stochastic ZK spot-checks at rate \rho < 0.01 on sampled transactions.[9]

Proof of Efficient Attribution (PoEA) ties payment to the weight set loaded in the enclave. Reported simulations: ~99% of centralised throughput, ~0.07 s inference latency.[61][8]

### 8.2 Ritual (Privacy-Preserving + Verifiable Inference)

Ritual (arXiv:2602.17223, February 2026) reuses privacy-preserving inference primitives (SMPC, FHE) for verified inference at small marginal cost. Infernet and Ritual Chain expose ZKML, optimistic proofs, and TEE-backed paths behind a common interface.[62][63][64]

### 8.3 Conan: Accuracy Verification + Inference Integrity

Conan (IEEE 2025, arXiv:2503.07466) addresses a subtler problem: even if inference was run on the correct model, the provider might use a model that was fine-tuned to reduce quality while maintaining architecture. Conan requires the server to first commit to the model and prove in zero-knowledge that the committed model achieves the claimed accuracy, and then perform secure inference on that committed model using maliciously-secure two-party computation (2PC) protocols. This simultaneously achieves accuracy verification, inference integrity, and input/output privacy.[65][66]

---

## 9. Fully Homomorphic Encryption (FHE)

FHE allows computation over encrypted data — a client could submit an encrypted query and receive an encrypted output, with the server never seeing plaintext. EncryptedLLM (ICML 2025, arXiv) presents a GPU-accelerated CKKS-based FHE implementation for GPT-2 forward passes, achieving runtimes over 200× faster than CPU baselines. Full LLM inference via FHE remains impractical at production scale (bootstrapping, ciphertext expansion). For near-term confidential inference, TEE paths dominate; FHE is primarily a research track.[67][68][69]

---

## 10. Standardisation and Governance

### 10.1 IETF Draft: AI Model Lifecycle Attestation

An IETF Internet-Draft (draft-sharif-ai-model-lifecycle-attestation-00, February 2026) proposes a standards-track protocol for cryptographic attestation across the full AI model lifecycle — from training data provenance through inference output. If ratified, this would provide interoperability between attestation implementations across vendors and enable regulatory compliance based on verifiable provenance chains.[70]

### 10.2 Open-Source Model Registry (IEEE 2025)

A proposed AI model registry (IEEE, November 2025) uses Ethereum and IPFS to provide on-chain model registration with dual verification paths: ZKML for strong cryptographic guarantees and opML via dispute resolution for efficiency. The system enables users to confirm both model identity and execution correctness through a single registry lookup.[71]

### 10.3 Industry Positions

The major AI labs have published their own positions:

- Anthropic / Irregular: Joint whitepaper on Confidential Inference via Trusted VMs (June 2025), explicitly noting CPU-only TEEs are insufficient and requiring GPU-level confidential computing[38][31][33]
- OpenAI: May 2024 post "Reimagining Secure Infrastructure for Advanced AI" called for confidential computing primitives to extend beyond CPU hosts into AI accelerators[31]
- Google DeepMind: Frontier Safety Framework (May 2024) lists TPUs with confidential compute capabilities at its highest security level[31]

---

## 11. Decentralised Infrastructure Layer

Several networks expose verifiable inference as infrastructure rather than a bilateral API feature:

### EigenLayer AVS

EigenLayer's EigenAI (mainnet live late 2025) provides an AVS (Actively Validated Service) for LLM inference where operators post ETH restaking collateral as a slash-able economic bond. Every input, output, and model version is cryptographically guaranteed through on-chain attestation. EigenCompute (January 2026 mainnet alpha) handles off-chain execution verification as a separate AVS. By 2026, over 280 crypto-AI projects require trust-minimised model evaluation, making AI verification the largest AVS category.[72][73]

### Allora Network

Allora uses zkML to allow deployed models to supply predictions alongside ZK proofs that the value was generated by the claimed algorithm — without revealing model weights — enabling permissionless, verifiable AI oracles for DeFi and governance.[74]

### Gensyn

Gensyn's three-layer architecture (AXL for communication, Chain for identity and settlement, REE for authentication) uses Verde and RepOps to provide a fully decentralised ML network where any third party can reproduce and verify any computation.[75][60]

---

## 12. Performance Benchmarks and Practical Trade-offs

### Proving time by model scale (2025–2026)


| System               | Model Scale           | Proving Time          | Proof Size            | Approach            |
| -------------------- | --------------------- | --------------------- | --------------------- | ------------------- |
| EZKL                 | Linear regression     | 0.12 seconds          | ~KB                   | SNARK (Halo2)       |
| DeepProve            | CNN                   | ~milliseconds         | ~KB                   | GKR sumcheck        |
| zkLLM                | 13B params            | ~15 minutes           | < 200 KB              | CUDA tlookup        |
| NANOZK               | per transformer layer | 24 ms verification    | 5.5 KB/layer          | Layerwise ZK        |
| zkPyTorch (Expander) | 8B params (Llama-3)   | ~150 sec/token        | —                     | GKR                 |
| SVIP                 | Any open-weight       | < 0.01 sec            | (API overhead)        | Statistical proxy   |
| Verde / Gensyn       | Up to 72B             | Near-zero overhead    | Checkpoint hashes     | Refereed delegation |
| TEE (H100 CC)        | Any (hardware-bound)  | ~0% overhead          | Attestation report    | Hardware            |
| OTR                  | Any LLM               | Sub-second (TEE tier) | Attestation + spot ZK | Hybrid              |


### Cost Benchmarks

On-chain ZK proof verification adds a fixed Ethereum gas cost of approximately 200,000 to 600,000 gas units per proof, translating to roughly USD 3–18 per verification call at current mainnet prices. A zkML MVP (lightweight model, testnet deployment, end-to-end integration) typically requires 4–6 weeks and USD 40,000–80,000 depending on optimisation depth.[20]

---

## 13. Open Research Problems

Open problems:

1. Billion-parameter ZK proving: No current system can generate a ZK proof for a 70B+ model inference in real-time. The NANOZK layerwise decomposition and lightweight trace commitment schemes represent the frontier but have not yet been demonstrated at GPT-4 scale.
2. Hardware non-determinism: Floating-point non-determinism across GPU hardware makes bit-identical reproducibility a hard engineering constraint. RepOps addresses this but at operator-level granularity with custom CUDA kernels — not universally applicable.
3. Compound attestation for agent chains: As AI operates as multi-hop agent pipelines, the attestation problem compounds — each hop's correctness must be proven, and the chain of attestations must be efficiently aggregated. The survey "When Agents Handle Secrets" identifies this as a major open challenge.[39]
4. Side-channel attacks on GPU TEEs: Current H100 Confidential Computing does not protect against physical attacks, and RAND's SL4 requirements are not fully met. The Anthropic whitepaper acknowledges fallback architectures bridging CPU enclaves to GPUs as non-airtight.[31]
5. Model commitment binding at fine-tuning: If a provider fine-tunes from a base model, the committed base model hash no longer corresponds to the running weights. Auditable Fine-Tuning and Inference for Proprietary AI (arXiv:2603.07466, March 2026) addresses this by requiring providers to publicly release base model hash commitments before fine-tuning contracts are established and providing recomputation-based verification protocols for fine-tuned deltas.[76]
6. Non-deterministic sampling: LLMs sample stochastically at generation time. A ZK proof of inference proves the forward pass given a fixed sequence of sampled tokens, not the distribution. The prover must also commit to the sampling randomness — a non-trivial extension.

---

## 14. Choosing an approach

| Constraint | Direction |
| --- | --- |
| Cryptographic proof required; minutes of proving acceptable | EZKL, zkPyTorch, or zkLLM for models ≲13B; run proving off the request path |
| Near-native latency; trust hardware vendor | H100/Blackwell CC with remote attestation; weight commitments (e.g. Modelwrap); Anthropic/Irregular confidential-inference design |
| On-chain enforcement; economic slashing | EigenLayer EigenAI AVS; opML where dispute-window latency is acceptable |
| API-scale statistical check; open weights | SVIP-style hidden-state probes (`logprobs` or custom fields per §2.1) |
| Decentralised compute; large models; no ZK prover fleet | Gensyn Verde + RepOps (≥1 honest replica) |
| Regulated / high-stakes; layered assurance | OTR: TEE finality + challengers + sparse ZK audits |

---

## Conclusion

Between 2024 and 2026, several results moved verifiable inference from prototypes toward deployable components: zkLLM at 13B scale, NANOZK's per-layer proofs, Verde on Gensyn mainnet, and GPU confidential-compute designs from Anthropic and Irregular.[77][3][38][5]

No single mechanism satisfies the Verifiability Trilemma. Frontier-scale ZK proving is not yet on the inference latency path; TEEs import trust in the silicon vendor; optimistic and refereed schemes need at least one honest participant. Hybrids such as OTR combine attestation, economics, and sampled ZK.[6][8]

Governance pressure (NIST, EU AI Act) and audit requirements in finance, healthcare, and legal workflows are driving attestation standards (IETF draft-sharif-ai-model-lifecycle-attestation) and vendor alignment on GPU CC.[70][31]

---

*State of the art as of May 2026. Primary references: [EZKL](https://docs.ezkl.xyz), [DeepProve](https://lagrange.dev/deepprove), [Verde](https://arxiv.org/abs/2502.19405), [NANOZK](https://arxiv.org/abs/2603.18046), [zkLLM](https://arxiv.org/abs/2404.16109), [Anthropic confidential inference](https://www.anthropic.com/research/confidential-inference-trusted-vms), [awesome-zkml](https://github.com/worldcoin/awesome-zkml).*

---

## References

1. [TensorCommitments: A Lightweight Verifiable Inference for Language Models](https://arxiv.org/abs/2602.12630) - Most large language models (LLMs) run on external clouds: users send a prompt, pay for inference, an...
2. [The Inference Interference - Everything Bagel](https://blog.bagel.com/p/the-inference-interference) - The solution to this is Verifiable inference, using mechanisms to verify the use of specific models ...
3. [NANOZK: Layerwise Zero-Knowledge Proofs for Verifiable Large Language Model Inference](https://www.semanticscholar.org/paper/afc78715fb0f09fca40f206d45fdd353de7ba47b) - When users query proprietary LLM APIs, they receive outputs with no cryptographic assurance that the...
4. [SVIP: Towards Verifiable Inference of Open-source Large Language Models](https://arxiv.org/abs/2410.22307) - The ever-increasing size of open-source Large Language Models (LLMs) renders local deployment imprac...
5. [NANOZK: Layerwise Zero-Knowledge Proofs for Verifiable ... - arXiv](https://arxiv.org/abs/2603.18046) - We present METHOD, a zero-knowledge proof system that makes LLM inference verifiable: users can cryp...
6. [Optimistic TEE-Rollups: A Hybrid Architecture for Scalable and Verifiable Generative AI Inference on Blockchain](https://arxiv.org/abs/2512.20176) - The rapid integration of Large Language Models (LLMs) into decentralized physical infrastructure net...
7. [Optimistic TEE-Rollups: Solving the Verifiability Trilemma ... - DGrid AI](https://blog.dgrid.ai/posts/2026-01-14/) - ... integrity, sub-second latency, and near-native cost. The Verifiability Trilemma: Why Decentraliz...
8. [Tee-rollups Enable 0.07 Second Blockchain Inference](https://quantumzeitgeist.com/blockchain-models-tee-rollups-enable-second-inference-large-language/) - This trilemma states that systems struggle to simultaneously achieve high computational integrity, l...
9. [[論文評述] Optimistic TEE-Rollups: A Hybrid Architecture for Scalable ...](https://www.themoonlight.io/tw/review/optimistic-tee-rollups-a-hybrid-architecture-for-scalable-and-verifiable-generative-ai-inference-on-blockchain) - Optimistic TEE-Rollups (OTR) is a novel hybrid verification protocol designed to address the Verifia...
10. [Zero-knowledge Machine Learning (ZKML) - Introduction to Nesa](https://docs.nesa.ai/nesa/major-innovations/private-inference-for-ai/background-and-exploratory-notes/software-algorithm-side-model-verification/zero-knowledge-machine-learning-zkml) - For model verification, P \mathcal{P} P demonstrates that M \mathcal{M} M executed correctly on inpu...
11. [An introduction to zero-knowledge machine learning (ZKML)](https://world.org/blog/engineering/intro-to-zkml) - A zero-knowledge (ZK) proof is a cryptographic protocol in which one party, the prover, can prove to...
12. [ZKML: Verifiable Machine Learning using Zero-Knowledge Proof](https://kudelskisecurity.com/modern-ciso-blog/zkml-verifiable-machine-learning-using-zero-knowledge-proof) - ZKML offers a solution by providing cryptographic proofs that the training procedures were executed ...
13. [JOLT Atlas Reaching For SOTA In Zero Knowledge Machine ... - Kinic](https://www.kinic.io/blog/joltx-reaching-for-sota-in-zero-knowledge-machine-learning-zkml) - When we talk scalability in zkML, we could mean two things: handling large models (many layers, many...
14. [zkCNN: Zero Knowledge Proofs for Convolutional Neural Network ...](https://dl.acm.org/doi/10.1145/3460120.3485379) - zkCNN: Zero Knowledge Proofs for Convolutional Neural Network Predictions and Accuracy. Security and...
15. [zkLLM: Zero Knowledge Proofs for Large Language Models - arXiv](https://arxiv.org/abs/2404.16109) - Addressing the persistent challenge of non-arithmetic operations in deep learning, we introduce tloo...
16. [[PDF] zkLLM: Zero Knowledge Proofs for Large Language Models](https://hongyanz.github.io/publications/CCS_zkLLM.pdf) - We propose tlookup, a unique ZKP protocol for universal non-arithmetic operations in deep learning, ...
17. [[Literature Review] zkLLM: Zero Knowledge Proofs for Large ...](https://www.themoonlight.io/en/review/zkllm-zero-knowledge-proofs-for-large-language-models) - The paper introduces "zkLLM," a novel framework for generating zero-knowledge proofs (ZKPs) tailored...
18. [Towards Verifiable AI with Lightweight Cryptographic Proofs of Inference](https://www.semanticscholar.org/paper/a5162ebbe07d08cafb655c3bec79bfe6f5d659aa) - When large AI models are deployed as cloud-based services, clients have no guarantee that responses ...
19. [The EZKL System](https://docs.ezkl.xyz) - EZKL is a developer-friendly system for verifiable AI and analytics. Analytics can be descriptive (a...
20. [zkML Proof Generation Costs: Benchmark Analysis 2026 - Ancilar](https://www.ancilar.com/knowledge-hub/blogs/zkml-proof-generation-costs-benchmark-analysis-for-on-chain-ai-models-in-2026) - EZKL proves a linear regression in 0.118 seconds (EZKL Benchmarks, 2025). Operators who separate the...
21. [GitHub - Lagrange-Labs/deep-prove](https://github.com/Lagrange-Labs/deep-prove) - Welcome to DeepProve, a cutting-edge framework designed to prove neural network inference using zero...
22. [DeepProve — Verifiable AI Inference & zkML | Lagrange](https://lagrange.dev/deepprove) - Lagrange's DeepProve generates cryptographic proofs for AI inferences — up to 158× faster than the l...
23. [Lagrange's DeepProve - A Pioneer in AI Verification for - Binance](https://www.binance.com/en/square/post/28050527452217) - DeepProve of @Lagrange Official supports multi-layer perceptrons (MLP) and convolutional neural netw...
24. [Polyhedra Boosts zkML with Expander: 9000 zk Proofs Per Second](https://coinfomania.com/polyhedra-expander-upgrade-zkml-performance/) - Polyhedra upgrades its Expander system with CUDA 13.0 support, 1 TB/s bandwidth, and GPU-accelerated...
25. [Polyhedra Introduces a Breakthrough in AI Trust Infrastructure](https://www.prnewswire.com/news-releases/polyhedra-introduces-a-breakthrough-in-ai-trust-infrastructure-302411421.html) - Polyhedra is building foundational infrastructure for trust and scalability in AI and blockchain sys...
26. [Expander - Polyhedra Network](https://www.polyhedra.network/expander) - Expander is open-source and continuously evolving, delivering unmatched performance to help develope...
27. [JSTprove: Pioneering Verifiable AI for a Trustless Future - arXiv](https://arxiv.org/html/2510.21024v1) - DeepProve [23] is a zkML framework that supports multi-layer perceptrons (MLPs) and convolutional ne...
28. [Daily Papers - Hugging Face](https://huggingface.co/papers?q=verifiable+AI+inference) - In this paper, we introduce JSTprove, a specialized zkML toolkit, built on Polyhedra Network's Expan...
29. [A Technical Dive into Jolt: The RISC-V zkVM - ZK/SEC Quarterly](https://blog.zksecurity.xyz/posts/how-jolt-works/) - In our latest post, we take you inside the workings of Jolt, a zero-knowledge virtual machine for th...
30. [GitHub - a16z/jolt: The simplest and most extensible zkVM. Fast and ...](https://github.com/a16z/jolt) - This repository currently contains an implementation of Jolt for the RISC-V 64-bit Base Integer Inst...
31. [Confidential Computing for AI Workloads - Elsie's Blog - Substack](https://elsiejang1.substack.com/p/confidential-computing-for-ai-workloads) - Confidential computing resolves this by running inference inside a hardware-secured TEE. The hardwar...
32. [What Is a Trusted Execution Environment (TEE)? - Chainlink](https://chain.link/article/trusted-execution-environment-tee) - A Trusted Execution Environment (TEE) is a secure area of a main processor that guarantees code and ...
33. [Confidential Inference via Trusted Virtual Machines - Anthropic](https://www.anthropic.com/research/confidential-inference-trusted-vms) - Confidential Inference is a set of tools we can use to process encrypted data and to show that such ...
34. [Enhancing AI inference security with confidential computing](https://next.redhat.com/2025/10/23/enhancing-ai-inference-security-with-confidential-computing-a-path-to-private-data-inference-with-proprietary-llms/) - TEEs use hardware-backed cryptographic technologies to create an isolated, verified execution enviro...
35. [Confidential Computing on NVIDIA H100 GPUs for Secure and ...](https://developer.nvidia.com/blog/confidential-computing-on-h100-gpus-for-secure-and-trustworthy-ai/) - The NVIDIA H100 Tensor Core GPU is the first ever GPU to introduce support for confidential computin...
36. [AI Security with Confidential Computing - NVIDIA](https://www.nvidia.com/en-us/data-center/solutions/confidential-computing/) - It protects GPU execution, memory, and register states while keeping models, training data, and infe...
37. [NVIDIA GPU Confidential Computing Demystified - arXiv](https://arxiv.org/html/2507.02770v1) - In this paper, we aim to demystify the implementation of NVIDIA GPU-CC system by piecing together th...
38. [Irregular and Anthropic Publish Whitepaper on Confidential AI ...](https://www.irregular.com/publications/confidential-inference-systems) - TEEs provide cryptographic attestation to verify code integrity, allowing parties to confirm the com...
39. [When Agents Handle Secrets: A Survey of Confidential Computing for Agentic AI](https://www.semanticscholar.org/paper/56a58fe4bfc6b21c9fb9f552b7c3c7e9a70228e2) - Agentic AI systems, specifically LLM-driven agents that plan, invoke tools, maintain persistent memo...
40. [Modelwrap: Cryptographic Assurance of AI Model Identity - UBOS](https://ubos.tech/news/modelwrap-cryptographic-assurance-of-ai-model-identity/) - Modelwrap combines three proven techniques to create an end‑to‑end guarantee of model integrity: Pub...
41. [How Tinfoil Proves Exactly What Model Is Running](https://tinfoil.sh/blog/2026-02-03-proving-model-identity) - With Modelwrap we end up attesting two things: The cryptographic commitment to the model weights (a ...
42. [Confidential Computing for AI: TEEs, Attestations, and Limits](https://blogcpp.org/confidential-computing-for-ai-tees-attestations-and-limits) - Confidential computing now encompasses GPU-enabled Trusted Execution Environments (TEEs) which provi...
43. [A Confidential Computing Transparency Framework for a Comprehensive
  Trust Chain]([http://arxiv.org/pdf/2409.03720.pdf](http://arxiv.org/pdf/2409.03720.pdf)) - Confidential Computing enhances privacy of data in-use through hardware-based

Trusted Execution Envi...

1. [opML: Optimistic Machine Learning on Blockchain](https://arxiv.org/abs/2401.17555) - The integration of machine learning with blockchain technology has witnessed increasing interest, dr...
2. [opML - ORA](https://docs.ora.io/doc/onchain-ai-oracle-oao/fraud-proof-virtual-machine-fpvm-and-frameworks/opml) - opML (Optimistic Machine Learning) is an innovative framework that enables efficient and scalable ma...
3. [opML - Projects & Protocols - IQ.wiki](https://iq.wiki/wiki/opml) - This framework enhances transparency and fosters trust in machine learning inference by allowing for...
4. [[Literature Review] opML: Optimistic Machine Learning on Blockchain](https://www.themoonlight.io/en/review/opml-optimistic-machine-learning-on-blockchain) - opML uses a fraud-proof system instead of zero-knowledge proofs to guarantee the correctness of ML r...
5. [opML: A Method for Verifying Machine Learning Output Using ...](https://www.reddit.com/r/learnmachinelearning/comments/17ba11a/opml_a_method_for_verifying_machine_learning/) - Objective: opML aims to port AI model inference and training/fine-tuning into blockchain using an op...
6. [zk-OPML: Using zero-knowledge proofs to optimize OPML](https://link.springer.com/10.1007/s44443-026-00573-1)
7. [SVIP: Towards Verifiable Inference of Open-source Large Language...](https://openreview.net/forum?id=OaKEGJLhP9) - TL;DR: We propose SVIP, a secret-based protocol for verifiable LLM inference using hidden representa...
8. [SVIP: Towards Verifiable Inference of Open-Source Large ... - GitHub](https://github.com/ASTRAL-Group/SVIP_LLM_Inference_Verification) - This repository contains the implementation of SVIP, a secret-based verifiable LLM inference protoco...
9. [State of Verifiable Inference & Future Directions - Equilibrium Labs](https://equilibrium.co/writing/state-of-verifiable-inference) - Verifiable inference enables proving the correct model and weights were used, and that inputs/output...
10. [A Fingerprint for Large Language Models - arXiv](https://arxiv.org/html/2407.01235v1) - This paper to propose a novel black-box fingerprinting technique for LLMs, which requires neither mo...
11. [Robust LLM Fingerprinting via Domain-Specific Watermarks](https://icml.cc/virtual/2025/50958) - Our evaluations show that domain-specific watermarking enables model fingerprinting with strong stat...
12. [Deterministic LLMs: a Practical Forensic Framework for Verifiable and Reproducible Local LLM Inference](https://ieeexplore.ieee.org/document/11458980/) - Large Language Models (LLMs) have a growing role in legal, investigative, and enterprise workflows, ...
13. [Verde: Verification via Refereed Delegation for Machine Learning Programs](https://arxiv.org/abs/2502.19405) - Machine learning programs, such as those performing inference, fine-tuning, and training of LLMs, ar...
14. [Verde: Verification via Refereed Delegation for Machine Learning ...](https://arxiv.org/html/2502.19405v1) - We design Verde, a dispute arbitration protocol that efficiently handles the large scale and graph-b...
15. [Verification via Refereed Delegation for Machine Learning Programs](https://huggingface.co/papers/2502.19405) - A cryptographic refereed delegation approach ensures correct results for machine learning programs r...
16. [Verde: a verification system for machine learning over untrusted nodes](https://blog.gensyn.ai/verde-a-verification-system-for-machine-learning-over-untrusted-nodes/) - This is an academic paper describing Verde, a verification protocol for machine learning programs, a...
17. [Competition for AI upper-layer applications is fierce; Gensyn has ...](https://www.panewslab.com/en/articles/019e4980-5ed7-7553-9d84-ae2e5468e0fc) - ... decentralized AI field: ... The second type is zkML , such as Ritual and Giza, which use cryptog...
18. [Optimistic TEE-Rollups: A Hybrid Architecture for Scalable and ...](https://arxiv.org/html/2512.20176v1) - ... Trilemma, which posits that a decentralized inference system cannot simultaneously achieve high ...
19. [Igniting the Decentralized AI Revolution on the Blockchain With Ritual](https://www.youtube.com/watch?v=rCj48tQ5cyY) - ... Ritual supports various proof systems, including ZKML, optimistic proofs, and trusted execution ...
20. [Blockchain X AI, 6 Must-Know Infra Projects - DeSpread Research](https://research.despread.io/ai-infra-projects/) - Ritual Chain – a Layer 1 blockchain optimized for AI workloads; Infernet – a decentralized oracle ne...
21. [Privacy-Preserving Mechanisms Enable Cheap Verifiable Inference ...](https://arxiv.org/html/2602.17223v1) - These include methods such as secure multi-party computation (SMPC) and fully homomorphic encryption...
22. [Conan: Secure and Reliable Machine Learning Inference Against Malicious Service Providers](https://ieeexplore.ieee.org/document/11314627/) - In the Machine Learning as a Service paradigm, a service provider (e.g., a server) hosting a model o...
23. [[PDF] Conan: Secure and Reliable Machine Learning Inference against ...](https://tianweiz07.github.io/Papers/25-tifs-5.pdf) - The second is secure inference on the committed model against malicious servers, built on our generi...
24. [Homomorphic encryption for LLM inference: Is it viable, or are TEE ...](https://www.reddit.com/r/cryptography/comments/1q1hqq0/homomorphic_encryption_for_llm_inference_is_it/) - FHE for full LLM inference is still researchy; bootstrapping and ciphertext bloat kill latency and c...
25. [Privacy-Preserving Large Language Model Inference via GPU ...](https://icml.cc/virtual/2025/poster/45395) - Fully homomorphic encryption can, in principle, evaluate any function over encrypted data. One appro...
26. [Privacy-Preserving Large Language Model Inference via GPU ...](https://openreview.net/forum?id=PGNff6H1TV) - This paper presents a GPU-accelerated implementation of CKKS-based fully homomorphic encryption (FHE...
27. [draft-sharif-ai-model-lifecycle-attestation-00 - IETF Datatracker](https://datatracker.ietf.org/doc/draft-sharif-ai-model-lifecycle-attestation/) - Cryptographic Attestation for AI Model Lifecycle: From Training Data to Inference Output.
28. [Demo: Verify, Don’t Trust: Open-Source AI Is Not Enough](https://ieeexplore.ieee.org/document/11302934/) - Artificial intelligence (AI) is increasingly used in critical domains, yet models are often executed...
29. [EigenLayer Crosses $18B in Restaked ETH — How Vertical AVS ...](https://blockeden.xyz/blog/2026/03/20/eigenlayer-18b-tvl-vertical-avs-specialization-restaking-evolution/) - EigenAI went live on mainnet in late 2025, providing verifiable AI inference — allowing blockchain a...
30. [EigenLayer's Double Launch: 4x Rewards and Verifiable AI ...](https://p2p.org/economy/eigenlayers-double-launch-4x-rewards-and-verifiable-ai-infrastructure/) - P2P.org reduced commission from 10% to 5% through end of 2025 for all $EIGEN restakers. EigenLayer m...
31. [Allora Network: A comprehensive overview of a self-improving ...](https://oakresearch.io/en/reports/protocols/allora-network-comprehensive-overview-of-self-improving-decentralized-ai-network) - Gensyn focuses on large-scale AI model training by leveraging decentralized compute power with crypt...
32. [Introducing Judge - Gensyn](https://blog.gensyn.ai/introducing-judge/) - Verde ensures every judgment can be independently checked. It uses refereed delegation: multiple unt...
33. [Auditable Fine-Tuning and Inference for Proprietary AI - arXiv](https://arxiv.org/html/2603.07466v1) - We require the cloud provider to publicly release the hash commitment of the base model before any f...
34. [The zkML Singularity: A Comprehensive Analysis of the 2025 ...](https://academy.extropy.io/pages/articles/zkml-singularity.html) - The ability to compress the intelligence of a Neural Network into a succinct cryptographic proof (De...

