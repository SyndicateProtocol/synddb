# gcp-confidential-space

GCP Confidential Space attestation verification library for SP1 zkVM.

## Overview

This library verifies JWT attestation tokens from Google's Confidential Space TEE environment. It's designed to be compatible with SP1 zero-knowledge proofs and can be used to verify that code is running in a genuine GCP Confidential Space VM.

## Key Differences from AWS Nitro

| Feature | AWS Nitro | GCP Confidential Space |
|---------|-----------|------------------------|
| Format | CBOR/COSE_Sign1 | JWT (JSON Web Token) |
| Signature | P-384 ECDSA | RS256 (RSA-2048 + SHA-256) |
| Identity | PCR values (SHA-384) | image_digest (SHA-256) |
| Trust anchor | AWS root certificate | Google JWKS public keys |

## Features

- `std` (default) - Standard library support
- `sp1` - Enable SP1-specific types (`PublicValuesStruct` with alloy `sol!` macro)

## Usage

### Basic Verification

```rust,norun
use gcp_confidential_space::{verify_gcp_cs_attestation, JwkKey};

let jwk = JwkKey {
    alg: "RS256".into(),
    kid: "d6d5071ab75246a42acfa46d29316311cdab51f7".into(),
    kty: "RSA".into(),
    n: "oXx5rKdo3qd...".into(),  // base64url modulus
    e: "AQAB".into(),              // base64url exponent
    use_: "sig".into(),
};

let result = verify_gcp_cs_attestation(
    jwt_bytes,
    &jwk,
    Some("https://my-audience.example.com"),
    None, // Skip time validation
)?;

println!("Image digest: {}", result.image_digest);
println!("Secure boot: {}", result.secboot);
```

### SP1 Program Usage

The SP1 program is located in `../sp1/program/`. It uses this library with the `sp1` feature:

```toml
[dependencies]
gcp-confidential-space = { path = "../../gcp-confidential-space", default-features = false, features = ["sp1"] }
```

See `../sp1/program/src/main.rs` for the full implementation.

## PublicValuesStruct

The `PublicValuesStruct` is the interface between the SP1 proof and the Solidity verifier contract. It contains hashed versions of the attestation claims:

```solidity
struct PublicValuesStruct {
    bytes32 jwk_key_hash;          // keccak256(kid)
    uint64 validity_window_start;  // iat timestamp
    uint64 validity_window_end;    // exp timestamp
    bytes32 image_digest_hash;     // keccak256(image_digest string)
    address tee_signing_key;       // Zero for GCP CS (no embedded key)
    bool secboot;                  // Secure boot status
    bytes32 audience_hash;         // keccak256(audience string)
}
```

## Sample Data

Attestation samples are available in `../samples/`. These contain:

- `raw_token` - Complete JWT
- `header` - Decoded JWT header (alg, kid, typ)
- `claims` - Decoded attestation claims
- `jwks` - Google's public keys for verification

## Important Claims

| Claim | Description |
|-------|-------------|
| `iss` | Must be `https://confidentialcomputing.googleapis.com` |
| `aud` | Audience requested by workload |
| `exp` | Token expiration (Unix timestamp) |
| `secboot` | Secure boot enabled (should be true) |
| `swname` | Should be `CONFIDENTIAL_SPACE` |
| `submods.container.image_digest` | Container image SHA256 hash |

## Solidity Verifier Contract

Your colleague needs to create a Solidity contract that:

1. Accepts SP1 proofs and decodes `PublicValuesStruct`
2. Maintains a registry of trusted JWKS key hashes (from Google's OIDC discovery)
3. Validates that:
   - `jwk_key_hash` is in the trusted registry
   - `validity_window_end > block.timestamp` (not expired)
   - `secboot == true`
   - `image_digest_hash` matches expected container
   - `audience_hash` matches expected audience

Example pattern:

```solidity,norun
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ISP1Verifier} from "@sp1-contracts/ISP1Verifier.sol";

contract GcpCsAttestationVerifier {
    ISP1Verifier public immutable verifier;
    bytes32 public immutable programVKey;

    mapping(bytes32 => bool) public trustedJwkHashes;
    bytes32 public expectedImageDigestHash;

    struct PublicValuesStruct {
        bytes32 jwk_key_hash;
        uint64 validity_window_start;
        uint64 validity_window_end;
        bytes32 image_digest_hash;
        address tee_signing_key;
        bool secboot;
        bytes32 audience_hash;
    }

    constructor(address _verifier, bytes32 _vkey) {
        verifier = ISP1Verifier(_verifier);
        programVKey = _vkey;
    }

    function verifyAttestation(
        bytes calldata proof,
        bytes calldata publicValues
    ) external view returns (bool) {
        // Verify SP1 proof
        verifier.verifyProof(programVKey, publicValues, proof);

        // Decode public values
        PublicValuesStruct memory values = abi.decode(
            publicValues, (PublicValuesStruct)
        );

        // Validate claims
        require(trustedJwkHashes[values.jwk_key_hash], "Untrusted JWK");
        require(values.validity_window_end > block.timestamp, "Expired");
        require(values.secboot, "Secure boot required");
        require(values.image_digest_hash == expectedImageDigestHash, "Wrong image");

        return true;
    }
}
```

## Testing

```bash
# Run tests
cargo test

# Build with SP1 types
cargo build --features sp1
```

## Generating Proofs

```bash
cd ../sp1/script

# Test execution (fast, no proof)
cargo run --release --bin gcp-cs-prover -- --execute --sample ../../samples/*.json

# Generate ZK proof
cargo run --release --bin gcp-cs-prover -- --prove --sample ../../samples/*.json

# Get verification key for Solidity
cargo run --release --bin gcp-cs-vkey
```
