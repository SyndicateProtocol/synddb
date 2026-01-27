//! SP1 program for GCP Confidential Space attestation verification
//!
//! This program runs inside the SP1 zkVM and:
//! 1. Reads a JWT attestation token, JWK public key, and EVM signing key from the prover
//! 2. Verifies the RS256 signature on the GCP attestation
//! 3. Validates the attestation claims
//! 4. Parses the cosign signature and public key for on-chain verification
//! 5. Derives the Ethereum address from the EVM public key
//! 6. Commits the public values for on-chain verification
//!
//! The cosign signature (P-256 ECDSA over the image digest) is verified on-chain
//! using the RIP-7212 P256 precompile, not inside the zkVM. This allows the smart
//! contract to verify the image was signed by an authorized cosign key.

#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy::{
    primitives::{keccak256, Address, FixedBytes},
    sol_types::SolType,
};
use gcp_attestation::{verify_attestation, JwkKey};
use gcp_cs_attestation_sp1_program::PublicValuesStruct;

pub fn main() {
    // Read inputs from the prover
    let jwt_bytes: Vec<u8> = sp1_zkvm::io::read();
    let jwk: JwkKey = sp1_zkvm::io::read();
    let expected_audience: String = sp1_zkvm::io::read();
    let evm_public_key_vec: Vec<u8> = sp1_zkvm::io::read();

    // Cosign inputs (signature and pubkey for on-chain verification)
    let cosign_signature_vec: Vec<u8> = sp1_zkvm::io::read(); // 64 bytes (r || s)
    let cosign_pubkey_vec: Vec<u8> = sp1_zkvm::io::read(); // 65 bytes (0x04 || x || y) or 64 bytes (x || y)

    // Convert to fixed-size array (serde doesn't support [u8; 64] directly)
    assert!(
        evm_public_key_vec.len() == 64,
        "EVM public key must be exactly 64 bytes"
    );
    let evm_public_key: [u8; 64] = evm_public_key_vec.try_into().unwrap();

    // Verify the attestation (skip time validation inside zkVM)
    let result = verify_attestation(
        &jwt_bytes,
        &jwk,
        Some(&expected_audience),
        None, // Time validation happens on-chain
    )
    .expect("Invalid GCP Confidential Space attestation");

    // Parse cosign signature (r || s, 64 bytes total)
    let (cosign_sig_r, cosign_sig_s) = parse_cosign_signature(&cosign_signature_vec);

    // Parse cosign public key (x || y, with optional 0x04 prefix)
    let (cosign_pubkey_x, cosign_pubkey_y) = parse_cosign_pubkey(&cosign_pubkey_vec);

    // Derive Ethereum address from EVM public key: keccak256(pubkey)[12..32]
    let pubkey_hash = keccak256(&evm_public_key);
    let evm_address = Address::from_slice(&pubkey_hash[12..]);

    // Encode public values for on-chain verification
    // The on-chain contract will verify the cosign signature using RIP-7212 P256 precompile
    let bytes = PublicValuesStruct::abi_encode(&PublicValuesStruct {
        jwk_key_hash: keccak256(result.signing_key_id.as_bytes()),
        validity_window_start: result.validity_window_start,
        validity_window_end: result.validity_window_end,
        image_digest_hash: keccak256(result.image_digest.as_bytes()),
        tee_signing_key: evm_address,
        secboot: result.secboot,
        dbgstat_disabled: result.dbgstat == "disabled", // Reject debug mode VMs
        audience_hash: keccak256(result.audience.as_bytes()),
        cosign_signature_r: FixedBytes::from(cosign_sig_r),
        cosign_signature_s: FixedBytes::from(cosign_sig_s),
        cosign_pubkey_x: FixedBytes::from(cosign_pubkey_x),
        cosign_pubkey_y: FixedBytes::from(cosign_pubkey_y),
    });

    // Commit public values to the proof
    sp1_zkvm::io::commit_slice(&bytes);
}

/// Parse cosign signature from raw bytes (r || s, 64 bytes total)
fn parse_cosign_signature(signature_bytes: &[u8]) -> ([u8; 32], [u8; 32]) {
    assert!(
        signature_bytes.len() == 64,
        "Cosign signature must be exactly 64 bytes (r || s)"
    );

    let r: [u8; 32] = signature_bytes[0..32].try_into().unwrap();
    let s: [u8; 32] = signature_bytes[32..64].try_into().unwrap();

    (r, s)
}

/// Parse cosign public key from raw bytes
/// Accepts either 65 bytes (0x04 || x || y) or 64 bytes (x || y)
fn parse_cosign_pubkey(pubkey_bytes: &[u8]) -> ([u8; 32], [u8; 32]) {
    if pubkey_bytes.len() == 65 {
        assert!(
            pubkey_bytes[0] == 0x04,
            "Uncompressed public key must start with 0x04"
        );
        (
            pubkey_bytes[1..33].try_into().unwrap(),
            pubkey_bytes[33..65].try_into().unwrap(),
        )
    } else if pubkey_bytes.len() == 64 {
        (
            pubkey_bytes[0..32].try_into().unwrap(),
            pubkey_bytes[32..64].try_into().unwrap(),
        )
    } else {
        panic!("Cosign public key must be 64 or 65 bytes");
    }
}
