# Binary Storage Formats for Blockchain Signed Payloads

## SyndDB Sequencer: GCS + Arweave Dual-Target Storage Analysis

**Date:** December 2025 
**Context:** Analysis for SyndDB sequencer storing SQLite changesets with secp256k1 signatures

---

## Executive Summary

For a blockchain sequencer targeting both GCS and Arweave, the optimal stack is **CBOR serialization + COSE_Sign1 signing + zstd compression**. This achieves ~40-50% size reduction over JSON+base64, uses the standardized ES256K algorithm for secp256k1 signatures, and relies on mature, actively-maintained Rust crates.

ANS-104 bundles are valuable for Arweave batching and payment delegation, but should be used as a **transport wrapper** rather than the canonical storage format.

---

## Current Architecture

```
SQLite Changeset
    → JSON serialize (ChangesetData)
    → zstd compress
    → base64 encode into SignedMessage.payload
    → JSON serialize SignedMessage/SignedBatch
    → Upload to GCS
```

The signature scheme is solid—`keccak256(sequence || timestamp || message_hash)` over the compressed payload hash, with secp256k1 via alloy. This is already Ethereum-compatible.

---

## The Key Decision: Where Does ANS-104 Fit?

ANS-104 is **not a serialization format**—it's a **bundling protocol** for Arweave. Two architectural choices:

### Option A: ANS-104 as Transport Layer Only (Recommended)

```
SignedBatch (your format)
    → CBOR/COSE serialize
    → Wrap in ANS-104 DataItem for Arweave upload
    → Store raw CBOR/COSE to GCS
```

ANS-104 becomes an Arweave-specific wrapper. Your canonical format remains independent.

### Option B: ANS-104 as Primary Format

```
Each SignedMessage → ANS-104 DataItem
SignedBatch → ANS-104 Bundle containing DataItems
    → Upload bundle to Arweave
    → Store same bundle bytes to GCS
```

### Why Option A is Better

1. **ANS-104 overhead is significant for single items** (~132 bytes minimum per DataItem with secp256k1). For multi-message batches, this adds up.

2. **ANS-104 tags are AVRO-encoded**, which is unusual and adds parsing complexity for GCS consumers who just want to read your data.

3. **Your existing signature scheme is cleaner** for your use case. ANS-104 signs `deep_hash(data)` which is Arweave-specific—you'd need to maintain two signature schemes or abandon your current one.

4. **GCS doesn't benefit from ANS-104 structure**. The bundling value (payment delegation, batched upload) only matters for Arweave.

---

## Recommended Format: COSE_Sign1 + Batch Envelope

### Individual Message Structure

```rust
// Individual message - COSE_Sign1 structure
// This replaces SignedMessage when serializing to binary

use coset::{CoseSign1Builder, HeaderBuilder, iana};
use ciborium::Value;

// Protected header (signed over)
let protected = HeaderBuilder::new()
    .algorithm(iana::Algorithm::ES256K)  // secp256k1
    .value(1, Value::Integer(sequence.into()))      // custom: sequence
    .value(2, Value::Integer(timestamp.into()))     // custom: timestamp  
    .value(3, Value::Integer(message_type.into()))  // custom: type enum
    .build();

// Unprotected header (for discovery, not signed)
let unprotected = HeaderBuilder::new()
    .value(10, Value::Bytes(signer_address.to_vec()))  // Ethereum address
    .build();

let cose_message = CoseSign1Builder::new()
    .protected(protected)
    .unprotected(unprotected)
    .payload(compressed_payload)  // Raw zstd bytes, no base64!
    .create_signature(&[], |data| sign_with_secp256k1(data))
    .build();
```

### Batch Envelope Structure

```rust
// Batch envelope - CBOR map wrapping COSE_Sign1 messages
#[derive(Serialize, Deserialize)]
struct CborBatch {
    #[serde(rename = "v")]
    version: u8,  // Schema version for evolution
    #[serde(rename = "s")]
    start_sequence: u64,
    #[serde(rename = "e")]
    end_sequence: u64,
    #[serde(rename = "t")]
    created_at: u64,
    #[serde(rename = "m")]
    messages: Vec<serde_bytes::ByteBuf>,  // Each is a COSE_Sign1 blob
    #[serde(rename = "sig")]
    batch_signature: serde_bytes::ByteBuf,  // 65 bytes
    #[serde(rename = "addr")]
    signer: serde_bytes::ByteBuf,  // 20 bytes
}
```

---

## Size Analysis

### Per-Component Comparison

| Component | JSON+base64 | CBOR/COSE | Savings |
|-----------|-------------|-----------|---------|
| Sequence (u64) | ~15 bytes (`"sequence":42`) | 9 bytes | 40% |
| Timestamp (u64) | ~25 bytes | 9 bytes | 64% |
| MessageType | ~25 bytes (`{"type":"changeset"}`) | 1 byte | 96% |
| 10KB payload | ~13.7KB (base64) | 10KB (raw) | 27% |
| Signature | ~134 bytes (hex) | 65 bytes | 51% |
| Signer address | ~44 bytes (hex) | 20 bytes | 55% |

### Single Message (10KB payload)

- Current JSON+base64: ~14.2KB
- COSE_Sign1: ~10.2KB
- **Reduction: ~28%**

### 50-Message Batch (10KB each)

- Current JSON+base64: ~750KB
- CBOR batch of COSE_Sign1: ~515KB
- With zstd on full batch: ~100-150KB (similar payloads compress well together)

---

## ANS-104 Integration for Arweave

When uploading to Arweave, wrap your CBOR batch in an ANS-104 DataItem:

```rust
use bundles_rs::{ans104::DataItem, crypto::ethereum::EthereumSigner};

async fn upload_to_arweave(batch_cbor: &[u8], signer: &EthereumSigner) -> Result<String> {
    let tags = vec![
        ("Content-Type", "application/cbor"),
        ("App-Name", "synddb-sequencer"),
        ("App-Version", "1.0"),
        ("Schema-Version", "1"),
        ("Start-Sequence", &start_seq.to_string()),
        ("End-Sequence", &end_seq.to_string()),
        // Content hash for cross-system addressing
        ("Content-SHA256", &hex::encode(sha256(batch_cbor))),
    ];
    
    let data_item = DataItem::build_and_sign(
        signer,
        None,  // no target
        None,  // no anchor
        tags,
        batch_cbor.to_vec(),
    )?;
    
    // Upload via Irys/Bundlr for payment delegation
    irys_client.upload_data_item(data_item).await
}
```

### Arweave GraphQL Queries

The ANS-104 tags enable GraphQL queries on Arweave:

```graphql
query {
  transactions(
    tags: [
      { name: "App-Name", values: ["synddb-sequencer"] },
      { name: "Start-Sequence", values: ["1"] }
    ]
  ) {
    edges { node { id } }
  }
}
```

---

## Dual-Target Storage Pipeline

```
                                    ┌─────────────────────────────┐
                                    │         GCS Bucket          │
                                    │  batches/000001_000050.cbor │
                                    │  (optionally .cbor.zst)     │
                                    └─────────────────────────────┘
                                                 ▲
                                                 │ Upload raw CBOR
                                                 │
SignedBatch ──► CBOR serialize ──► zstd compress ┼──────────────────┐
                                                 │                  │
                                                 │                  ▼
                                                 │    ┌─────────────────────────────┐
                                                 │    │   Wrap in ANS-104 DataItem  │
                                                 │    │   (adds tags, secp256k1 sig)│
                                                 │    └─────────────┬───────────────┘
                                                 │                  │
                                                 │                  ▼
                                                 │    ┌─────────────────────────────┐
                                                 │    │   Upload via Irys/Bundlr   │
                                                 │    │   → Arweave permanent store │
                                                 │    └─────────────────────────────┘
                                                 │
                                    Store Arweave TX ID in GCS metadata
```

---

## Migration Strategy

Given you currently have 1 message per batch, here's a practical rollout:

### Phase 1: Add Compression to JSON Batches

- `.json.zst` files, GCS serves with `Content-Encoding: zstd`
- Immediate ~3-5x reduction
- No code changes for readers (GCS auto-decompresses)

### Phase 2: Implement CBOR/COSE Writer Alongside JSON

- New file extension: `.cbor` or `.cbor.zst`
- Sequencer writes both formats during transition
- Validator detects format by extension/magic bytes

### Phase 3: Enable Multi-Message Batches

- This is where CBOR really shines—50 messages compress dramatically better together
- Individual message signatures still verifiable
- Batch signature covers the whole thing

### Phase 4: Add Arweave Upload

- ANS-104 wrapper for CBOR batches
- Store Arweave TX ID in GCS object metadata for cross-referencing
- Use `Content-SHA256` tag for content-addressable lookup across systems

---

## Rust Crate Recommendations

```toml
[dependencies]
# CBOR encoding (replaces serde_json for binary)
ciborium = "0.2"

# COSE structures (bring your own crypto)
coset = "0.3"

# secp256k1 signing (you already have alloy, but k256 is lighter)
k256 = { version = "0.13", features = ["ecdsa"] }

# ANS-104 bundles for Arweave
bundles-rs = "0.1"  # Note: check current version, relatively new

# Compression
zstd = "0.13"
```

### Crate Notes

- **ciborium** (93M+ downloads) - Actively maintained, replaces unmaintained `serde_cbor`
- **coset** (Google-maintained) - Type-safe COSE structures, bring-your-own-crypto design
- **bundles-rs** (Decent Land Labs) - ANS-104 support for Rust, supports Ethereum signers

---

## Binary Format Comparison

| Format | Rust Crate | Downloads | Size vs JSON | Schema | Serde | Deterministic |
|--------|-----------|-----------|--------------|--------|-------|---------------|
| CBOR | ciborium | 93M | ~60-70% | Optional | ✅ | With care |
| MessagePack | rmp-serde | 65M | ~65% | No | ✅ | No |
| Protobuf | prost | High | ~25-40% | Required | ⚠️ | No |
| Borsh | borsh | 67M | ~40-50% | No | ❌ | ✅ Yes |
| RLP | alloy-rlp | 628K/mo | ~40-60% | No | ❌ | ✅ Yes |

### Why CBOR/COSE Wins

1. **Native binary support** - No base64 encoding overhead
2. **Optional schema flexibility** - Can evolve without breaking changes
3. **Serde compatibility** - Minimal code changes from JSON
4. **Deterministic encoding capability** - Critical for content-addressable storage
5. **Direct COSE integration** - Standardized signing (RFC 9052)
6. **secp256k1 support** - ES256K algorithm registered in RFC 8812

---

## ANS-104 Deep Dive

### DataItem Structure

| Field | Description | Size |
|-------|-------------|------|
| signature_type | 2-byte algorithm identifier | 2 bytes |
| signature | Cryptographic signature | 65 bytes (secp256k1) |
| owner | Public key | 65 bytes (uncompressed) |
| target | Optional recipient address | 0-32 bytes |
| anchor | Replay protection nonce | 0-32 bytes |
| tags | AVRO-encoded key-value metadata | Variable |
| data | Raw binary payload (inline) | Variable |

### Key Points

- **secp256k1 is fully supported** as signature type 2 (Ethereum-style)
- **Minimum overhead**: ~132 bytes (65-byte signature + 65-byte owner + headers)
- **DataItem ID** is SHA-256 hash of signature, enabling content verification
- **Value proposition**: Payment delegation and batching via Irys/Bundlr

---

## Implementation Priority

1. **Compress JSON batches** (`.json.zst`) — Easy win, do next
2. **Design CBOR schema** with versioning — The `version` field is crucial
3. **Implement CBOR writer** when ready for multi-message batches
4. **Add ANS-104 wrapping** when Arweave becomes priority

---

## Key Insight

COSE/CBOR is your **canonical format** for both storage targets, and ANS-104 is just the **Arweave transport wrapper**. This keeps your architecture clean and avoids coupling your data format to Arweave-specific concerns.

---

## References

- [RFC 9052 - COSE Structures and Process](https://datatracker.ietf.org/doc/html/rfc9052)
- [RFC 8812 - COSE/JOSE secp256k1 Registration](https://datatracker.ietf.org/doc/html/rfc8812)
- [ANS-104 Specification](https://github.com/ArweaveTeam/arweave-standards/blob/master/ans/ANS-104.md)
- [bundles-rs Documentation](https://blog.decent.land/bundles-rs/)
- [coset Crate](https://google.github.io/coset/rust/coset/index.html)
- [ciborium Crate](https://crates.io/crates/ciborium)
