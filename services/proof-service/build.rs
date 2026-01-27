//! Build script for proof-service
//!
//! Compiles the SP1 program for GCP Confidential Space attestation verification.

use sp1_build::build_program_with_args;

fn main() {
    // Build the SP1 program from the bootstrap crate
    build_program_with_args(
        "../../crates/synddb-bootstrap/sp1/program",
        Default::default(),
    )
}
