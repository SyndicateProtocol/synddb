//! Bridge signer module for signing messages for the bridge contract
//!
//! When `--bridge-signer` is enabled, the validator signs withdrawal and other
//! messages that can be submitted to the bridge contract by relayers.
//!
//! # Signature Format
//!
//! The bridge contract uses EIP-191 signed messages:
//! ```text
//! keccak256("\x19Ethereum Signed Message:\n32" + messageId)
//! ```
//!
//! Signatures are stored for relayer pickup via the `/signatures/*` API endpoints.

mod signature_store;
mod signer;

pub use signature_store::SignatureStore;
pub use signer::{BridgeSigner, MessageSignature};
