//! Build script for RISC Zero test script
//!
//! Compiles the RISC Zero guest program for GCP Confidential Space attestation verification.

fn main() {
    risc0_build::embed_methods();
}
