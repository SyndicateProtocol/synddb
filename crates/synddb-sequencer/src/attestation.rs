//! TEE attestation verification for incoming requests
//!
//! Verifies that clients are running in a trusted execution environment (TEE)
//! by validating their attestation tokens. Currently supports GCP Confidential Space
//! OIDC tokens.
//!
//! Attestation tokens are JWTs signed by Google's attestation service. The verification
//! process:
//! 1. Fetch Google's OIDC discovery document to get the JWKS URL
//! 2. Fetch the JSON Web Key Set (JWKS) from Google
//! 3. Verify the JWT signature using the appropriate RSA key
//! 4. Validate standard claims (iss, aud, exp, iat)
//! 5. Validate TEE-specific claims (secboot, dbgstat, image digest)

use base64::Engine;
use rsa::{
    pkcs1v15::{Signature, VerifyingKey},
    signature::Verifier,
    BigUint, RsaPublicKey,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Google's OIDC discovery endpoint for Confidential Space
const GOOGLE_OIDC_DISCOVERY_URL: &str =
    "https://confidentialcomputing.googleapis.com/.well-known/openid-configuration";

/// JWKS cache duration (1 hour)
const JWKS_CACHE_DURATION: Duration = Duration::from_secs(3600);

/// Errors that can occur during attestation verification
#[derive(Debug, Error)]
pub enum AttestationError {
    #[error("Token verification failed: {0}")]
    VerificationFailed(String),

    #[error("Invalid token format: {0}")]
    InvalidFormat(String),

    #[error("Token expired")]
    Expired,

    #[error("Token not yet valid")]
    NotYetValid,

    #[error("Invalid issuer: expected {expected}, got {actual}")]
    InvalidIssuer { expected: String, actual: String },

    #[error("Invalid audience: expected {expected}, got {actual}")]
    InvalidAudience { expected: String, actual: String },

    #[error("Failed to fetch JWKS: {0}")]
    JwksFetchError(String),

    #[error("No matching key found in JWKS for kid: {0}")]
    NoMatchingKey(String),

    #[error("Signature verification failed: {0}")]
    SignatureError(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Secure boot not enabled")]
    SecureBootRequired,

    #[error("Debug mode is enabled (dbgstat != disabled)")]
    DebugModeEnabled,

    #[error("Image digest mismatch: expected {expected}, got {actual}")]
    ImageDigestMismatch { expected: String, actual: String },
}

/// Configuration for attestation verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationConfig {
    /// Expected audience (typically the sequencer URL)
    pub expected_audience: String,
    /// Whether to verify TEE-specific claims
    pub verify_tee_claims: bool,
    /// Optional: expected container image digest
    pub expected_image_digest: Option<String>,
}

/// JWT header structure
#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: String,
}

/// Claims from a Confidential Space attestation token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationClaims {
    /// Issuer (should be Google's Confidential Computing service)
    pub iss: String,
    /// Subject (service account identity)
    pub sub: String,
    /// Audience
    pub aud: String,
    /// Expiration time (Unix timestamp)
    pub exp: u64,
    /// Issued at (Unix timestamp)
    pub iat: u64,
    /// Not before (Unix timestamp)
    #[serde(default)]
    pub nbf: u64,
    /// TEE-specific claims (Confidential Space)
    #[serde(default)]
    pub secboot: bool,
    /// Debug status ("disabled" for production, "enabled" for debug VMs)
    #[serde(default)]
    pub dbgstat: Option<String>,
    #[serde(default)]
    pub swname: Option<String>,
    #[serde(default)]
    pub swversion: Option<Vec<String>>,
    #[serde(default)]
    pub hwmodel: Option<String>,
    /// Submodules containing container info
    #[serde(default)]
    pub submods: Option<Submods>,
}

/// Submodules structure containing container information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submods {
    pub container: Option<ContainerInfo>,
}

/// Container information from attestation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub image_reference: Option<String>,
    pub image_digest: Option<String>,
}

impl AttestationClaims {
    /// Get the container image digest if available
    pub fn image_digest(&self) -> Option<&str> {
        self.submods
            .as_ref()
            .and_then(|s| s.container.as_ref())
            .and_then(|c| c.image_digest.as_deref())
    }
}

/// JSON Web Key structure
#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kty: String,
    pub alg: Option<String>,
    pub kid: String,
    #[serde(rename = "use")]
    pub use_: Option<String>,
    pub n: String,
    pub e: String,
}

/// JSON Web Key Set
#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

/// OIDC Discovery Document
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

/// Cached JWKS with timestamp
#[derive(Debug)]
struct CachedJwks {
    jwks: Jwks,
    fetched_at: Instant,
}

/// Attestation verifier for TEE tokens
#[derive(Debug)]
pub struct AttestationVerifier {
    config: AttestationConfig,
    /// Cached JWKS (JSON Web Key Set)
    jwks_cache: Arc<RwLock<Option<CachedJwks>>>,
    /// Cached JWKS URI from OIDC discovery
    jwks_uri_cache: Arc<RwLock<Option<String>>>,
    http_client: reqwest::Client,
}

impl AttestationVerifier {
    /// Create a new attestation verifier
    pub fn new(config: AttestationConfig) -> Self {
        info!(
            audience = %config.expected_audience,
            verify_tee = config.verify_tee_claims,
            "Attestation verifier initialized"
        );

        Self {
            config,
            jwks_cache: Arc::new(RwLock::new(None)),
            jwks_uri_cache: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Verify an attestation token with full cryptographic signature verification
    pub async fn verify(&self, token: &str) -> Result<AttestationClaims, AttestationError> {
        debug!("Verifying attestation token");

        // Split the JWT into parts
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(AttestationError::InvalidFormat(
                "JWT must have 3 parts".to_string(),
            ));
        }

        let header_b64 = parts[0];
        let payload_b64 = parts[1];
        let signature_b64 = parts[2];

        // Decode and parse header
        let header_bytes = base64_decode_url_safe(header_b64)?;
        let header: JwtHeader = serde_json::from_slice(&header_bytes)
            .map_err(|e| AttestationError::InvalidFormat(format!("Failed to parse header: {e}")))?;

        // Verify algorithm is RS256
        if header.alg != "RS256" {
            return Err(AttestationError::InvalidFormat(format!(
                "Unsupported algorithm: {}. Expected RS256",
                header.alg
            )));
        }

        // Fetch JWKS and find the matching key
        let jwks = self.get_jwks().await?;
        let jwk = jwks
            .keys
            .iter()
            .find(|k| k.kid == header.kid)
            .ok_or_else(|| AttestationError::NoMatchingKey(header.kid.clone()))?;

        // Verify signature
        self.verify_signature(header_b64, payload_b64, signature_b64, jwk)?;

        // Decode payload
        let payload_bytes = base64_decode_url_safe(payload_b64)?;
        let claims: AttestationClaims = serde_json::from_slice(&payload_bytes)
            .map_err(|e| AttestationError::InvalidFormat(format!("Failed to parse claims: {e}")))?;

        // Validate issuer
        if claims.iss != "https://confidentialcomputing.googleapis.com" {
            return Err(AttestationError::InvalidIssuer {
                expected: "https://confidentialcomputing.googleapis.com".to_string(),
                actual: claims.iss,
            });
        }

        // Validate audience
        if claims.aud != self.config.expected_audience {
            return Err(AttestationError::InvalidAudience {
                expected: self.config.expected_audience.clone(),
                actual: claims.aud,
            });
        }

        // Check time validity
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if claims.exp < now {
            return Err(AttestationError::Expired);
        }

        if claims.nbf > 0 && claims.nbf > now {
            return Err(AttestationError::NotYetValid);
        }

        // Validate TEE claims if configured
        if self.config.verify_tee_claims {
            // Verify secure boot is enabled
            if !claims.secboot {
                return Err(AttestationError::SecureBootRequired);
            }

            // Verify debug mode is disabled
            if let Some(dbgstat) = &claims.dbgstat {
                if dbgstat != "disabled" {
                    warn!(dbgstat = %dbgstat, "Rejecting token with debug mode enabled");
                    return Err(AttestationError::DebugModeEnabled);
                }
            }

            // Verify image digest if configured
            if let Some(expected_digest) = &self.config.expected_image_digest {
                if let Some(actual_digest) = claims.image_digest() {
                    if actual_digest != expected_digest {
                        return Err(AttestationError::ImageDigestMismatch {
                            expected: expected_digest.clone(),
                            actual: actual_digest.to_string(),
                        });
                    }
                }
            }
        }

        debug!(
            iss = %claims.iss,
            sub = %claims.sub,
            secboot = claims.secboot,
            dbgstat = ?claims.dbgstat,
            "Attestation token verified with signature"
        );

        Ok(claims)
    }

    /// Verify RS256 signature
    fn verify_signature(
        &self,
        header_b64: &str,
        payload_b64: &str,
        signature_b64: &str,
        jwk: &Jwk,
    ) -> Result<(), AttestationError> {
        // Verify key type is RSA
        if jwk.kty != "RSA" {
            return Err(AttestationError::SignatureError(format!(
                "Unexpected key type: {}",
                jwk.kty
            )));
        }

        // Decode modulus and exponent
        let n_bytes = base64_decode_url_safe(&jwk.n)?;
        let e_bytes = base64_decode_url_safe(&jwk.e)?;

        // Build RSA public key
        let n = BigUint::from_bytes_be(&n_bytes);
        let e = BigUint::from_bytes_be(&e_bytes);

        let public_key = RsaPublicKey::new(n, e)
            .map_err(|e| AttestationError::SignatureError(format!("Invalid RSA key: {e}")))?;

        let verifying_key = VerifyingKey::<Sha256>::new(public_key);

        // Decode signature
        let signature_bytes = base64_decode_url_safe(signature_b64)?;
        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|e| AttestationError::SignatureError(format!("Invalid signature: {e}")))?;

        // The signing input is "header.payload" (without the signature)
        let signing_input = format!("{}.{}", header_b64, payload_b64);

        // Verify
        verifying_key
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|e| AttestationError::SignatureError(format!("Signature mismatch: {e}")))?;

        debug!("RS256 signature verified successfully");
        Ok(())
    }

    /// Get JWKS, using cache if available
    async fn get_jwks(&self) -> Result<Jwks, AttestationError> {
        // Check cache
        {
            let cache = self.jwks_cache.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.fetched_at.elapsed() < JWKS_CACHE_DURATION {
                    debug!("Using cached JWKS");
                    return Ok(cached.jwks.clone());
                }
            }
        }

        // Fetch fresh JWKS
        let jwks = self.fetch_jwks().await?;

        // Update cache
        {
            let mut cache = self.jwks_cache.write().await;
            *cache = Some(CachedJwks {
                jwks: jwks.clone(),
                fetched_at: Instant::now(),
            });
        }

        Ok(jwks)
    }

    /// Fetch JWKS from Google
    async fn fetch_jwks(&self) -> Result<Jwks, AttestationError> {
        // Get JWKS URI from discovery document
        let jwks_uri = self.get_jwks_uri().await?;

        debug!(uri = %jwks_uri, "Fetching JWKS");

        let response =
            self.http_client.get(&jwks_uri).send().await.map_err(|e| {
                AttestationError::JwksFetchError(format!("HTTP request failed: {e}"))
            })?;

        if !response.status().is_success() {
            return Err(AttestationError::JwksFetchError(format!(
                "JWKS endpoint returned {}",
                response.status()
            )));
        }

        let jwks: Jwks = response
            .json()
            .await
            .map_err(|e| AttestationError::JwksFetchError(format!("Failed to parse JWKS: {e}")))?;

        info!(key_count = jwks.keys.len(), "Fetched JWKS from Google");
        Ok(jwks)
    }

    /// Get JWKS URI from OIDC discovery document
    async fn get_jwks_uri(&self) -> Result<String, AttestationError> {
        // Check cache
        {
            let cache = self.jwks_uri_cache.read().await;
            if let Some(uri) = cache.as_ref() {
                return Ok(uri.clone());
            }
        }

        debug!("Fetching OIDC discovery document");

        let response = self
            .http_client
            .get(GOOGLE_OIDC_DISCOVERY_URL)
            .send()
            .await
            .map_err(|e| {
                AttestationError::JwksFetchError(format!("Discovery request failed: {e}"))
            })?;

        if !response.status().is_success() {
            return Err(AttestationError::JwksFetchError(format!(
                "Discovery endpoint returned {}",
                response.status()
            )));
        }

        let discovery: OidcDiscovery = response.json().await.map_err(|e| {
            AttestationError::JwksFetchError(format!("Failed to parse discovery document: {e}"))
        })?;

        // Cache the URI
        {
            let mut cache = self.jwks_uri_cache.write().await;
            *cache = Some(discovery.jwks_uri.clone());
        }

        Ok(discovery.jwks_uri)
    }

    /// Get the expected audience
    pub fn expected_audience(&self) -> &str {
        &self.config.expected_audience
    }
}

/// Decode base64url-encoded data (used in JWTs)
fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, AttestationError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| {
            // Try with standard base64 if URL-safe fails
            let standard = input.replace('-', "+").replace('_', "/");
            let padded = match standard.len() % 4 {
                2 => format!("{}==", standard),
                3 => format!("{}=", standard),
                _ => standard,
            };
            base64::engine::general_purpose::STANDARD.decode(&padded)
        })
        .map_err(|e| AttestationError::InvalidFormat(format!("Base64 decode failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AttestationConfig {
        AttestationConfig {
            expected_audience: "https://sequencer.example.com".to_string(),
            verify_tee_claims: false,
            expected_image_digest: None,
        }
    }

    fn test_config_with_tee_claims() -> AttestationConfig {
        AttestationConfig {
            expected_audience: "https://sequencer.example.com".to_string(),
            verify_tee_claims: true,
            expected_image_digest: None,
        }
    }

    #[test]
    fn test_base64_decode_url_safe() {
        // Test standard base64url decoding
        let input = "SGVsbG8gV29ybGQ"; // "Hello World" without padding
        let result = base64_decode_url_safe(input).unwrap();
        assert_eq!(result, b"Hello World");
    }

    #[tokio::test]
    async fn test_invalid_jwt_format() {
        let verifier = AttestationVerifier::new(test_config());

        // Missing parts
        let result = verifier.verify("invalid").await;
        assert!(matches!(result, Err(AttestationError::InvalidFormat(_))));

        // Only two parts
        let result = verifier.verify("part1.part2").await;
        assert!(matches!(result, Err(AttestationError::InvalidFormat(_))));
    }

    #[test]
    fn test_claims_parsing() {
        let claims_json = r#"{
            "iss": "https://confidentialcomputing.googleapis.com",
            "sub": "test-subject",
            "aud": "https://sequencer.example.com",
            "exp": 1700000000,
            "iat": 1699996400,
            "secboot": true,
            "dbgstat": "disabled",
            "hwmodel": "GCP_AMD_SEV",
            "submods": {
                "container": {
                    "image_digest": "sha256:abc123"
                }
            }
        }"#;

        let claims: AttestationClaims = serde_json::from_str(claims_json).unwrap();
        assert_eq!(claims.iss, "https://confidentialcomputing.googleapis.com");
        assert!(claims.secboot);
        assert_eq!(claims.dbgstat, Some("disabled".to_string()));
        assert_eq!(claims.image_digest(), Some("sha256:abc123"));
    }

    #[test]
    fn test_claims_without_submods() {
        let claims_json = r#"{
            "iss": "https://confidentialcomputing.googleapis.com",
            "sub": "test-subject",
            "aud": "https://sequencer.example.com",
            "exp": 1700000000,
            "iat": 1699996400
        }"#;

        let claims: AttestationClaims = serde_json::from_str(claims_json).unwrap();
        assert_eq!(claims.image_digest(), None);
    }

    #[tokio::test]
    #[ignore] // Only run when explicitly requested (requires network)
    async fn test_google_oidc_discovery_document() {
        // Fetch the OIDC discovery document from Google
        let client = reqwest::Client::new();
        let response = client
            .get(GOOGLE_OIDC_DISCOVERY_URL)
            .send()
            .await
            .expect("Failed to fetch OIDC discovery document");

        assert_eq!(
            response.status(),
            200,
            "OIDC discovery endpoint returned non-200 status"
        );

        let discovery: OidcDiscovery = response
            .json()
            .await
            .expect("Failed to parse OIDC discovery document");

        // Verify jwks_uri is present and looks correct
        assert!(
            discovery.jwks_uri.starts_with("https://"),
            "JWKS URI should use HTTPS"
        );
        assert!(
            discovery.jwks_uri.contains("googleapis.com"),
            "JWKS URI should be from googleapis.com"
        );
    }

    #[tokio::test]
    #[ignore] // Only run when explicitly requested (requires network)
    async fn test_fetch_jwks() {
        let verifier = AttestationVerifier::new(test_config());
        let jwks = verifier.fetch_jwks().await.expect("Failed to fetch JWKS");

        assert!(!jwks.keys.is_empty(), "JWKS should contain keys");

        // Verify all keys are RSA
        for key in &jwks.keys {
            assert_eq!(key.kty, "RSA", "Key should be RSA type");
            assert!(!key.kid.is_empty(), "Key should have a kid");
            assert!(!key.n.is_empty(), "Key should have modulus");
            assert!(!key.e.is_empty(), "Key should have exponent");
        }
    }
}
