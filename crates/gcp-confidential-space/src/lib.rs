//! GCP Confidential Space attestation verification library with SP1 support
//!
//! This library provides attestation verification for GCP Confidential Space TEE tokens.
//! It wraps the core `gcp-attestation` crate and adds SP1-specific types for zero-knowledge
//! proof generation.
//!
//! # Features
//!
//! - `std` (default) - Standard library support
//! - `sp1` - Enable SP1-specific types (PublicValuesStruct with alloy sol! macro)
//!
//! # Usage
//!
//! ```ignore
//! use gcp_confidential_space::{verify_attestation, JwkKey};
//!
//! let result = verify_attestation(
//!     &jwt_bytes,
//!     &jwk_key,
//!     Some("https://my-audience.example.com"),
//!     None, // Skip time validation for testing
//! )?;
//!
//! println!("Image digest: {}", result.image_digest);
//! println!("Secure boot: {}", result.secboot);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

// Re-export everything from the shared gcp-attestation crate
pub use gcp_attestation::*;

// SP1-specific module containing PublicValuesStruct
#[cfg(feature = "sp1")]
pub mod sp1_types;

// Re-export SP1 types at the crate root for convenience
#[cfg(feature = "sp1")]
pub use sp1_types::PublicValuesStruct;
