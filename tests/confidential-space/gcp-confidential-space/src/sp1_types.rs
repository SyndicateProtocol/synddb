//! SP1-specific types for on-chain verification
//!
//! This module contains the PublicValuesStruct that matches the Solidity definition
//! in the verifier contract.

use alloy::sol;

// SP1 public values struct for on-chain verification.
// Must match the Solidity definition in the verifier contract.
sol! {
    struct PublicValuesStruct {
        // Hash of the JWKS key that signed this token (keccak256 of kid)
        bytes32 jwk_key_hash;
        // Token validity window start (iat - issued at)
        uint64 validity_window_start;
        // Token validity window end (exp - expiration)
        uint64 validity_window_end;
        // Container image digest (keccak256 of the sha256:... string)
        bytes32 image_digest_hash;
        // TEE signing key address (derived from public key in token, if any)
        address tee_signing_key;
        // Whether secure boot was enabled
        bool secboot;
        // Whether debug mode is disabled (dbgstat == "disabled")
        bool dbgstat_disabled;
        // Audience hash (keccak256 of audience string)
        bytes32 audience_hash;
    }
}
