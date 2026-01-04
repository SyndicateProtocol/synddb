//! SP1 program for GCP Confidential Space attestation verification
//!
//! This program runs inside the SP1 zkVM and:
//! 1. Reads a JWT attestation token, JWK public key, and EVM signing key from the prover
//! 2. Verifies the RS256 signature
//! 3. Validates the attestation claims
//! 4. Derives the Ethereum address from the EVM public key
//! 5. Commits the public values (including signer address) for on-chain verification

#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy::{
    primitives::{keccak256, Address},
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

    // Derive Ethereum address from EVM public key: keccak256(pubkey)[12..32]
    let pubkey_hash = keccak256(&evm_public_key);
    let evm_address = Address::from_slice(&pubkey_hash[12..]);

    // Encode public values for on-chain verification
    let bytes = PublicValuesStruct::abi_encode(&PublicValuesStruct {
        jwk_key_hash: keccak256(result.signing_key_id.as_bytes()),
        validity_window_start: result.validity_window_start,
        validity_window_end: result.validity_window_end,
        image_digest_hash: keccak256(result.image_digest.as_bytes()),
        tee_signing_key: evm_address,
        secboot: result.secboot,
        dbgstat_disabled: result.dbgstat == "disabled", // Reject debug mode VMs
        audience_hash: keccak256(result.audience.as_bytes()),
    });

    // Commit public values to the proof
    sp1_zkvm::io::commit_slice(&bytes);
}
