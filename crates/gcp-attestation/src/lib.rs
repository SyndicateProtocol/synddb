//! GCP Confidential Space attestation verification library
//!
//! This library verifies JWT attestation tokens from Google's Confidential Space
//! TEE environment. It is `no_std` compatible for use in zkVM environments.
//!
//! # Features
//!
//! - `std` (default) - Standard library support
//!
//! # Key Differences from AWS Nitro
//!
//! | Feature | AWS Nitro | GCP Confidential Space |
//! |---------|-----------|------------------------|
//! | Format | `CBOR/COSE_Sign1` | JWT (JSON Web Token) |
//! | Signature | P-384 ECDSA | RS256 (RSA-2048 + SHA-256) |
//! | Identity | PCR values (SHA-384) | `image_digest` (SHA-256) |
//! | Trust anchor | AWS root certificate | Google JWKS public keys |
//!
//! # Usage
//!
//! ```ignore
//! use gcp_attestation::{verify_attestation, JwkKey};
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

extern crate alloc;

mod jwt;
mod verify;

pub use jwt::{
    decode_base64url, parse_jwt, ContainerInfo, GcpCsClaims, JwkKey, JwtHeader, ParsedJwt, SubMods,
};
pub use verify::{
    extract_kid_from_jwt, find_jwk_by_kid, verify_attestation, ValidationResult, VerificationError,
};
