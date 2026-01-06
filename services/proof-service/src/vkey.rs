//! Outputs the SP1 verification key for the GCP Confidential Space attestation program.
//!
//! The vkey is needed for deploying the `AttestationVerifier` Solidity contract.
//! It uniquely identifies the SP1 program and is used on-chain to verify proofs.
//!
//! Usage:
//! ```bash
//! cargo run --release --bin sp1-vkey
//! ```

use sp1_sdk::{include_elf, HashableKey, Prover, ProverClient};

/// The ELF file for the GCP CS attestation verification program
const GCP_CS_ATTESTATION_ELF: &[u8] = include_elf!("gcp-cs-attestation-sp1-program");

fn main() {
    let client = ProverClient::builder().cpu().build();
    let (_, vk) = client.setup(GCP_CS_ATTESTATION_ELF);

    println!("Verification Key (bytes32): {}", vk.bytes32());
}
