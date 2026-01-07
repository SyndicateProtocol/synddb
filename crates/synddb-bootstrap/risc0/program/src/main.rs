//! RISC Zero program for GCP Confidential Space attestation verification
//!
//! This program runs inside the RISC Zero zkVM and:
//! 1. Reads a JWT attestation token, JWK public key, and EVM signing key from the prover
//! 2. Verifies the RS256 signature on the GCP attestation
//! 3. Validates the attestation claims
//! 4. Parses the image signature (secp256k1) for on-chain verification
//! 5. Derives the Ethereum address from the EVM public key
//! 6. Commits the public values for on-chain verification
//!
//! The image signature (secp256k1 ECDSA over image_digest_hash) is verified on-chain
//! using ecrecover. This allows the smart contract to verify the image was signed
//! by an authorized Ethereum key (e.g., from CI).

#![no_main]

use alloy::{
    primitives::{keccak256, Address, FixedBytes},
    sol_types::SolType,
};
use gcp_attestation::{verify_attestation, JwkKey};
use gcp_cs_attestation_risc0_program::PublicValuesStruct;
use risc0_zkvm::guest::env;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    // Read inputs from the prover (same order as SP1 program)
    let jwt_bytes: Vec<u8> = env::read();
    let jwk: JwkKey = env::read();
    let expected_audience: String = env::read();
    let evm_public_key_vec: Vec<u8> = env::read();

    // Image signature inputs (secp256k1 for on-chain ecrecover verification)
    let image_signature_vec: Vec<u8> = env::read(); // 65 bytes (r || s || v)

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

    // Parse image signature (r || s || v, 65 bytes total)
    let (sig_v, sig_r, sig_s) = parse_image_signature(&image_signature_vec);

    // Derive Ethereum address from EVM public key: keccak256(pubkey)[12..32]
    let pubkey_hash = keccak256(&evm_public_key);
    let evm_address = Address::from_slice(&pubkey_hash[12..]);

    // Encode public values for on-chain verification
    // The on-chain contract will verify the image signature using ecrecover
    let bytes = PublicValuesStruct::abi_encode(&PublicValuesStruct {
        jwk_key_hash: keccak256(result.signing_key_id.as_bytes()),
        validity_window_start: result.validity_window_start,
        validity_window_end: result.validity_window_end,
        image_digest_hash: keccak256(result.image_digest.as_bytes()),
        tee_signing_key: evm_address,
        secboot: result.secboot,
        dbgstat_disabled: result.dbgstat == "disabled-since-boot", // Only accept production VMs
        audience_hash: keccak256(result.audience.as_bytes()),
        image_signature_v: sig_v,
        image_signature_r: FixedBytes::from(sig_r),
        image_signature_s: FixedBytes::from(sig_s),
    });

    // Commit public values to the journal (RISC Zero's equivalent of SP1's commit_slice)
    env::commit_slice(&bytes);
}

/// Parse image signature from raw bytes (r || s || v, 65 bytes total)
/// This is the standard Ethereum signature format
fn parse_image_signature(signature_bytes: &[u8]) -> (u8, [u8; 32], [u8; 32]) {
    assert!(
        signature_bytes.len() == 65,
        "Image signature must be exactly 65 bytes (r || s || v)"
    );

    let r: [u8; 32] = signature_bytes[0..32].try_into().unwrap();
    let s: [u8; 32] = signature_bytes[32..64].try_into().unwrap();
    let v: u8 = signature_bytes[64];

    (v, r, s)
}
