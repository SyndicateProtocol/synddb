//! Outputs the verification key for the GCP CS attestation program.
//! This is needed for the Solidity verifier contract.

use sp1_sdk::{include_elf, HashableKey, Prover, ProverClient};

pub const GCP_CS_ATTESTATION_ELF: &[u8] = include_elf!("gcp-cs-attestation-sp1-program");

fn main() {
    let prover = ProverClient::builder().cpu().build();
    let (_, vk) = prover.setup(GCP_CS_ATTESTATION_ELF);

    println!("Verification Key (bytes32): {}", vk.bytes32());
}
