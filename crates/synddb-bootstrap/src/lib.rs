//! Key bootstrapping for `SyndDB` TEE services
//!
//! This crate provides automated key registration for sequencers and validators
//! running in GCP Confidential Space. When a service starts:
//!
//! 1. Generate ephemeral signing key inside TEE
//! 2. Fetch attestation token from Confidential Space
//! 3. Request SP1 proof from GPU proof service
//! 4. Submit proof to `TeeKeyManager` contract
//! 5. Wait for on-chain confirmation
//!
//! # Usage
//!
//! ```ignore
//! use synddb_bootstrap::{BootstrapConfig, BootstrapStateMachine};
//!
//! let config = BootstrapConfig::parse();
//! let mut bootstrap = BootstrapStateMachine::new();
//!
//! // This blocks until key is registered on-chain
//! let key_manager = bootstrap.run(&config).await?;
//!
//! // Service is now ready to accept requests
//! ```

mod config;
mod drain;
mod error;
mod funding;
mod proof_client;
mod state_machine;
mod submitter;

pub use config::{BootstrapConfig, ProverMode};
pub use drain::drain_to_treasury;
pub use error::BootstrapError;
pub use funding::{FundingClient, FundingRequest, FundingResponse};
pub use proof_client::{ProofClient, ProofResponse};
pub use state_machine::{BootstrapState, BootstrapStateMachine};
pub use submitter::ContractSubmitter;
