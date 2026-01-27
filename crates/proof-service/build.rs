//! Build script for proof-service
//!
//! Compiles the RISC Zero program for GCP Confidential Space attestation verification.

fn main() {
    // Build the RISC Zero guest program from the bootstrap crate
    risc0_build::embed_methods();
}
