//! GCP Confidential Space attestation verification library
//!
//! This library verifies JWT attestation tokens from Google's Confidential Space
//! TEE environment. It can optionally be compiled with SP1 support for
//! zero-knowledge proof generation.
//!
//! # Features
//!
//! - `std` (default) - Standard library support
//! - `sp1` - Enable SP1-specific types (PublicValuesStruct with alloy sol! macro)
//!
//! # Key Differences from AWS Nitro
//!
//! | Feature | AWS Nitro | GCP Confidential Space |
//! |---------|-----------|------------------------|
//! | Format | CBOR/COSE_Sign1 | JWT (JSON Web Token) |
//! | Signature | P-384 ECDSA | RS256 (RSA-2048 + SHA-256) |
//! | Identity | PCR values (SHA-384) | image_digest (SHA-256) |
//! | Trust anchor | AWS root certificate | Google JWKS public keys |
//!
//! # Usage
//!
//! ```ignore
//! use gcp_confidential_space::{verify_gcp_cs_attestation, JwkKey};
//!
//! let result = verify_gcp_cs_attestation(
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

extern crate alloc;

pub mod attestation;
pub mod jwt;