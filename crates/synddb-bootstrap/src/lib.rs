//! Key bootstrapping for `SyndDB` TEE services
//!
//! This crate provides automated key registration for sequencers and validators
//! running in GCP Confidential Space. When a service starts:
//!
//! 1. Generate ephemeral signing key inside TEE
//! 2. Fetch attestation token from Confidential Space
//! 3. Request RISC Zero proof from proof service
//! 4. Sign registration request (EIP-712)
//! 5. Send to relayer for on-chain submission (relayer pays gas)
//! 6. Verify key registration on-chain
//!
//! # Usage
//!
//! ```ignore
//! use synddb_bootstrap::{BootstrapConfig, BootstrapStateMachine};
//!
//! let config = BootstrapConfig::parse();
//! let mut bootstrap = BootstrapStateMachine::for_sequencer();
//!
//! // This blocks until key is registered on-chain
//! let key_manager = bootstrap.run(&config).await?;
//!
//! // Service is now ready to accept requests
//! ```

mod config;
mod drain;
mod error;
mod proof_client;
mod relayer_client;
mod state_machine;
mod submitter;

pub use config::{BootstrapConfig, ProverMode};
pub use drain::drain_to_treasury;
pub use error::BootstrapError;
pub use proof_client::{ProofClient, ProofResponse};
pub use relayer_client::{KeyType, RegisterKeyResponse, RelayerClient};
pub use state_machine::{BootstrapState, BootstrapStateMachine};
pub use submitter::ContractSubmitter;
