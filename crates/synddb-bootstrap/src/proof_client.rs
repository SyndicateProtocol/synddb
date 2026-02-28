//! Client for proof generation (RISC Zero service or Stylus local construction)

use crate::{BootstrapConfig, BootstrapError, ProverMode};
use alloy::{
    primitives::{keccak256, Address, FixedBytes},
    sol,
    sol_types::SolValue,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Timeout for GCP metadata requests
const METADATA_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for JWKS fetch requests
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

sol! {
    /// ABI-encoded attestation claims. Must match the Solidity/Stylus definitions.
    struct PublicValuesStruct {
        bytes32 jwk_key_hash;
        uint64 validity_window_start;
        uint64 validity_window_end;
        bytes32 image_digest_hash;
        address tee_signing_key;
        bool secboot;
        bool dbgstat_disabled;
        bytes32 audience_hash;
        uint8 image_signature_v;
        bytes32 image_signature_r;
        bytes32 image_signature_s;
    }

    /// Proof data for the Stylus verifier: raw JWT + JWK RSA key material.
    struct StylusProofData {
        bytes jwt;
        bytes jwk_modulus;
        bytes jwk_exponent;
    }
}

/// Fetch an identity token from the GCP metadata server for authenticating to Cloud Run.
/// Uses the provided HTTP client to avoid creating new connections for each request.
async fn fetch_identity_token(
    client: &reqwest::Client,
    audience: &str,
) -> Result<String, BootstrapError> {
    let metadata_url = format!(
        "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity?audience={}",
        audience
    );

    let response = client
        .get(&metadata_url)
        .header("Metadata-Flavor", "Google")
        .timeout(METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            BootstrapError::ProofServiceUnavailable(format!(
                "Failed to fetch identity token: {}",
                e
            ))
        })?;

    if !response.status().is_success() {
        return Err(BootstrapError::ProofServiceUnavailable(format!(
            "Failed to fetch identity token: HTTP {}",
            response.status()
        )));
    }

    response.text().await.map_err(|e| {
        BootstrapError::ProofServiceUnavailable(format!("Failed to read identity token: {}", e))
    })
}

/// Request to generate an attestation proof
#[derive(Debug, Clone, Serialize)]
struct ProofRequest {
    /// Raw JWT attestation token from Confidential Space
    pub jwt_token: String,
    /// Expected audience claim
    pub expected_audience: String,
    /// EVM public key (64-byte uncompressed secp256k1, hex-encoded)
    pub evm_public_key: String,
    /// Image signature (65 bytes: r || s || v, hex-encoded)
    /// This is a secp256k1 ECDSA signature over `keccak256(image_digest)` for on-chain ecrecover
    pub image_signature: String,
}

/// Response from the proof service
#[derive(Debug, Clone, Deserialize)]
pub struct ProofResponse {
    /// ABI-encoded `PublicValuesStruct` (hex)
    pub public_values: String,
    /// Proof bytes (hex) - RISC Zero Groth16 proof or ABI-encoded `StylusProofData`
    pub proof_bytes: String,
    /// Derived TEE address for verification
    pub tee_address: String,
}

/// Client for communicating with the proof generation service
#[derive(Debug)]
pub struct ProofClient {
    client: reqwest::Client,
    service_url: String,
    timeout: Duration,
    health_check_timeout: Duration,
    prover_mode: ProverMode,
    google_jwks_url: String,
}

impl ProofClient {
    /// Create a new proof client from config
    pub fn from_config(config: &BootstrapConfig) -> Result<Self, BootstrapError> {
        let service_url = match config.prover_mode {
            ProverMode::Service => config
                .proof_service_url
                .clone()
                .ok_or_else(|| BootstrapError::Config("PROOF_SERVICE_URL is required".into()))?,
            ProverMode::Mock => "mock://localhost".into(),
            // Stylus mode constructs proofs locally; no external service needed
            ProverMode::Stylus => String::new(),
        };

        let client = reqwest::Client::builder()
            .timeout(config.proof_timeout)
            .build()
            .map_err(|e| BootstrapError::ProofServiceUnavailable(e.to_string()))?;

        Ok(Self {
            client,
            service_url,
            timeout: config.proof_timeout,
            health_check_timeout: config.proof_health_check_timeout,
            prover_mode: config.prover_mode,
            google_jwks_url: config.google_jwks_url.clone(),
        })
    }

    /// Generate a proof for the given attestation
    ///
    /// # Arguments
    /// * `jwt_token` - Raw JWT attestation token from Confidential Space
    /// * `expected_audience` - Expected audience claim
    /// * `evm_public_key` - 64-byte uncompressed secp256k1 public key
    /// * `image_signature` - 65-byte secp256k1 signature (r || s || v) over `keccak256(image_digest)`
    pub async fn generate_proof(
        &self,
        jwt_token: &str,
        expected_audience: &str,
        evm_public_key: &[u8; 64],
        image_signature: &[u8],
    ) -> Result<ProofResponse, BootstrapError> {
        // Check for mock mode
        if self.prover_mode == ProverMode::Mock {
            return self.generate_mock_proof(evm_public_key);
        }

        // Stylus mode: construct proof locally (no external service)
        if self.prover_mode == ProverMode::Stylus {
            return self
                .generate_stylus_proof(jwt_token, evm_public_key, image_signature)
                .await;
        }

        let request = ProofRequest {
            jwt_token: jwt_token.to_string(),
            expected_audience: expected_audience.to_string(),
            evm_public_key: format!("0x{}", hex::encode(evm_public_key)),
            image_signature: format!("0x{}", hex::encode(image_signature)),
        };

        info!(
            service_url = %self.service_url,
            timeout_secs = self.timeout.as_secs(),
            "Requesting proof generation"
        );

        // Log attestation sample before sending to proof service (query: "attestation_sample")
        // This data is not sensitive - tokens contain only TEE metadata, no secrets.
        info!(
            event = "attestation_sample",
            source = "proof_client",
            raw_token = %jwt_token,
            audience = %expected_audience,
            "Attestation sample being sent to proof service"
        );

        // Fetch identity token for Cloud Run authentication (reuses client connection pool)
        let identity_token = fetch_identity_token(&self.client, &self.service_url).await?;

        let response = self
            .client
            .post(format!("{}/prove", self.service_url))
            .header("Authorization", format!("Bearer {}", identity_token))
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BootstrapError::ProofGenerationTimeout(self.timeout)
                } else if e.is_connect() {
                    BootstrapError::ProofServiceUnavailable(e.to_string())
                } else {
                    BootstrapError::ProofGenerationFailed(e.to_string())
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();

            // HTTP 400 indicates a permanent error that should NOT be retried
            // (e.g., insufficient PROVE tokens, invalid inputs)
            if status == reqwest::StatusCode::BAD_REQUEST {
                return Err(BootstrapError::ProofGenerationPermanent(body));
            }

            // Other errors (5xx) are transient and may be retried
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "HTTP {status}: {body}"
            )));
        }

        let proof_response: ProofResponse = response
            .json()
            .await
            .map_err(|e| BootstrapError::ProofGenerationFailed(e.to_string()))?;

        info!(
            tee_address = %proof_response.tee_address,
            "Proof generation complete"
        );

        Ok(proof_response)
    }

    /// Check if the proof service is healthy
    pub async fn health_check(&self) -> Result<bool, BootstrapError> {
        if self.prover_mode == ProverMode::Mock || self.prover_mode == ProverMode::Stylus {
            return Ok(true);
        }

        // Fetch identity token for Cloud Run authentication (reuses client connection pool)
        let identity_token = fetch_identity_token(&self.client, &self.service_url).await?;

        let response = self
            .client
            .get(format!("{}/health", self.service_url))
            .header("Authorization", format!("Bearer {}", identity_token))
            .timeout(self.health_check_timeout)
            .send()
            .await
            .map_err(|e| BootstrapError::ProofServiceUnavailable(e.to_string()))?;

        Ok(response.status().is_success())
    }

    /// Generate proof data for Stylus on-chain JWT verification.
    ///
    /// No external service is needed. This method:
    /// 1. Parses the JWT locally to extract claims
    /// 2. Fetches the JWK RSA public key from Google's JWKS endpoint
    /// 3. Constructs the `PublicValuesStruct` and `StylusProofData`
    /// 4. The Stylus contract will verify the JWT RS256 signature on-chain
    async fn generate_stylus_proof(
        &self,
        jwt_token: &str,
        evm_public_key: &[u8; 64],
        image_signature: &[u8],
    ) -> Result<ProofResponse, BootstrapError> {
        info!("Constructing Stylus proof data locally (no external service)");

        // Parse the JWT to extract header and claims
        let parts: Vec<&str> = jwt_token.splitn(3, '.').collect();
        if parts.len() != 3 {
            return Err(BootstrapError::ProofGenerationFailed(
                "Invalid JWT format: expected 3 dot-separated parts".into(),
            ));
        }

        let header_bytes = base64url_decode(parts[0]).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to decode JWT header: {e}"))
        })?;
        let payload_bytes = base64url_decode(parts[1]).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to decode JWT payload: {e}"))
        })?;

        let header: JwtHeader = serde_json::from_slice(&header_bytes).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to parse JWT header: {e}"))
        })?;
        let claims: JwtClaims = serde_json::from_slice(&payload_bytes).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to parse JWT claims: {e}"))
        })?;

        // Derive TEE address from public key
        let pubkey_hash = keccak256(evm_public_key);
        let tee_address = Address::from_slice(&pubkey_hash[12..]);

        // Parse image signature (r || s || v, 65 bytes)
        if image_signature.len() != 65 {
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "Image signature must be 65 bytes, got {}",
                image_signature.len()
            )));
        }
        let sig_r: [u8; 32] = image_signature[0..32].try_into().unwrap();
        let sig_s: [u8; 32] = image_signature[32..64].try_into().unwrap();
        let sig_v: u8 = image_signature[64];

        // Extract image digest from claims
        let image_digest = claims.image_digest().ok_or_else(|| {
            BootstrapError::ProofGenerationFailed(
                "JWT claims missing submods.container.image_digest".into(),
            )
        })?;

        // Construct PublicValuesStruct
        let public_values = PublicValuesStruct {
            jwk_key_hash: keccak256(header.kid.as_bytes()),
            validity_window_start: claims.iat,
            validity_window_end: claims.exp,
            image_digest_hash: keccak256(image_digest.as_bytes()),
            tee_signing_key: tee_address,
            secboot: claims.secboot.unwrap_or(false),
            dbgstat_disabled: claims.dbgstat.as_deref() == Some("disabled-since-boot"),
            audience_hash: keccak256(claims.aud.as_bytes()),
            image_signature_v: sig_v,
            image_signature_r: FixedBytes::from(sig_r),
            image_signature_s: FixedBytes::from(sig_s),
        };
        let public_values_encoded = public_values.abi_encode();

        // Fetch the JWK RSA public key from Google's JWKS endpoint
        let jwk = self.fetch_jwk_by_kid(&header.kid).await?;
        let modulus_bytes = base64url_decode(&jwk.n).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to decode JWK modulus: {e}"))
        })?;
        let exponent_bytes = base64url_decode(&jwk.e).map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to decode JWK exponent: {e}"))
        })?;

        info!(
            kid = %header.kid,
            modulus_len = modulus_bytes.len(),
            "Fetched JWK RSA public key from Google"
        );

        // Construct StylusProofData (raw JWT + JWK key material for on-chain verification)
        let proof_data = StylusProofData {
            jwt: jwt_token.as_bytes().to_vec().into(),
            jwk_modulus: modulus_bytes.into(),
            jwk_exponent: exponent_bytes.into(),
        };
        let proof_bytes_encoded = proof_data.abi_encode();

        info!(
            tee_address = %tee_address,
            "Stylus proof data constructed locally"
        );

        Ok(ProofResponse {
            public_values: format!("0x{}", hex::encode(&public_values_encoded)),
            proof_bytes: format!("0x{}", hex::encode(&proof_bytes_encoded)),
            tee_address: format!("{tee_address}"),
        })
    }

    /// Fetch JWK (JSON Web Key) by key ID from Google's JWKS endpoint.
    async fn fetch_jwk_by_kid(&self, kid: &str) -> Result<JwkKeyResponse, BootstrapError> {
        let response = self
            .client
            .get(&self.google_jwks_url)
            .timeout(JWKS_FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                BootstrapError::ProofGenerationFailed(format!("Failed to fetch JWKS: {e}"))
            })?;

        if !response.status().is_success() {
            return Err(BootstrapError::ProofGenerationFailed(format!(
                "JWKS fetch failed: HTTP {}",
                response.status()
            )));
        }

        let jwks: JwksResponse = response.json().await.map_err(|e| {
            BootstrapError::ProofGenerationFailed(format!("Failed to parse JWKS response: {e}"))
        })?;

        jwks.keys.into_iter().find(|k| k.kid == kid).ok_or_else(|| {
            BootstrapError::ProofGenerationFailed(format!(
                "JWK with kid '{}' not found in JWKS response",
                kid
            ))
        })
    }

    /// Generate a mock proof for testing
    fn generate_mock_proof(
        &self,
        evm_public_key: &[u8; 64],
    ) -> Result<ProofResponse, BootstrapError> {
        warn!("Using MOCK prover - proofs will NOT be valid on-chain");

        // Derive address from public key (same as EvmKeyManager)
        let hash = keccak256(evm_public_key);
        let address = Address::from_slice(&hash[12..]);

        debug!(address = %address, "Generated mock proof");

        // Build ABI-encoded public values with correct tee_address placement
        // PublicValuesStruct has 11 fields x 32 bytes = 352 bytes ABI-encoded
        // Slot 4 (bytes 128-160): tee_signing_key (address is right-aligned, bytes 140-160)
        let mut public_values_bytes = vec![0u8; 352];
        // Place address at bytes 140-160 (right-aligned in 32-byte slot 4)
        public_values_bytes[140..160].copy_from_slice(address.as_slice());

        Ok(ProofResponse {
            public_values: format!("0x{}", hex::encode(&public_values_bytes)),
            proof_bytes: "0x".into(),
            tee_address: format!("{address}"),
        })
    }
}

// -- JWT / JWKS types for local parsing --

/// Minimal JWT header for extracting kid and alg
#[derive(Debug, Deserialize)]
struct JwtHeader {
    kid: String,
    #[allow(dead_code)]
    alg: String,
}

/// Minimal JWT claims for constructing `PublicValuesStruct`
#[derive(Debug, Deserialize)]
struct JwtClaims {
    aud: String,
    iat: u64,
    exp: u64,
    #[serde(default)]
    secboot: Option<bool>,
    #[serde(default)]
    dbgstat: Option<String>,
    #[serde(default)]
    submods: Option<JwtSubmods>,
}

impl JwtClaims {
    fn image_digest(&self) -> Option<&str> {
        self.submods
            .as_ref()
            .and_then(|s| s.container.as_ref())
            .and_then(|c| c.image_digest.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct JwtSubmods {
    container: Option<JwtContainer>,
}

#[derive(Debug, Deserialize)]
struct JwtContainer {
    image_digest: Option<String>,
}

/// Google JWKS response
#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKeyResponse>,
}

/// Individual JWK key from Google's JWKS endpoint
#[derive(Debug, Deserialize)]
struct JwkKeyResponse {
    kid: String,
    /// RSA modulus (base64url-encoded)
    n: String,
    /// RSA exponent (base64url-encoded)
    e: String,
}

/// Decode base64url to bytes, handling missing padding.
fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    // Convert base64url to standard base64
    let mut standard = String::with_capacity(input.len() + 4);
    for c in input.chars() {
        match c {
            '-' => standard.push('+'),
            '_' => standard.push('/'),
            c => standard.push(c),
        }
    }

    // Add padding
    match standard.len() % 4 {
        2 => standard.push_str("=="),
        3 => standard.push('='),
        _ => {}
    }

    base64_decode(&standard)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const DECODE_TABLE: [i8; 128] = [
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 62, -1, -1,
        -1, 63, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, -1, -1, -1, -2, -1, -1, -1, 0, 1, 2, 3, 4,
        5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, -1, -1, -1,
        -1, -1, -1, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
        46, 47, 48, 49, 50, 51, -1, -1, -1, -1, -1,
    ];

    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("Invalid base64 length".into());
    }

    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;

    while i < bytes.len() {
        let a = bytes[i];
        let b = bytes[i + 1];
        let c = bytes[i + 2];
        let d = bytes[i + 3];

        let va = if a < 128 {
            DECODE_TABLE[a as usize]
        } else {
            -1
        };
        let vb = if b < 128 {
            DECODE_TABLE[b as usize]
        } else {
            -1
        };

        if va < 0 || vb < 0 {
            return Err("Invalid base64 character".into());
        }

        output.push(((va as u8) << 2) | ((vb as u8) >> 4));

        if c != b'=' {
            let vc = if c < 128 {
                DECODE_TABLE[c as usize]
            } else {
                -1
            };
            if vc < 0 {
                return Err("Invalid base64 character".into());
            }
            output.push(((vb as u8) << 4) | ((vc as u8) >> 2));

            if d != b'=' {
                let vd = if d < 128 {
                    DECODE_TABLE[d as usize]
                } else {
                    -1
                };
                if vd < 0 {
                    return Err("Invalid base64 character".into());
                }
                output.push(((vc as u8) << 6) | (vd as u8));
            }
        }

        i += 4;
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_prover() {
        let config = BootstrapConfig {
            prover_mode: ProverMode::Mock,
            ..Default::default()
        };

        let client = ProofClient::from_config(&config).unwrap();
        assert_eq!(client.prover_mode, ProverMode::Mock);
    }

    #[test]
    fn test_stylus_prover_no_service_url() {
        let config = BootstrapConfig {
            prover_mode: ProverMode::Stylus,
            ..Default::default()
        };

        let client = ProofClient::from_config(&config).unwrap();
        assert!(client.service_url.is_empty());
        assert_eq!(client.prover_mode, ProverMode::Stylus);
    }

    #[test]
    fn test_base64url_decode() {
        assert_eq!(base64url_decode("SGVsbG8").unwrap(), b"Hello");
        assert_eq!(base64url_decode("PDw_Pz4-").unwrap(), b"<<??>>".to_vec());
    }

    #[test]
    fn test_jwt_claims_parsing() {
        let claims_json = br#"{
            "aud": "https://test.example.com",
            "iat": 1764707757,
            "exp": 1764711357,
            "secboot": true,
            "dbgstat": "disabled-since-boot",
            "submods": {
                "container": {
                    "image_digest": "sha256:abc123"
                }
            }
        }"#;

        let claims: JwtClaims = serde_json::from_slice(claims_json).unwrap();
        assert_eq!(claims.aud, "https://test.example.com");
        assert_eq!(claims.iat, 1764707757);
        assert_eq!(claims.exp, 1764711357);
        assert_eq!(claims.secboot, Some(true));
        assert_eq!(claims.dbgstat.as_deref(), Some("disabled-since-boot"));
        assert_eq!(claims.image_digest(), Some("sha256:abc123"));
    }

    #[test]
    fn test_public_values_abi_encoding() {
        let values = PublicValuesStruct {
            jwk_key_hash: FixedBytes::ZERO,
            validity_window_start: 1000,
            validity_window_end: 2000,
            image_digest_hash: FixedBytes::ZERO,
            tee_signing_key: Address::ZERO,
            secboot: true,
            dbgstat_disabled: true,
            audience_hash: FixedBytes::ZERO,
            image_signature_v: 27,
            image_signature_r: FixedBytes::ZERO,
            image_signature_s: FixedBytes::ZERO,
        };

        let encoded = values.abi_encode();
        // 11 fields x 32 bytes each = 352 bytes
        assert_eq!(encoded.len(), 352);

        let decoded = PublicValuesStruct::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.validity_window_start, 1000);
        assert_eq!(decoded.validity_window_end, 2000);
        assert!(decoded.secboot);
    }

    #[test]
    fn test_stylus_proof_data_abi_encoding() {
        let proof = StylusProofData {
            jwt: b"jwt-bytes".to_vec().into(),
            jwk_modulus: b"modulus".to_vec().into(),
            jwk_exponent: b"exponent".to_vec().into(),
        };

        let encoded = proof.abi_encode();
        let decoded = StylusProofData::abi_decode(&encoded).unwrap();
        assert_eq!(decoded.jwt.as_ref(), b"jwt-bytes");
        assert_eq!(decoded.jwk_modulus.as_ref(), b"modulus");
        assert_eq!(decoded.jwk_exponent.as_ref(), b"exponent");
    }
}
