# Stylus Infrastructure Case Study: SyndDB

**Executive Summary:** Syndicate uses Arbitrum Stylus for on-chain TEE attestation verification, replacing a GPU-dependent zkVM pipeline with a single WASM smart contract that verifies GCP Confidential Space JWT tokens directly on-chain -- delivering simpler architecture, lower operational costs, and faster bootstrapping for TEE-based infrastructure.

## Opportunity

The core challenge in blockchain infrastructure today is bridging offchain and onchain systems. High-performance applications -- orderbooks, gaming, social feeds -- need sub-millisecond latency and high throughput, but also need onchain verifiability and settlement. These two requirements have historically been at odds: you either get performance (offchain) or transparency (onchain), but not both.

SyndDB solves this by enabling developers to build applications in **any programming language** using SQLite for persistence, while automatically capturing SQL operations and publishing them for verification and replication. Applications run inside Trusted Execution Environments (TEEs) on Google Cloud Confidential Space, providing hardware-backed attestations that the correct code is running. Validators, also in TEEs, verify the SQL operations before signing for on-chain settlement through a cross-chain bridge contract.

The critical missing piece was **TEE bootstrapping**: how do you prove on-chain that a sequencer or validator is actually running inside a legitimate TEE, running the correct container image, with secure boot enabled and debug mode disabled? Without this, the entire trust model breaks down -- anyone could claim to be running in a TEE without proof.

This matters for every developer building on SyndDB. The bootstrapping flow is what lets sequencers and validators register their signing keys on-chain, which in turn enables them to submit state updates and process cross-chain messages through the bridge contract. Without reliable, cost-effective attestation verification, the system cannot go to production.

## Solution Overview

SyndDB runs sequencers and validators inside TEEs on Google Cloud Confidential Space. When a TEE service starts, it generates an ephemeral signing keypair and obtains a JWT attestation token from Google's attestation service. This token proves the service is running the expected container image with secure boot enabled and debug mode disabled. The token must be verified on-chain to register the TEE's signing key, enabling it to participate in the protocol.

**Stylus is used for this on-chain TEE attestation verification.** The Stylus contract receives the raw JWT token and RSA public key material, then performs full RS256 signature verification and claim validation directly on-chain using EVM precompiles -- all in a single transaction.

### Why Stylus Was the Right Choice

TEE attestation verification requires parsing JWT tokens, verifying RSA signatures, validating PKCS#1 v1.5 padding, checking multiple claims, and recovering secp256k1 signatures. This is fundamentally a compute-heavy operation, not a storage-heavy one -- which maps perfectly to Stylus's strengths.

We evaluated three approaches:

**1. Pure Solidity: Technically possible but prohibitively expensive**

The most direct comparison is [op-enclave](https://github.com/base/op-enclave) by Base, which verifies AWS Nitro attestations in Solidity using their [nitro-validator](https://github.com/base/nitro-validator) library. Their approach requires traversing the full certificate chain on-chain, which costs approximately **63 million gas** with no prior verified certificates. Even with certificate caching and multi-transaction strategies, [Marlin's NitroProver](https://blog.marlin.org/on-chain-verification-of-aws-nitro-enclave-attestations) still requires **20+ million gas** for attestation verification after pre-verifying individual certificates at 12-13 million gas each.

The gas cost comes from the fundamental operations required: CBOR/COSE parsing, X.509 certificate chain validation, and ECDSA P-384 signature verification -- all of which are expensive in the EVM. The op-enclave project itself is difficult to use and not well maintained, and the gas costs make it impractical for frequent attestation verification.

**2. zkVMs: Works but expensive and operationally complex**

Our initial approach used zero-knowledge virtual machines (first SP1, then RISC Zero) to verify the JWT token off-chain and generate a succinct proof for on-chain verification. The zkVM program runs inside a GPU-based proof service, verifies the RS256 JWT signature, all attestation claims, and generates a Groth16 proof that can be verified on-chain cheaply.

This works, but the operational burden is significant:

- **GPU infrastructure required**: Proof generation takes 2-5 minutes on an NVIDIA L4 GPU. At ~$0.67/hour on Google Cloud Run, each proof costs roughly $0.02-0.06 in compute alone -- and that's before accounting for idle time, since the proof service must remain available.
- **Operational complexity**: Requires a separate GPU-enabled Cloud Run service with CUDA support, 60-minute timeout configuration, single-instance concurrency limits, and RISC Zero toolchain management.
- **SP1 limitations**: We started with SP1 but its closed-source GPU prover cannot run in Google Cloud Run due to Docker-in-Docker requirements and memory constraints. Network proving adds setup burden and costs scale quickly.
- **RISC Zero migration**: We migrated to RISC Zero 3.0 for its open-source GPU prover and Cloud Run compatibility, but this was a significant engineering effort.
- **zkVM maturity concerns**: zkVMs are relatively new technology and have had critical vulnerabilities. For maximum security, you'd want to use multiple zkVMs (e.g., both RISC Zero and SP1), which multiplies infrastructure costs.
- **Build determinism**: The RISC Zero image ID (a hash of the compiled guest program) must match exactly between the proof service binary and the on-chain contract. Different build environments produce different ELF binaries, requiring careful CI/CD pipelines with CUDA stubs for image ID extraction during Docker builds.

**3. Stylus: The right abstraction**

Stylus eliminates the entire off-chain proving pipeline. The attestation verifier is a Rust smart contract compiled to WASM that runs directly on-chain:

- **No external services**: No GPU instances, no proof service, no CUDA dependencies.
- **Single transaction**: JWT verification happens in one on-chain call.
- **Battle-tested runtime**: Stylus runs on Arbitrum's WASM runtime (Wasmer), which is production-proven infrastructure -- not an experimental proving system.
- **Rust ecosystem**: The contract is written in the same language as the rest of SyndDB's infrastructure, using the same cryptographic primitives and patterns.
- **Drop-in replacement**: Both the Solidity/RISC Zero verifier and the Stylus verifier implement the same `IAttestationVerifier` interface, so the `TeeKeyManager` contract can switch between them via `updateAttestationVerifier()`.
- **Additive, not exclusive**: Nothing precludes adding zkVM verification alongside Stylus if additional security guarantees are desired. The modular interface means both can coexist.

## Background

### Infrastructure Overview

SyndDB is a SQLite replication system for blockchain applications. The architecture consists of:

1. **Application** (any language) -- writes to SQLite inside a TEE, using any framework or library
2. **SyndDB Client Library** (Rust/Python/Node.js/FFI) -- captures SQL changesets via SQLite Session Extension, sends to sequencer
3. **Sequencer Service** (separate TEE) -- receives changesets, batches and signs them with COSE_Sign1, publishes to storage layers (GCS, with Celestia/EigenDA planned)
4. **Validators** (in TEEs) -- sync from storage, replay changesets, verify invariants, sign for settlement
5. **Bridge Contract** -- processes cross-chain messages from validators with modular pre/post validation

The application and sequencer run in **separate TEEs** for defense in depth -- the sequencer holds signing keys isolated from application code.

```
Application (TEE #1)          Sequencer (TEE #2)           Validators (TEE)
       |                            |                           |
       |  SQLite + Client Library   |                           |
       |----------------------------+   Batch & Sign            |
       |     HTTP + Attestation     |       |                   |
       |                            |       v                   |
       |                            |  Storage (GCS/DA)         |
       |                            |       |                   |
       |                            |       +------------------>|  Verify & Sign
       |                            |                           |
       |                            |                           v
       |                            |                     Bridge.sol
```

### Previous Approach

Our initial proposal was to embed all of SQLite inside of Stylus -- running the entire database engine as a WASM smart contract. This would have been the ultimate integration: applications would write SQL directly on-chain with native performance.

Unfortunately, this wasn't feasible. Stylus makes **compute** dramatically cheaper (10-100x vs EVM), but **storage** still costs the same as on the EVM. Storage operations (SLOAD/SSTORE) are the dominant cost for database workloads. We couldn't store SQLite changesets, WAL pages, or database state inside Stylus without incurring the same storage costs as Solidity. The use case we needed was compute-heavy, not storage-heavy.

This led us to scope Stylus to its ideal use case: **TEE attestation verification**, which is almost entirely compute (JWT parsing, RSA signature verification, claim validation) with minimal storage (just the trusted key hashes and image digest allowlists). The bulk of the system -- changeset capture, batching, storage, validation -- runs off-chain in TEEs, with Stylus handling the critical on-chain bootstrapping step.

## Stylus Unlocks for Infrastructure

### Compute Cost Advantages

Stylus introduces a fundamentally different cost model for on-chain computation:

| Dimension | Stylus vs EVM | Impact |
|-----------|---------------|--------|
| **Compute** | 10-100x cheaper | RSA signature verification, JWT parsing, PKCS#1 padding validation all become practical |
| **Memory** | 100-500x cheaper | Parsing large JWT payloads (~2KB+) and RSA key material in-memory is feasible |
| **Storage** | Same as EVM | Trusted JWK hashes and image digest allowlists cost the same (but are minimal for this use case) |

The Stylus VM uses **ink** -- a fractional gas unit where 1 gas = 10,000 ink -- because WASM opcodes execute orders of magnitude faster than EVM equivalents. Unlike the EVM which charges before each opcode via table lookups, Stylus strategically batches gas charges within the program, further reducing overhead.

### Comparison to Alternative Approaches

#### vs. Pure Solidity (op-enclave / nitro-validator)

| Metric | Solidity (nitro-validator) | Stylus |
|--------|---------------------------|--------|
| Gas cost (full verification) | ~63M gas | Significantly lower (WASM compute is 10-100x cheaper) |
| Gas cost (with cert caching) | ~20M gas | N/A (single transaction, no multi-step process) |
| Multi-transaction required? | Yes (cert-by-cert) | No (single call) |
| Code complexity | CBOR decoding + X.509 parsing in Solidity | Native Rust with EVM precompiles |
| Maintenance | Difficult (op-enclave is not well maintained) | Standard Rust toolchain |

#### vs. zkVM Proving (RISC Zero / SP1)

| Metric | zkVM (RISC Zero) | Stylus |
|--------|-------------------|--------|
| GPU infrastructure | Required (NVIDIA L4, ~$0.67/hr) | None |
| Proof generation time | 2-5 minutes | Instant (single transaction) |
| Per-proof compute cost | ~$0.02-0.06 | On-chain gas only |
| Monthly infrastructure | ~$500+ (always-on GPU instance) | $0 (no off-chain services) |
| External dependencies | CUDA, RISC Zero toolchain, proof service | Stylus SDK only |
| Build determinism | Critical (image ID must match exactly) | Standard Rust compilation |
| Maturity risk | zkVMs have had critical vulnerabilities | WASM is battle-tested |
| Multiple prover security | Requires running 2+ zkVMs (2x+ cost) | Single verifier sufficient |

#### Appchain Cost Model

For Arbitrum appchains with custom gas tokens, the economics become even more compelling. Appchain operators control their gas token economics and can configure the ink-to-gas ratio. This means TEE attestation verification can be effectively **zero additional cost** beyond the standard Arbitrum chain operation costs. Combined with gas subsidy contracts (as used by chains like Xai), attestation verification can be made invisible to end users.

## Implementation

### Architecture: zkVM vs. Stylus

**zkVM Approach** (original):
```
TEE Service starts
       |
       v
Generate ephemeral secp256k1 keypair
       |
       v
Request GCP Confidential Space JWT token
       |
       v
Send JWT to proof-service (GPU instance)  <-- External dependency
       |
       v
RISC Zero zkVM verifies JWT (2-5 min)     <-- GPU compute cost
       |
       v
Generate Groth16 proof                     <-- Complex build pipeline
       |
       v
Submit proof to RiscZeroAttestationVerifier.sol
       |
       v
On-chain: verify Groth16 proof + check claims
       |
       v
TeeKeyManager registers signing key
```

**Stylus Approach** (current):
```
TEE Service starts
       |
       v
Generate ephemeral secp256k1 keypair
       |
       v
Request GCP Confidential Space JWT token
       |
       v
Submit JWT + JWK key material directly to Stylus contract
       |
       v
On-chain: full JWT verification in single tx
  - Verify JWK key material hash
  - Verify RS256 signature (SHA-256 + modexp precompiles)
  - Validate PKCS#1 v1.5 padding
  - Parse and validate all claims
  - Verify image signature (ecrecover)
       |
       v
TeeKeyManager registers signing key
```

The Stylus approach eliminates three components from the architecture: the GPU proof service, the RISC Zero toolchain, and the proof generation pipeline. The TEE service constructs the proof data locally (just the raw JWT and JWK RSA key material) and submits it directly on-chain.

### Integration and Libraries

The Stylus attestation verifier (`contracts/stylus/attestation-verifier/src/lib.rs`) is built with:

- **`stylus-sdk` 0.10.0** -- Stylus contract framework
- **`alloy-primitives` and `alloy-sol-types`** -- Ethereum type compatibility
- **EVM precompiles** used directly:
  - `ecrecover` (0x01) -- secp256k1 signature recovery for image signature verification
  - `SHA-256` (0x02) -- JWT signing input hash
  - `modexp` (0x05) -- RSA signature verification via modular exponentiation

The contract implements custom base64url decoding and minimal JSON parsing without external dependencies -- no `serde_json`, no allocator-heavy crates. This keeps the WASM binary small and gas-efficient.

Both the Solidity (`RiscZeroAttestationVerifier`) and Stylus (`StylusAttestationVerifier`) contracts implement the same `IAttestationVerifier` interface:

```solidity
interface IAttestationVerifier {
    function verifyAttestationProof(
        bytes calldata publicValues,
        bytes calldata proofBytes
    ) external view returns (address);
}
```

Switching between verifiers is a single call to `TeeKeyManager.updateAttestationVerifier()`. No other contracts need to change.

### Integration Process

Deploying the Stylus attestation verifier is straightforward:

1. **Deploy the Stylus contract** to an Arbitrum chain using the provided deployment script
2. **Initialize trusted JWK hashes** -- add Google's signing key hashes to the allowlist
3. **Set allowed image digests** -- register the container image hashes for your TEE services
4. **Point `TeeKeyManager` to the new verifier** via `updateAttestationVerifier()`
5. **Configure TEE services** to use `ProverMode::Stylus` -- this skips the proof service entirely

The bulk of the work is in step 1. The remaining steps are the same configuration that would be needed with any attestation verifier.

### Challenges and Resolution

The main challenge was **scoping Stylus to the right use case**. Our initial instinct was to run as much as possible inside Stylus -- potentially the entire SQLite engine. Through prototyping, we learned that Stylus's cost advantages are concentrated in compute and memory, while storage costs remain unchanged. This led us to identify TEE attestation verification as the ideal target: a compute-intensive, memory-intensive, storage-minimal operation that was already a pain point in our architecture.

A secondary challenge was implementing JWT parsing and RSA verification without Rust's standard library. The Stylus `no_std` environment required custom implementations of base64url decoding, minimal JSON extraction (searching for key-value patterns rather than full parsing), and PKCS#1 v1.5 padding verification. These are well-understood algorithms, but implementing them correctly in a constrained environment required care.

### Adoption for Builders

For developers building on SyndDB, Stylus is invisible -- it's an infrastructure detail. The developer experience is:

1. **Write your application in any language** that uses SQLite (Python, JavaScript, Go, Rust, etc.)
2. **Import the SyndDB client library** (2-3 lines of code to attach to your SQLite connection)
3. **Deploy to a TEE** on Google Cloud Confidential Space
4. **The infrastructure handles the rest**: changeset capture, sequencing, attestation verification (via Stylus), validator signing, and bridge settlement

From the builder's perspective, they write a SQLite application and run it anywhere. The Stylus contract handles the trust bootstrapping that makes the entire system work.

## Impact

### Quantitative

| Metric | zkVM Approach | Stylus Approach | Improvement |
|--------|---------------|-----------------|-------------|
| Infrastructure services | 4 (TEE + proof service + relayer + contracts) | 3 (TEE + relayer + contracts) | 1 fewer service |
| GPU instances required | 1 (NVIDIA L4) | 0 | Eliminated |
| Proof generation time | 2-5 minutes | <1 second (single tx) | ~100-300x faster |
| Monthly GPU cost | ~$500+ (always-on L4 on Cloud Run) | $0 | 100% reduction |
| Build pipeline complexity | CUDA stubs, image ID extraction, deterministic builds | Standard Rust/WASM compilation | Significantly simpler |
| External dependencies | RISC Zero toolchain, CUDA, proof service | Stylus SDK | Fewer dependencies |
| Bootstrapping latency | Minutes (proof generation) | Seconds (single transaction) | Orders of magnitude faster |

### Qualitative

**Simpler architecture**: Removing the GPU proof service eliminates an entire class of operational concerns -- CUDA driver compatibility, GPU availability, proof service scaling, build determinism for RISC Zero image IDs, and the complexity of coordinating between the TEE service, proof service, and on-chain contracts.

**Better developer experience**: New team members no longer need to understand the RISC Zero toolchain, guest program compilation, or GPU infrastructure to work on attestation. The Stylus contract is a single Rust file that reads like normal application code.

**Faster iteration**: Changes to attestation logic require recompiling and redeploying a single WASM contract, rather than rebuilding a RISC Zero guest program, updating the image ID on-chain, and redeploying the proof service with matching binaries.

**Defense in depth option**: The modular `IAttestationVerifier` interface means Stylus and zkVM verification can coexist. For maximum security, teams can require both a Stylus verification and a zkVM proof, getting the speed of Stylus for normal operations with the additional security of zkVM proofs as a secondary check.

### Comparison to Other Approaches

**vs. op-enclave (Solidity-only)**: op-enclave's [nitro-validator](https://github.com/base/nitro-validator) requires ~63M gas for full attestation verification in Solidity. Even with certificate caching, the multi-transaction approach adds complexity and still costs 20M+ gas. Stylus's 10-100x compute discount makes the same cryptographic operations dramatically cheaper in a single transaction. For appchains, this cost effectively disappears.

**vs. zkVM proving costs**: Based on Google Cloud L4 GPU pricing (~$0.67/hour on Cloud Run) and 2-5 minute proof times, each attestation proof costs $0.02-0.06 in compute. With an always-on proof service for availability, monthly costs exceed $500. Stylus replaces this entirely with on-chain gas costs, which on an appchain with a custom gas token can be subsidized to zero.

## Lessons for Other Infrastructure Teams

### 1. Scope Stylus to Compute, Not Storage

Stylus makes compute 10-100x cheaper and memory 100-500x cheaper, but storage costs remain identical to the EVM. If your use case is compute-heavy with minimal storage (cryptographic verification, complex parsing, mathematical operations), Stylus is a strong fit. If it's storage-heavy (database operations, large state management), the cost savings will be limited.

**Our experience**: We initially planned to run SQLite inside Stylus. Storage costs made this infeasible. By narrowing to TEE attestation verification -- a purely compute-bound operation -- we found the ideal use case.

### 2. Consider Individual Components, Not the Whole Application

Don't try to move your entire application into Stylus. Instead, identify the specific components where Stylus's cost model provides the most leverage. In our case, that was a single contract (attestation verification) out of a larger system of contracts, services, and infrastructure.

**Pattern to look for**: Operations that are expensive or impractical in Solidity but straightforward in Rust. JWT parsing, certificate chain verification, complex encoding/decoding, and mathematical operations are all good candidates.

### 3. If It's Hard in Solidity, That's a Signal

We spent significant effort on the RISC Zero approach specifically because on-chain JWT verification in Solidity would have been prohibitively expensive. The difficulty of the Solidity approach was the signal that an alternative execution environment could provide value. Stylus turned a multi-service, GPU-dependent pipeline into a single smart contract.

**Ask yourself**: Is this component complex because of the business logic, or because of EVM limitations? If the latter, Stylus may be the right tool.

### 4. Easy in Rust Doesn't Always Mean Easy in Stylus

While Stylus runs Rust, it's a `no_std` environment with gas metering. Not all Rust crates work out of the box -- we had to implement custom base64url decoding and JSON parsing. EVM precompiles (SHA-256, modexp, ecrecover) are powerful building blocks, but you need to understand their gas costs and calling conventions. Test your assumptions about gas costs early.

### 5. The Modular Interface Pattern

Design your contracts with swappable implementations behind a common interface. Our `IAttestationVerifier` interface lets us switch between Solidity/RISC Zero and Stylus verification without changing any other contract. This pattern made adopting Stylus low-risk -- if it didn't work out, we could switch back with a single transaction. It also enables defense-in-depth strategies where multiple verification approaches can coexist.
