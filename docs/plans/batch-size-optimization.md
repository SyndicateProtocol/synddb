# Batch File Size Optimization

This document outlines strategies for reducing the size of batch files published to GCS.

## Current State

Batch files are JSON-serialized `SignedBatch` structs containing signed messages. Current file sizes grow linearly with payload size:
- Single message batches: 88KB - 240KB
- The `payload` field (zstd-compressed changesets) uses base64 encoding (~1.33x overhead)

## Implemented

### 1. Base64 Payload Encoding

**Status**: Implemented

Changed `payload` serialization from JSON byte array to base64 string.

- Before: `"payload": [40, 181, 47, 253, ...]` (~4x overhead)
- After: `"payload": "KLUv/QAAA..."` (~1.33x overhead)

Expected reduction: **~3x smaller files**

## Future Strategies

### 2. Compress Entire Batch File

**Effort**: Low

**Benefit**: ~2-3x for single messages, ~5-10x for multi-message batches

Store batch files with compression:
- Option A: `.json.zst` files (zstd compression)
- Option B: `.json.gz` files (gzip, broader tooling support)

GCS can serve compressed content with automatic decompression via `Content-Encoding` headers.

Multi-message batches benefit more because:
- Repetitive JSON structure compresses well
- Identical `signer` fields across messages deduplicate
- Similar payload patterns compress together

### 3. Binary Format (MessagePack/CBOR)

**Effort**: Medium
**Benefit**: ~2x smaller than JSON

Replace JSON with a binary serialization format:
- MessagePack: compact, fast, good Rust support (`rmp-serde`)
- CBOR: standardized (RFC 8949), better for typed data

Tradeoffs:
- Loses human readability (need tooling to inspect)
- Existing batch files become incompatible (need migration or version detection)

### 4. Deduplicate Redundant Fields

**Effort**: Low-Medium
**Benefit**: Modest for single messages, significant for large batches

In multi-message batches:
- `signer` field is identical in every message AND the batch wrapper
- Could move to batch level only and omit from individual messages

### 5. Shorter Field Names

**Effort**: Low
**Benefit**: Minor (~5-10%)

Rename fields for compactness:
- `sequence` -> `seq`
- `timestamp` -> `ts`
- `message_hash` -> `hash`
- `signature` -> `sig`
- `batch_signature` -> `bsig`

Better combined with other changes to avoid churn.

## Recommended: COSE Format

**Effort**: High
**Benefit**: Optimal compactness + standardization

COSE (CBOR Object Signing and Encryption, RFC 8152) is the binary equivalent of JWT/JOSE:

```
COSE_Sign1 = [
    protected_header,   // CBOR-encoded, signed over
    unprotected_header, // Not signed
    payload,            // Raw bytes (no encoding overhead)
    signature           // 64-65 bytes
]
```

Advantages:
- **Standard format** for signed data
- **No encoding overhead** - payload is raw binary
- **Signature verification** is well-defined
- **Compact** - CBOR is already binary
- **Extensible** - custom headers for sequence, timestamp

Rust libraries:
- `coset` - maintained, feature-complete
- `cose-rust` - alternative implementation

Migration approach:
1. Implement COSE writer in sequencer
2. Implement COSE reader in validator
3. Support both formats during transition (detect by file extension or magic bytes)
4. Eventually deprecate JSON format

### COSE Structure for SyndDB

```
SignedMessage as COSE_Sign1:
  protected: {
    alg: ES256K,          // secp256k1
    seq: 42,              // custom: sequence number
    ts: 1700000000,       // custom: timestamp
    type: "changeset"     // custom: message type
  }
  unprotected: {
    signer: 0x...         // Ethereum address (for discovery)
  }
  payload: <raw zstd bytes>
  signature: <65 bytes>

SignedBatch as COSE_Sign:
  protected: {
    alg: ES256K,
    start_seq: 1,
    end_seq: 50,
    created_at: 1700000000
  }
  signatures: [<batch sig>]
  payload: [<array of COSE_Sign1 messages>]
```

## Size Comparison Estimate

For a batch with 50 messages, each with 10KB compressed payload:

| Format | Estimated Size |
|--------|---------------|
| Current JSON (byte array) | ~2.5 MB |
| JSON + base64 | ~750 KB |
| JSON + base64 + zstd | ~150-250 KB |
| COSE | ~500 KB uncompressed |
| COSE + zstd | ~100-150 KB |

## Implementation Priority

1. **Base64 payload** - Done
2. **Compress batch files** - Easy win, do next
3. **COSE format** - Major improvement, plan for future PR
4. **Field deduplication** - Consider as part of COSE migration
5. **Short field names** - Only if sticking with JSON long-term
