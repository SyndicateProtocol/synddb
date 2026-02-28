//! Key bootstrapping for `SyndDB` TEE services
//!
//! This crate provides automated key registration for sequencers and validators
//! running in GCP Confidential Space. When a service starts:
//!
//! 1. Generate ephemeral signing key inside TEE
//! 2. Fetch attestation token from Confidential Space
//! 3. Generate attestation proof (RISC Zero service or Stylus local construction)
//! 4. Sign registration request (EIP-712)
//! 5. Send to relayer for on-chain submission (relayer pays gas)
//! 6. Verify key registration on-chain
//!
//! # Verification Modes
//!
//! - **Service** (default): Uses RISC Zero zkVM proof service for ZK proof generation.
//!   The proof service runs the RISC Zero guest program to verify the JWT and generate
//!   a Groth16 proof, which is verified on-chain by `RiscZeroAttestationVerifier`.
//! - **Stylus**: Constructs proof data locally and sends the raw JWT for direct on-chain
//!   verification by an Arbitrum Stylus contract. The contract verifies the RS256 signature
//!   using SHA-256 and modexp EVM precompiles. No external proof service needed.
//! - **Mock**: For testing only, generates invalid proofs
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
