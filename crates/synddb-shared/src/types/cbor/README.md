# CBOR/COSE Signature Verification

This document explains the signature verification approach used in this module.

## Overview

SyndDB uses COSE (CBOR Object Signing and Encryption) with secp256k1 ECDSA signatures
and keccak256 hashing for Ethereum compatibility.

## Signature Format

### COSE Standard: 64 bytes (r || s)

COSE uses the standard ECDSA signature format:
- `r`: 32 bytes
- `s`: 32 bytes

This is the canonical ECDSA output for secp256k1.

### Ethereum Extended: 65 bytes (r || s || v)

Ethereum extends this with a recovery ID:
- `r`: 32 bytes
- `s`: 32 bytes
- `v`: 1 byte (recovery ID)

The recovery ID allows recovering the signer's public key from just the signature
and message hash, which is why Ethereum transactions don't include the sender's
public key.

### Why COSE Uses 64 Bytes

COSE messages include the signer's public key in the unprotected header, so there's
no need for signature recovery. The standard 64-byte format is sufficient.

## Verification Approaches in k256

The k256 crate provides three ways to verify secp256k1 signatures:

### 1. Recovery-Based Verification

```rust,norun
let recid = RecoveryId::try_from(v)?;
let recovered_key = VerifyingKey::recover_from_digest(digest, &signature, recid)?;
assert_eq!(recovered_key, expected_key);
```

**Blocked for us**: Requires the recovery ID (`v`), which COSE signatures don't include.

### 2. Direct `verify()` Method

```rust,norun
verifying_key.verify(message, &signature)?;
```

**Blocked for us**: Internally uses SHA-256, but we need keccak256 for Ethereum
compatibility.

### 3. `DigestVerifier::verify_digest()` (What We Use)

```rust,norun
use sha3::{Keccak256, Digest};
use signature::DigestVerifier;

let digest = Keccak256::new_with_prefix(data);
verifying_key.verify_digest(digest, &signature)?;
```

**This works**: Accepts any `Digest` implementation with 32-byte output, including
Keccak256.

## Why `verify_digest` Over `verify_prehash`

| Aspect | `verify_prehash` | `verify_digest` |
|--------|------------------|-----------------|
| Module | `hazmat` (advanced) | Standard trait |
| Input | Raw bytes | Type-safe `Digest` |
| Safety | Manual hash handling | Compiler-enforced |
| API level | Low-level | Idiomatic |

The `verify_prehash` function lives in the `hazmat` module, signaling it's for
advanced users who understand the security implications. `DigestVerifier` is the
standard, type-safe approach from the `signature` crate.

## Summary

| Constraint | Source | Solution |
|------------|--------|----------|
| 64-byte signatures | COSE standard | Cannot use recovery-based verification |
| keccak256 hashing | Ethereum compatibility | Cannot use `verify()` (uses SHA-256) |
| Both constraints | - | Use `DigestVerifier::verify_digest()` with Keccak256 |

## Dependencies

```toml
k256 = { version = "0.13", features = ["ecdsa"] }
sha3 = "0.10"  # Provides Keccak256
```

The `DigestVerifier` trait is re-exported via `k256::ecdsa::signature::DigestVerifier`.
