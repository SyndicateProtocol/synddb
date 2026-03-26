# SyndDB + Stylus

### Build Any App. Use SQLite. Get Blockchain Verifiability.

---

## About Syndicate

We build infrastructure for developers to move applications onchain.

Our focus: meet developers **where they are**, not force them into new paradigms.

---

## The Problem

Asking developers to rewrite everything as smart contracts is a losing battle.

- Less scalable technology
- Far more moving parts
- EVM/SVM can't handle CEX-scale orderbooks

---

## Our Insight

Don't force them to change.

**Let them bring their databases. We wire them up to the chain.**

---

## Introducing SyndDB

Write apps in **any language**. Use **SQLite**. Get **blockchain verifiability**.

---

## Why SQLite?

- **Billions** of deployments worldwide
- Available in every programming language
- Deterministic execution
- Sub-millisecond writes
- Trivial to run inside a TEE
- Massive LLM training data

---

## The Developer Experience

```python
import sqlite3
from synddb import SyndDB

db = sqlite3.connect('app.db')
synddb = SyndDB.attach('app.db', 'https://sequencer:8433')

# Just write SQL. We handle the rest.
db.execute("INSERT INTO trades VALUES (?, ?)", (123, 'BUY'))
```

---

## How It Works

1. **Write apps** in any language with SQLite
2. **Import SyndDB client** (2-3 lines of code)
3. **Run in a TEE** (Docker container)
4. **We capture everything** and publish to DA layers

---

## Architecture Overview

```
Application (Any Language)
         |
    SQLite + SyndDB Client (TEE #1)
         |
    Sequencer Service (TEE #2)
         |
    DA Layers (Celestia, GCS, IPFS)
         |
    Validators (TEE) --> Bridge Contract
```

---

## The Four Components

- **Client Library**: Embeds in your app, captures SQL changesets, zero code changes to business logic
- **Sequencer**: Separate TEE with isolated signing keys, batches and publishes to DA layers
- **Validators**: Replay SQL operations, verify invariants, sign for bridge settlement
- **Bridge Contract**: Processes cross-chain messages with modular validation

---

## TEE Security Model

```
+---------------------------------------------+
|         GCP Confidential Space VM           |
|  +-----------------+  +-----------------+  |
|  |   Application   |  |    Sequencer    |  |
|  |   + Client      |  |   (signing keys)|  |
|  |   Container     |  |    Container    |  |
|  +--------+--------+  +--------+--------+  |
|           +----------+---------+           |
|              Shared SQLite DB              |
|  Hardware Root of Trust (AMD SEV-SNP)      |
+---------------------------------------------+
```

---

## The Missing Piece: TEE Bootstrapping

Everything depends on one critical question:

**How do you prove onchain that a service is actually running inside a legitimate TEE?**

Without this, anyone could claim to be running in a TEE without proof.

---

## What TEE Bootstrapping Requires

When a TEE service starts, it must:

1. Generate an ephemeral signing keypair
2. Obtain a JWT attestation token from Google
3. **Verify that token onchain** to register the signing key

That verification is the bottleneck. It requires JWT parsing, RSA signature verification, PKCS#1 v1.5 padding validation, and claim checking.

---

## Approach #1: Pure Solidity

Base's [op-enclave / nitro-validator](https://github.com/base/nitro-validator) verifies attestations in Solidity.

- **~63 million gas** for full verification
- Even with certificate caching: **20M+ gas** (Marlin's NitroProver)
- Multi-transaction, multi-step process
- Not well maintained, difficult to use

Cryptographic operations are fundamentally expensive in the EVM.

---

## Approach #2: Zero-Knowledge VMs

We tried SP1, then RISC Zero. Verify the JWT offchain, submit a succinct proof onchain.

This works, but:

- **$0.018 - $0.13 per proof** (every service restart)
- **30 seconds to 3 minutes** proof generation
- GPU infrastructure or prover network required
- zkVM toolchain management, build determinism concerns
- Proof latency **directly delays node bootup**

Sequencers and validators cannot operate until the proof completes.

---

## The Bootstrapping Bottleneck

```
TEE starts --> Generate key --> Get JWT --> Wait for proof...
                                                |
                                          30s - 3 min
                                                |
                                           Submit proof
                                                |
                                          Register key
                                                |
                                        Begin operating
```

Every restart, every scaling event, every key rotation hits this delay.

For infrastructure that needs **fast recovery and elastic scaling**, this is a meaningful bottleneck.

---

## Approach #3: Stylus

Stylus eliminates the entire off-chain proving pipeline.

One Rust smart contract compiled to WASM. Full JWT verification in a single onchain transaction. No external services. No GPU. No proof generation delay.

---

## Why Stylus Is the Right Abstraction

TEE attestation verification is:

- **Compute-heavy**: RSA signature verification, JWT parsing, padding validation
- **Memory-heavy**: ~2KB+ JWT payloads, RSA key material
- **Storage-minimal**: Just trusted key hashes and image digest allowlists

This maps **perfectly** to Stylus's strengths.

---

## Stylus Cost Model

| Dimension | Stylus vs EVM | Impact |
|-----------|---------------|--------|
| **Compute** | 10-100x cheaper | RSA verification, JWT parsing become practical |
| **Memory** | 100-500x cheaper | Large JWT payloads handled in-memory |
| **Storage** | Same as EVM | Minimal for this use case |

Stylus uses **ink** (1 gas = 10,000 ink) because WASM opcodes execute orders of magnitude faster than EVM equivalents.

---

## The Stylus Flow

```
TEE starts
    |
Generate ephemeral secp256k1 keypair
    |
Request GCP Confidential Space JWT token
    |
Submit JWT + JWK directly to Stylus contract
    |
Onchain (single tx):
  - Verify JWK key material hash
  - Verify RS256 signature (SHA-256 + modexp precompiles)
  - Validate PKCS#1 v1.5 padding
  - Parse and validate all claims
  - Verify image signature (ecrecover)
    |
TeeKeyManager registers signing key --> Begin operating
```

No proof service. No GPU. No waiting.

---

## Head-to-Head: Bootstrapping Speed

| Metric | SP1 Network | RISC Zero GPU | Stylus |
|--------|-------------|---------------|--------|
| Bootstrap time | 30s - 3 min | 30s - 3 min | **Seconds** |
| Per-bootstrap cost | ~$0.13 | ~$0.018 | **Gas only** |
| GPU required | No (hosted) | Yes (NVIDIA L4) | **No** |
| External deps | PROVE tokens, prover network | CUDA, proof service | **Stylus SDK** |
| Build determinism | N/A | Critical (image ID match) | **Standard Rust** |
| Maturity risk | zkVM vulnerabilities | zkVM vulnerabilities | **Battle-tested WASM** |

---

## The Appchain Advantage

For Arbitrum appchains with custom gas tokens, Stylus attestation verification is effectively **zero cost**.

- Appchain operators control gas token economics
- Ink-to-gas ratio is configurable
- Combined with gas subsidy contracts (like Xai), verification is invisible to users

zkVM proofs always have a floor cost. Stylus on an appchain has none.

---

## Infrastructure Eliminated

The zkVM approach required:

- A GPU-enabled Cloud Run service (RISC Zero) **or** prover network integration (SP1)
- CUDA dependencies and 60-minute timeout configuration
- zkVM toolchain management
- Guest program compilation and image ID coordination
- Build determinism pipelines

Stylus replaces **all of this** with a single Rust smart contract.

---

## What This Means in Practice

| Metric | zkVM Approach | Stylus Approach |
|--------|---------------|-----------------|
| Infrastructure services | 4 (TEE + proof service + relayer + contracts) | **3** (TEE + relayer + contracts) |
| Per-bootstrap cost | $0.018 - $0.13 | **Onchain gas only** |
| Proof generation time | 30s - 3 min | **< 1 second** |
| Node bootup delay | 30s - 3 min | **Seconds** |
| Build pipeline | zkVM toolchain + image IDs | **Standard Rust/WASM** |

---

## Implementation Details

The Stylus attestation verifier is:

- **Single Rust file** using `stylus-sdk` 0.10.0
- Uses EVM precompiles directly: `ecrecover`, `SHA-256`, `modexp`
- Custom base64url decoding and minimal JSON parsing (no heavy dependencies)
- Implements `IAttestationVerifier`, same interface as the zkVM verifier
- Drop-in replacement: switch via `updateAttestationVerifier()` in one transaction

---

## Modular and Additive

```solidity
interface IAttestationVerifier {
    function verifyAttestationProof(
        bytes calldata publicValues,
        bytes calldata proofBytes
    ) external view returns (address);
}
```

Stylus doesn't exclude zkVMs. Both can coexist behind this interface.

Want defense in depth? Require **both** Stylus verification and a zkVM proof.

Speed of Stylus for normal operations. Additional security of zkVM as a secondary check.

---

## Lessons for Builders

**Scope Stylus to compute, not storage.** We initially planned to embed SQLite inside Stylus. Storage costs made this infeasible. TEE attestation verification -- purely compute-bound -- was the ideal target.

**If it's hard in Solidity, that's a signal.** We built an entire zkVM pipeline because onchain JWT verification was too expensive in Solidity. Stylus turned that pipeline into a single contract.

**Design modular interfaces.** Our `IAttestationVerifier` made adopting Stylus zero-risk. If it didn't work, we switch back in one transaction.

---

## The Comparison

| | Traditional Onchain | Traditional Offchain | SyndDB |
|---|---|---|---|
| Latency | ~seconds | sub-ms | sub-ms |
| Throughput | 10-50 ops/s | unlimited | unlimited |
| Verifiable | yes | no | yes |
| Any language | no | yes | yes |

---

## Use Cases

- **Perp DEXs** -- orderbook matching at scale
- **Prediction markets** -- high-frequency updates
- **Stablecoin payments** -- instant settlements
- **Gaming** -- real-time state
- **NFT marketplaces** -- metadata and trading

Any application that needs **performance + verifiability**.

---

## Migration Path

Already using SQLite? Even easier.

1. Import SyndDB client library
2. Add message tables (if needed)
3. Deploy sequencer in separate TEE
4. **No other code changes required**

The Stylus contract handles trust bootstrapping that makes the entire system work.

---

## Summary

- Write apps in **any language** using **SQLite**
- Get **blockchain verifiability** for free
- **Sub-millisecond latency**, unlimited throughput
- Stylus enables **near-instant TEE bootstrapping**, replacing zkVM proving pipelines
- **30-180x faster** node bootup, **zero per-proof costs**
- A single Rust smart contract replaces an entire GPU proving infrastructure

---

## Stylus Made This Possible

Without Stylus, we needed either:
- Prohibitively expensive Solidity verification (~63M gas)
- An entire zkVM proving pipeline ($0.02-0.13/proof, 30s-3min delay)

With Stylus: **one contract, one transaction, instant bootstrapping.**

Stylus didn't just improve our system. It simplified our entire architecture.

---

## Questions?

We're happy to discuss:
- TEE attestation verification on Stylus
- Building high-throughput applications with SyndDB
- The Stylus cost model for compute-heavy operations
- Orderbooks, prediction markets, payments, gaming
