//! GCP Confidential Space SP1 attestation program types
//!
//! This crate contains the SP1 program for verifying GCP Confidential Space attestations,
//! along with the public types needed for proof generation and verification.
//!
//! # Usage
//!
//! External crates (like the proof-service) can depend on this crate to access
//! `PublicValuesStruct` for decoding proof outputs:
//!
//! ```toml
//! [dependencies]
//! gcp-cs-attestation-sp1-program = { path = "...", default-features = false }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

mod types;

pub use types::PublicValuesStruct;
