# Binary serialization formats for blockchain signed payloads

For a blockchain sequencer storing SQLite changesets, **CBOR with COSE_Sign1 signatures** offers the best balance of efficiency, secp256k1 support, and Rust ecosystem maturity—reducing payload size by **~40-50%** versus JSON+base64 while maintaining compatibility with both GCS and Arweave. ANS-104 bundles are valuable for Arweave batching but add complexity that may not be justified for dual-target storage.

## ANS-104 supports secp256k1 but adds bundling overhead

The Arweave Bundled Data Format (ANS-104) is designed for batching multiple signed data items into a single Arweave transaction, achieving **~3000x throughput improvement** over individual transactions. Each DataItem is a self-contained signed container with this structure:

| Field | Description | Size |
|-------|-------------|------|
| signature_type | 2-byte algorithm identifier | 2 bytes |
| signature | Cryptographic signature | 65 bytes (secp256k1) |
| owner | Public key | 65 bytes (uncompressed) |
| target | Optional recipient address | 0-32 bytes |
| anchor | Replay protection nonce | 0-32 bytes |
| tags | AVRO-encoded key-value metadata | Variable |
| data | Raw binary payload (inline) | Variable |

**secp256k1 is fully supported** as signature type 2 (Ethereum-style). The minimum overhead for an Ethereum-signed DataItem is **~132 bytes** (65-byte signature + 65-byte owner + headers), compared to ~1026 bytes for native Arweave RSA signatures. The DataItem ID is the SHA-256 hash of the signature, enabling content verification.

The primary Rust library is **bundles-rs** (0.1.x, actively maintained by Load Network/Decent Land Labs), which supports Ethereum, Solana, and Ed25519 signers:

```rust,ignore
use bundles_rs::{ans104::DataItem, crypto::ethereum::EthereumSigner};
let signer = EthereumSigner::from_private_key(key)?;
let item = DataItem::build_and_sign(&signer, None, None, tags, data)?;
```

**The value proposition of ANS-104** is payment delegation and batching—a third party (like Irys/Bundlr) pays Arweave storage fees while preserving the original creator's signature. However, for dual-target storage where you're also writing to GCS, ANS-104's bundling benefits diminish since GCS doesn't understand the format. Individual COSE-signed payloads may be simpler.

## COSE provides standardized secp256k1 signing with minimal overhead

COSE (RFC 9052) is the CBOR Object Signing and Encryption standard, offering a well-specified envelope for signed binary data. **secp256k1 is officially registered** via RFC 8812 as algorithm **ES256K (alg=-47)** with curve identifier 8.

The COSE_Sign1 structure (for single signatures) achieves **~16-26 bytes overhead** beyond the raw signature and payload:

```
COSE_Sign1 = [
    protected   : bstr,    // CBOR-encoded headers (~4-6 bytes for alg)
    unprotected : {},      // Empty map (~1 byte)
    payload     : bstr,    // Your data or nil for detached
    signature   : bstr     // 64-byte R||S for secp256k1
]
```

**Size comparison for a 1KB payload with secp256k1 signature:**

| Format | Approximate Size | Overhead |
|--------|-----------------|----------|
| Raw binary + signature | 1088 bytes | Baseline |
| COSE_Sign1 | ~1108 bytes | +20 bytes |
| ANS-104 DataItem | ~1220 bytes | +132 bytes |
| JSON + base64 + hex sig | ~1560 bytes | +472 bytes |

The Rust ecosystem is mature. **coset** (0.3.8, Google-maintained, 13M+ downloads) provides type-safe COSE structures with a bring-your-own-crypto design—you supply signing closures using libraries like `k256` for secp256k1. **ciborium** (0.2.2, 93M+ downloads) handles CBOR encoding with automatic deterministic output. Avoid `serde_cbor`, which is unmaintained (RUSTSEC-2021-0127).

## Binary format comparison reveals clear trade-offs

All binary formats support native byte embedding without base64 encoding, but they differ significantly in schema requirements, size, and ecosystem support:

| Format | Rust Crate | Downloads | Size vs JSON | Schema | Serde | Deterministic |
|--------|-----------|-----------|--------------|--------|-------|---------------|
| CBOR | ciborium | 93M | ~60-70% | Optional | ✅ | With care |
| MessagePack | rmp-serde | 65M | ~65% | No | ✅ | No |
| Protobuf | prost | High | ~25-40% | Required | ⚠️ | No |
| Borsh | borsh | 67M | ~40-50% | No | ❌ | ✅ Yes |
| RLP | alloy-rlp | 628K/mo | ~40-60% | No | ❌ | ✅ Yes |
| FlatBuffers | flatbuffers | 2.5M/mo | 150-200% | Required | ❌ | N/A |
| Cap'n Proto | capnp | 8.6M | ~100-150%* | Required | ❌ | N/A |

*Cap'n Proto with packing achieves ~25-40% of JSON size.

**CBOR stands out for this use case** because it offers the best combination of: native binary support, optional schema flexibility, serde compatibility, deterministic encoding capability (critical for content-addressable storage), and direct integration with COSE for signing. Protobuf achieves smaller sizes but lacks deterministic encoding guarantees and requires schema management.

**Borsh** (used by Solana/NEAR) is worth considering if you need guaranteed deterministic output—it was specifically designed for hashing consistency. However, it intentionally avoids serde compatibility for performance, requiring trait implementations on all types.

## Compression strategy significantly impacts storage costs

For SQLite changesets, **zstd with a trained dictionary** provides the best compression. Binary formats like CBOR still benefit from compression, typically achieving **30-50% additional reduction**:

| Data Type | zstd Ratio | With Dictionary |
|-----------|-----------|-----------------|
| JSON changeset | 25-35% | 20-30% |
| CBOR changeset | 40-50% | 30-40% |
| Raw SQLite changeset | 45-55% | 35-45% |

The `zstd` crate (binding to the official C library) achieves **~300-500 MB/s compression** at level 3 and **~1000+ MB/s decompression**. For small changesets under 10KB, dictionary training on sample data improves ratios by 10-30%.

**Compress after serialization**, not before. This maintains deterministic content hashes and allows dictionary training on the serialized format's byte patterns. For content-addressable storage, compute the hash on uncompressed serialized bytes, then compress for storage—include the content hash in metadata.

## Dual-target storage requires careful format choices

For GCS + Arweave storage, the key insight is that **Arweave is signature-addressed, not content-addressed**—the same content uploaded twice gets different transaction IDs because signatures differ. If you need consistent content addressing:

1. Compute a content hash (SHA-256) of your serialized data
2. Store this hash as an Arweave tag (`Content-Hash`) and GCS metadata
3. Optionally compute an IPFS CID for cross-system content addressing

**Recommended pipeline for your sequencer:**

```
SQLite Changeset
    → CBOR serialize (deterministic, ciborium)
    → COSE_Sign1 wrap (secp256k1 signature, coset)
    → zstd compress (with trained dictionary)
    → Store to GCS (with Arweave TX ID in metadata after upload)
    → Store to Arweave via Irys (with Content-Type, Schema-Version tags)
```

Set consistent `Content-Type` tags/metadata on both systems (`application/cbor` or a custom type like `application/x-sqlite-changeset`). For compressed data, add `Content-Encoding: zstd`.

## Schema evolution favors envelope patterns

For signed payloads that may evolve, use an explicit version field in your outer structure:

```rust
struct SignedChangeset {
    version: u32,           // Schema version for forward compatibility
    payload: Vec<u8>,       // CBOR-encoded inner data
    signature: [u8; 64],    // secp256k1 signature over version + payload
}
```

CBOR handles unknown fields gracefully during decoding, and integer-keyed maps enable efficient field numbering similar to protobuf. For Arweave, include a `Schema-Version` tag for discoverability via GraphQL queries.

## Conclusion

For a blockchain sequencer targeting both GCS and Arweave, the optimal stack is **CBOR serialization + COSE_Sign1 signing + zstd compression**. This achieves ~40-50% size reduction over JSON+base64, uses the standardized ES256K algorithm for secp256k1 signatures, and relies on mature, actively-maintained Rust crates (ciborium, coset, k256, zstd). 

ANS-104 bundles make sense if you're batching many items to Arweave and want payment delegation through Irys, but for simple dual-target storage with your own signing, COSE_Sign1 is simpler and more portable. The bundles-rs crate exists if you later want ANS-104 support—it can wrap COSE-signed payloads in DataItems for Arweave-specific features.