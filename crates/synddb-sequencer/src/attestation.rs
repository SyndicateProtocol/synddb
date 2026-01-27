//! TEE attestation verification for incoming requests
//!
//! Verifies that clients are running in a trusted execution environment (TEE)
//! by validating their attestation tokens. Currently supports GCP Confidential Space
//! OIDC tokens.
//!
//! This module wraps the `gcp-attestation` crate and adds:
//! - JWKS fetching and caching from Google's endpoints
//! - Configuration-based verification options
//! - Sequencer-specific error handling

use gcp_attestation::{extract_kid_from_jwt, verify_attestation, JwkKey, ValidationResult};
use serde::{Deserialize, Serialize};
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

impl From<gcp_attestation::VerificationError> for AttestationError {
    fn from(err: gcp_attestation::VerificationError) -> Self {
        use gcp_attestation::VerificationError;
        match err {
            VerificationError::JwtParseError(msg) => Self::InvalidFormat(msg.into()),
            VerificationError::UnsupportedAlgorithm(alg) => {
                Self::InvalidFormat(format!("Unsupported algorithm: {}", alg))
            }
            VerificationError::KeyNotFound(kid) => Self::NoMatchingKey(kid),
            VerificationError::InvalidKeyType(kty) => {
                Self::SignatureError(format!("Invalid key type: {}", kty))
            }
            VerificationError::KeyDecodeError(msg) => {
                Self::SignatureError(format!("Key decode error: {}", msg))
            }
            VerificationError::RsaKeyError(msg) => {
                Self::SignatureError(format!("RSA key error: {}", msg))
            }
            VerificationError::SignatureVerificationFailed(msg) => Self::SignatureError(msg),
            VerificationError::InvalidIssuer(iss) => Self::InvalidIssuer {
                expected: "https://confidentialcomputing.googleapis.com".into(),
                actual: iss,
            },
            VerificationError::InvalidSwname(swname) => {
                Self::VerificationFailed(format!("Invalid software name: {}", swname))
            }
            VerificationError::InvalidAudience { expected, actual } => {
                Self::InvalidAudience { expected, actual }
            }
            VerificationError::TokenExpired { .. } => Self::Expired,
            VerificationError::TokenNotYetValid { .. } => Self::NotYetValid,
        }
    }
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

/// Claims from a Confidential Space attestation token
///
/// This is a simplified view of the claims for the sequencer's use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationClaims {
    /// Issuer
    pub iss: String,
    /// Subject
    pub sub: String,
    /// Audience
    pub aud: String,
    /// Expiration time
    pub exp: u64,
    /// Issued at
    pub iat: u64,
    /// Secure boot enabled
    pub secboot: bool,
    /// Debug status
    pub dbgstat: Option<String>,
    /// Hardware model
    pub hwmodel: Option<String>,
    /// Container image digest
    pub image_digest: Option<String>,
}

impl From<&ValidationResult> for AttestationClaims {
    fn from(result: &ValidationResult) -> Self {
        Self {
            iss: "https://confidentialcomputing.googleapis.com".into(),
            sub: String::new(), // Not exposed by ValidationResult
            aud: result.audience.clone(),
            exp: result.validity_window_end,
            iat: result.validity_window_start,
            secboot: result.secboot,
            dbgstat: if result.dbgstat.is_empty() {
                None
            } else {
                Some(result.dbgstat.clone())
            },
            hwmodel: if result.hwmodel.is_empty() {
                None
            } else {
                Some(result.hwmodel.clone())
            },
            image_digest: if result.image_digest.is_empty() {
                None
            } else {
                Some(result.image_digest.clone())
            },
        }
    }
}

/// JSON Web Key Set from Google
#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<JwkKey>,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created.
    pub fn new(config: AttestationConfig) -> Result<Self, AttestationError> {
        info!(
            audience = %config.expected_audience,
            verify_tee = config.verify_tee_claims,
            "Attestation verifier initialized"
        );

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AttestationError::Config(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            config,
            jwks_cache: Arc::new(RwLock::new(None)),
            jwks_uri_cache: Arc::new(RwLock::new(None)),
            http_client,
        })
    }

    /// Verify an attestation token with full cryptographic signature verification
    pub async fn verify(&self, token: &str) -> Result<AttestationClaims, AttestationError> {
        debug!("Verifying attestation token");

        // Extract the key ID to find the right JWK
        let kid = extract_kid_from_jwt(token.as_bytes())
            .map_err(|e| AttestationError::InvalidFormat(e.into()))?;

        // Fetch JWKS and find the matching key
        let jwks = self.get_jwks().await?;
        let jwk = jwks
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| AttestationError::NoMatchingKey(kid.clone()))?;

        // Get current time for validation
        // SystemTime::now() can only fail if system clock is before UNIX epoch (1970),
        // which would indicate a misconfigured system. Default to 0 which will cause
        // token validation to fail with a clear error rather than panicking.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Verify the token using the shared library
        let result = verify_attestation(
            token.as_bytes(),
            jwk,
            Some(&self.config.expected_audience),
            Some(now),
        )?;

        // Additional TEE claim validation if configured
        if self.config.verify_tee_claims {
            // Verify secure boot is enabled
            if !result.secboot {
                return Err(AttestationError::SecureBootRequired);
            }

            // Verify debug mode is disabled
            if !result.dbgstat.is_empty() && result.dbgstat != "disabled" {
                warn!(dbgstat = %result.dbgstat, "Rejecting token with debug mode enabled");
                return Err(AttestationError::DebugModeEnabled);
            }

            // Verify image digest if configured
            if let Some(expected_digest) = &self.config.expected_image_digest {
                if !result.image_digest.is_empty() && result.image_digest != *expected_digest {
                    return Err(AttestationError::ImageDigestMismatch {
                        expected: expected_digest.clone(),
                        actual: result.image_digest,
                    });
                }
            }
        }

        let claims = AttestationClaims::from(&result);

        debug!(
            aud = %claims.aud,
            secboot = claims.secboot,
            dbgstat = ?claims.dbgstat,
            "Attestation token verified with signature"
        );

        Ok(claims)
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

    #[tokio::test]
    async fn test_invalid_jwt_format() {
        let verifier = AttestationVerifier::new(test_config()).expect("Failed to create verifier");

        // Missing parts
        let result = verifier.verify("invalid").await;
        assert!(matches!(result, Err(AttestationError::InvalidFormat(_))));

        // Only two parts
        let result = verifier.verify("part1.part2").await;
        assert!(matches!(result, Err(AttestationError::InvalidFormat(_))));
    }

    #[tokio::test]
    #[ignore] // Only run when explicitly requested (requires network)
    async fn test_google_oidc_discovery_document() {
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
        let verifier = AttestationVerifier::new(test_config()).expect("Failed to create verifier");
        let jwks = verifier.fetch_jwks().await.expect("Failed to fetch JWKS");

        assert!(!jwks.keys.is_empty(), "JWKS should contain keys");

        for key in &jwks.keys {
            assert_eq!(key.kty, "RSA", "Key should be RSA type");
            assert!(!key.kid.is_empty(), "Key should have a kid");
            assert!(!key.n.is_empty(), "Key should have modulus");
            assert!(!key.e.is_empty(), "Key should have exponent");
        }
    }
}
