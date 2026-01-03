//! SP1 proof generation wrapper

use alloy::sol_types::SolValue;
use anyhow::{Context, Result};
use gcp_attestation::{extract_kid_from_jwt, JwkKey};
use gcp_cs_attestation_sp1_program::PublicValuesStruct;
use sp1_sdk::{include_elf, ProverClient, SP1ProofWithPublicValues, SP1Stdin};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// The ELF file for the GCP CS attestation verification program
pub const GCP_CS_ATTESTATION_ELF: &[u8] = include_elf!("gcp-cs-attestation-sp1-program");

/// Wrapper around SP1 prover for attestation proofs
pub struct AttestationProver {
    client: ProverClient,
}

impl AttestationProver {
    /// Create a new prover using environment configuration
    ///
    /// The SP1_PROVER environment variable controls which prover backend to use:
    /// - Not set or "local": Use local CPU prover
    /// - "cuda": Use local GPU prover (requires CUDA)
    /// - "network": Use SP1 Network Prover (requires SP1_PRIVATE_KEY)
    pub fn new() -> Self {
        info!("Initializing SP1 prover from environment");
        Self {
            client: ProverClient::from_env(),
        }
    }

    /// Generate a proof for the given attestation token
    ///
    /// # Arguments
    /// * `jwt_token` - Raw JWT attestation token from Confidential Space
    /// * `jwk` - JWK public key that signed the token
    /// * `expected_audience` - Expected audience claim
    ///
    /// # Returns
    /// The SP1 proof with public values
    pub fn generate_proof(
        &self,
        jwt_token: &str,
        jwk: &JwkKey,
        expected_audience: &str,
    ) -> Result<SP1ProofWithPublicValues> {
        info!("Starting proof generation");

        // Prepare inputs for the zkVM
        let mut stdin = SP1Stdin::new();
        stdin.write(&jwt_token.as_bytes().to_vec());
        stdin.write(jwk);
        stdin.write(&expected_audience.to_string());

        // Setup proving and verification keys
        debug!("Setting up proving keys");
        let (pk, vk) = self.client.setup(GCP_CS_ATTESTATION_ELF);

        // Generate proof
        info!("Generating ZK proof (this may take several minutes)");
        let proof = self
            .client
            .prove(&pk, &stdin)
            .run()
            .context("Proof generation failed")?;

        // Verify the proof locally before returning
        debug!("Verifying proof locally");
        self.client
            .verify(&proof, &vk)
            .context("Local proof verification failed")?;

        info!("Proof generated and verified successfully");
        Ok(proof)
    }

    /// Execute the program without generating a proof (for testing)
    pub fn execute(&self, jwt_token: &str, jwk: &JwkKey, expected_audience: &str) -> Result<Vec<u8>> {
        info!("Executing program in test mode");

        let mut stdin = SP1Stdin::new();
        stdin.write(&jwt_token.as_bytes().to_vec());
        stdin.write(jwk);
        stdin.write(&expected_audience.to_string());

        let (output, report) = self
            .client
            .execute(GCP_CS_ATTESTATION_ELF, &stdin)
            .run()
            .context("Program execution failed")?;

        info!(cycles = report.total_instruction_count(), "Program executed");
        Ok(output.to_vec())
    }
}

/// JWKS cache for fetching Google's public keys
pub struct JwksCache {
    keys: Arc<RwLock<Option<CachedJwks>>>,
    discovery_url: String,
    cache_ttl_secs: u64,
    client: reqwest::Client,
}

struct CachedJwks {
    keys: Vec<JwkKey>,
    fetched_at: std::time::Instant,
}

#[derive(serde::Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

#[derive(serde::Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

impl JwksCache {
    /// Create a new JWKS cache
    pub fn new(discovery_url: String, cache_ttl_secs: u64) -> Self {
        Self {
            keys: Arc::new(RwLock::new(None)),
            discovery_url,
            cache_ttl_secs,
            client: reqwest::Client::new(),
        }
    }

    /// Get the JWK for a given key ID
    pub async fn get_jwk(&self, kid: &str) -> Result<JwkKey> {
        let keys = self.get_keys().await?;
        keys.into_iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| anyhow::anyhow!("JWK not found for kid: {}", kid))
    }

    /// Get all cached keys, refreshing if needed
    async fn get_keys(&self) -> Result<Vec<JwkKey>> {
        // Check cache
        {
            let cache = self.keys.read().await;
            if let Some(ref cached) = *cache {
                if cached.fetched_at.elapsed().as_secs() < self.cache_ttl_secs {
                    debug!("Using cached JWKS");
                    return Ok(cached.keys.clone());
                }
            }
        }

        // Fetch fresh JWKS
        info!("Fetching fresh JWKS from Google");

        // Get JWKS URI from discovery document
        let discovery: OidcDiscovery = self
            .client
            .get(&self.discovery_url)
            .send()
            .await
            .context("Failed to fetch OIDC discovery document")?
            .json()
            .await
            .context("Failed to parse OIDC discovery document")?;

        // Fetch JWKS
        let jwks: JwksResponse = self
            .client
            .get(&discovery.jwks_uri)
            .send()
            .await
            .context("Failed to fetch JWKS")?
            .json()
            .await
            .context("Failed to parse JWKS")?;

        info!(count = jwks.keys.len(), "Fetched JWKS keys");

        // Update cache
        let keys = jwks.keys.clone();
        {
            let mut cache = self.keys.write().await;
            *cache = Some(CachedJwks {
                keys: jwks.keys,
                fetched_at: std::time::Instant::now(),
            });
        }

        Ok(keys)
    }
}

/// Extract the key ID from a JWT token
pub fn get_jwt_kid(jwt_token: &str) -> Result<String> {
    extract_kid_from_jwt(jwt_token.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to extract kid from JWT: {:?}", e))
}

/// Decode public values from proof output
pub fn decode_public_values(output: &[u8]) -> Result<PublicValuesStruct> {
    PublicValuesStruct::abi_decode_validate(output)
        .map_err(|e| anyhow::anyhow!("Failed to decode public values: {:?}", e))
}
