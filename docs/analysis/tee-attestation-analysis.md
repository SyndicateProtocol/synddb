# TEE Attestation Verification Analysis

**Date:** 2025-01-02
**Scope:** Smart contract + zkVM + Rust implementation
**Status:** Pre-production review

## Executive Summary

The SyndDB TEE attestation system uses SP1 zkVM proofs for on-chain verification of GCP Confidential Space attestations. While the foundation is solid, several security gaps exist that should be addressed before production deployment.

### Key Findings

| Finding | Severity | Status |
|---------|----------|--------|
| Debug mode (`dbgstat`) not verified | High | ✅ FIXED |
| Sequencer JWT signatures not verified | High | ✅ FIXED |
| JWKS key rotation is manual | Medium | Manual process |
| `tee_signing_key` always zero for GCP CS | Medium | Known limitation |
| Replay protection is time-based only | Low | Acceptable |

---

## Architecture Overview

```
┌─────────────────┐     ┌──────────────────┐     ┌────────────────────┐
│  GCP Conf Space │────▶│    Sequencer     │────▶│  On-Chain Contract │
│  (TEE Client)   │     │  (JWT Receiver)  │     │  (SP1 Verifier)    │
└─────────────────┘     └──────────────────┘     └────────────────────┘
        │                        │                        │
        │ JWT Token              │ Simplified             │ Full verification
        │ (signed by Google)     │ validation only        │ via zkVM proof
        │                        │                        │
        ▼                        ▼                        ▼
   RS256 signature         Claims parsing           SP1 proof of:
   from JWKS keys          Issuer/audience          - RS256 signature
                           Expiration               - All claims
                                                    - secboot=true
```

### Components

1. **Client** (`crates/synddb-client/src/attestation.rs`)
   - Obtains attestation tokens from GCP Confidential Space
   - Communicates via Unix socket at `/run/container_launcher/teeserver.sock`
   - Supports token caching (50 minutes for 1-hour tokens)

2. **Sequencer** (`crates/synddb-sequencer/src/attestation.rs`)
   - Receives tokens from clients
   - Performs simplified JWT validation (no signature verification)
   - Validates issuer, audience, expiration

3. **SP1 Program** (`tests/confidential-space/sp1/program/src/main.rs`)
   - Runs inside zkVM
   - Performs full RS256 signature verification
   - Commits public values for on-chain verification

4. **Smart Contract** (`contracts/src/attestation/AttestationVerifier.sol`)
   - Verifies SP1 proofs
   - Validates public values (JWK hash, timestamps, secboot, image digest)
   - Manages trusted JWK key hashes

---

## Detailed Analysis

### 1. Debug Mode Verification

#### Current Implementation

The contract checks `secboot` (secure boot) but **not** `dbgstat` (debug status):

```solidity
// AttestationVerifier.sol:109-111
if (!values.secboot) {
    revert SecureBootRequired();
}
```

#### The Problem

GCP Confidential Space has two relevant claims:

| Claim | Purpose | Example Values |
|-------|---------|----------------|
| `secboot` | Secure boot chain verified | `true` / `false` |
| `dbgstat` | Debug mode status | `"disabled"` / `"enabled"` |

A VM deployed with `--image-family=confidential-space-debug` will have:
- `secboot: true` (secure boot still works)
- `dbgstat: "enabled"` (debug features active)

Debug VMs have additional attack surface:
- SSH access enabled
- Verbose logging
- Potential for memory inspection

#### Evidence from Samples

```json
// tests/confidential-space/samples/samples_20251202_20_50_37.json:12
"dbgstat": "enabled"
```

The `dbgstat` claim is extracted in the Rust code but never verified:

```rust
// tests/confidential-space/gcp-confidential-space/src/attestation.rs:166
dbgstat: parsed.claims.dbgstat.clone(),
```

#### Recommendation

Add `dbgstat` verification to both the SP1 program and smart contract.

---

### 2. Sequencer JWT Signature Verification

#### Current Implementation

The sequencer performs **simplified validation only**:

```rust
// crates/synddb-sequencer/src/attestation.rs:127-133
/// This performs a simplified verification that:
/// 1. Decodes the JWT (without full cryptographic verification in this version)
/// 2. Validates the issuer and audience claims
/// 3. Checks token expiration
///
/// For production use, full JWKS-based signature verification should be implemented.
```

#### The Problem

Without signature verification, an attacker could:
1. Create a forged JWT with arbitrary claims
2. Submit it to the sequencer
3. The sequencer would accept it based on claims alone

While on-chain verification would eventually catch this (if the attacker tries to register a key), the sequencer should not accept unverified tokens.

#### Recommendation

Use the existing `gcp-confidential-space` library which has full RS256 verification:

```rust
// tests/confidential-space/gcp-confidential-space/src/attestation.rs:171-203
fn verify_rs256_signature(parsed: &ParsedJwt, jwk: &JwkKey) -> Result<(), VerificationError>
```

---

### 3. JWKS Key Rotation

#### Current Implementation

Trusted JWK keys are managed via hash allowlist:

```solidity
// AttestationVerifier.sol:42
mapping(bytes32 jwkHash => bool isTrusted) public trustedJwkHashes;

// Adding keys requires owner
function addTrustedJwkHash(bytes32 jwkHash) external onlyOwner {
    trustedJwkHashes[jwkHash] = true;
    emit TrustedJwkHashAdded(jwkHash);
}
```

#### The Problem

Google rotates JWKS keys periodically. When rotation occurs:
1. New attestation tokens will have a different `kid` (key ID)
2. The `jwk_key_hash` will be different
3. Contract will reject with `UntrustedJwkHash`
4. Manual intervention required to add new key hash

#### Current JWKS Endpoint

```
https://confidentialcomputing.googleapis.com/.well-known/openid-configuration
  └─▶ jwks_uri: https://www.googleapis.com/service_accounts/v1/metadata/jwk/...
```

#### Recommendation

1. Create monitoring for JWKS endpoint changes
2. Document key rotation runbook
3. Consider automation with timelock for key additions

---

### 4. TEE Signing Key (GCP CS Limitation)

#### Current Implementation

The SP1 program always sets `tee_signing_key` to zero for GCP CS:

```rust
// tests/confidential-space/sp1/program/src/main.rs:37
tee_signing_key: alloy::primitives::Address::ZERO, // GCP CS doesn't embed a signing key
```

#### The Problem

GCP Confidential Space JWT tokens don't contain an embedded signing key that can be extracted as an Ethereum address. This differs from other TEE providers (e.g., AWS Nitro, Intel SGX DCAP).

As a result, `TeeKeyManager.addKey()` registers `address(0)` as a valid key, which is not useful for key-based authentication.

#### Current Workaround

The contract notes this limitation:

```solidity
// AttestationVerifier.sol:83-85
/// For GCP Confidential Space, the tee_signing_key field is always address(0)
/// because GCP CS JWT tokens do not contain an embedded signing key.
/// This field exists for compatibility with other TEE providers (e.g., AWS Nitro).
```

#### Recommendation

This requires a separate design decision:
- Option A: Derive a key from image digest + instance identity
- Option B: Use a different registration mechanism for GCP CS
- Option C: Have the TEE generate and attest to a signing key

**Deferred for separate implementation.**

---

### 5. Time Validation

#### Current Implementation

Time validation is split between zkVM and on-chain:

```rust
// SP1 program skips time validation
verify_gcp_cs_attestation(
    &jwt_bytes,
    &jwk,
    Some(&expected_audience),
    None, // Time validation happens on-chain
)
```

```solidity
// On-chain validation
if (block.timestamp < values.validity_window_start) {
    revert ValidityWindowNotStarted(...);
}

if (block.timestamp > values.validity_window_end + expirationTolerance) {
    revert ValidityWindowExpired(...);
}
```

#### Analysis

This is the **correct approach** because:
1. zkVM execution is deterministic and cannot access current time
2. On-chain verification happens at the actual submission time
3. The `expirationTolerance` (max 1 day) provides flexibility for proof generation delay

**No changes needed.**

---

### 6. Replay Protection

#### Current Implementation

Replay protection relies solely on token expiration:
- Tokens are valid for ~1 hour (configurable by GCP)
- Contract adds `expirationTolerance` (up to 1 day max)
- Same token can be replayed within validity window

#### Nonce Support

The client supports nonces but they're not verified on-chain:

```rust
// crates/synddb-client/src/attestation.rs:152-176
pub async fn get_token_with_nonces(&self, nonces: &[&[u8]]) -> Result<String>
```

```rust
// Nonce is in ValidationResult but not in PublicValuesStruct
pub struct ValidationResult {
    // ...
    pub nonce: Option<String>,
}
```

#### Analysis

For the current use case (TEE key registration), time-based replay protection is likely sufficient:
- Keys are registered once
- Duplicate registration fails with `KeyAlreadyExists`
- Token reuse within validity window doesn't grant additional access

#### Recommendation

Consider nonce verification if:
- Attestations are used for per-transaction authorization
- Higher replay protection is needed

---

### 7. Missing Claims in Public Values

#### Current PublicValuesStruct (UPDATED)

```solidity
struct PublicValuesStruct {
    bytes32 jwk_key_hash;
    uint64 validity_window_start;
    uint64 validity_window_end;
    bytes32 image_digest_hash;
    address tee_signing_key;
    bool secboot;
    bool dbgstat_disabled;  // NEW: blocks debug mode VMs
    bytes32 audience_hash;
}
```

#### Missing Claims

| Claim | Purpose | Risk if Missing |
|-------|---------|-----------------|
| ~~`dbgstat`~~ | ~~Debug mode~~ | ✅ IMPLEMENTED |
| `hwmodel` | Hardware model | Non-TEE hardware accepted |
| `swversion` | Software version | Outdated versions accepted |
| `oemid` | OEM identifier | Different TEE providers mixed |

#### Recommendation

Consider `hwmodel` for hardware verification.

---

## Test Coverage

### Current Tests

`contracts/test/attestation/AttestationVerifierTest.t.sol`:

| Test | Coverage |
|------|----------|
| `test_VerifyAttestationProof_RevertsOnSecureBootDisabled` | secboot=false rejection |
| `test_VerifyAttestationProof_RevertsOnDebugModeEnabled` | dbgstat_disabled=false rejection ✅ NEW |
| `test_VerifyAttestationProof_RevertsOnImageDigestMismatch` | Image validation |
| `test_VerifyAttestationProof_RevertsOnValidityWindowExpired` | Time bounds |
| `test_VerifyAttestationProof_RevertsOnUntrustedJwkHash` | JWK validation |
| `test_VerifyAttestationProof_RevertsOnSP1VerificationFailure` | Proof verification |

### Missing Tests

- Hardware model verification
- Multiple concurrent key registrations
- Key revocation timing
- JWKS rotation scenarios

---

## Recommendations Summary

### Priority 1 (High) - IMPLEMENTED

1. **Add `dbgstat` verification** ✅ DONE
   - Updated SP1 program to include `dbgstat_disabled` in public values
   - Updated contract to reject when `dbgstat_disabled == false`
   - Added contract test `test_VerifyAttestationProof_RevertsOnDebugModeEnabled`

2. **Implement sequencer JWT signature verification** ✅ DONE
   - Full RS256 signature verification using `rsa` crate
   - JWKS fetching from Google's OIDC discovery endpoint
   - 1-hour JWKS cache to avoid repeated fetches
   - `dbgstat` and `secboot` verification when `verify_tee_claims` enabled

### Priority 2 (Medium)

3. **Add `hwmodel` verification**
   - Verify hardware is `GCP_AMD_SEV` or other approved TEE
   - Reject non-TEE or simulated environments

4. **Document JWKS rotation process**
   - Create operational runbook
   - Set up monitoring for key changes
   - Consider automation

### Priority 3 (Low)

5. **Consider nonce-based replay protection**
   - Only if per-transaction attestation needed

6. **Address `tee_signing_key` limitation**
   - Separate design effort required

---

## Appendix: Code Locations

| Component | Path |
|-----------|------|
| Client attestation | `crates/synddb-client/src/attestation.rs` |
| Sequencer attestation | `crates/synddb-sequencer/src/attestation.rs` |
| GCP CS library | `tests/confidential-space/gcp-confidential-space/src/` |
| SP1 program | `tests/confidential-space/sp1/program/src/main.rs` |
| AttestationVerifier | `contracts/src/attestation/AttestationVerifier.sol` |
| TeeKeyManager | `contracts/src/attestation/TeeKeyManager.sol` |
| Contract tests | `contracts/test/attestation/AttestationVerifierTest.t.sol` |
