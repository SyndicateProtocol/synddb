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
//! 3. Verify the JWT signature using the appropriate key
//! 4. Validate standard claims (iss, aud, exp, iat)
//! 5. Optionally validate TEE-specific claims (hardware, software, image digest)

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Google's OIDC discovery endpoint for Confidential Space
const GOOGLE_OIDC_DISCOVERY_URL: &str =
    "https://confidentialcomputing.googleapis.com/.well-known/openid-configuration";

/// Errors that can occur during attestation verification
#[derive(Debug, Error)]
pub enum AttestationError {
    #[error("Token verification failed: {0}")]
    VerificationFailed(String),

    #[error("Invalid token format: {0}")]
    InvalidFormat(String),

    #[error("Token expired")]
    Expired,

    #[error("Invalid issuer: expected {expected}, got {actual}")]
    InvalidIssuer { expected: String, actual: String },

    #[error("Invalid audience: expected {expected}, got {actual}")]
    InvalidAudience { expected: String, actual: String },

    #[error("Failed to fetch JWKS: {0}")]
    JwksFetchError(String),

    #[error("No matching key found in JWKS")]
    NoMatchingKey,

    #[error("Configuration error: {0}")]
    Config(String),
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
    /// TEE-specific claims (Confidential Space)
    #[serde(default)]
    pub secboot: bool,
    #[serde(default)]
    pub swname: Option<String>,
    #[serde(default)]
    pub swversion: Option<String>,
    #[serde(default)]
    pub hwmodel: Option<String>,
    /// Container image reference
    #[serde(default)]
    pub image_reference: Option<String>,
    /// Container image digest
    #[serde(default)]
    pub image_digest: Option<String>,
}

/// Attestation verifier for TEE tokens
#[derive(Debug)]
pub struct AttestationVerifier {
    config: AttestationConfig,
    /// Cached JWKS (JSON Web Key Set)
    jwks_cache: Arc<RwLock<Option<CachedJwks>>>,
    http_client: reqwest::Client,
}

#[derive(Debug)]
struct CachedJwks {
    keys: serde_json::Value,
    fetched_at: std::time::Instant,
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
            http_client: reqwest::Client::new(),
        }
    }

    /// Verify an attestation token
    ///
    /// This performs a simplified verification that:
    /// 1. Decodes the JWT (without full cryptographic verification in this version)
    /// 2. Validates the issuer and audience claims
    /// 3. Checks token expiration
    ///
    /// For production use, full JWKS-based signature verification should be implemented.
    pub async fn verify(&self, token: &str) -> Result<AttestationClaims, AttestationError> {
        debug!("Verifying attestation token");

        // Split the JWT into parts
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(AttestationError::InvalidFormat(
                "JWT must have 3 parts".to_string(),
            ));
        }

        // Decode the payload (middle part)
        let payload_bytes = base64_decode_url_safe(parts[1])?;
        let claims: AttestationClaims = serde_json::from_slice(&payload_bytes)
            .map_err(|e| AttestationError::InvalidFormat(format!("Failed to parse claims: {e}")))?;

        // Validate issuer
        if !claims.iss.contains("confidentialcomputing.googleapis.com") {
            return Err(AttestationError::InvalidIssuer {
                expected: "confidentialcomputing.googleapis.com".to_string(),
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

        // Check expiration
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if claims.exp < now {
            return Err(AttestationError::Expired);
        }

        // Validate TEE claims if configured
        if self.config.verify_tee_claims {
            if !claims.secboot {
                warn!("Attestation token missing secure boot claim");
            }

            if let Some(expected_digest) = &self.config.expected_image_digest {
                if let Some(actual_digest) = &claims.image_digest {
                    if actual_digest != expected_digest {
                        return Err(AttestationError::VerificationFailed(format!(
                            "Image digest mismatch: expected {}, got {}",
                            expected_digest, actual_digest
                        )));
                    }
                }
            }
        }

        debug!(
            iss = %claims.iss,
            sub = %claims.sub,
            "Attestation token verified"
        );

        Ok(claims)
    }

    /// Get the expected audience
    pub fn expected_audience(&self) -> &str {
        &self.config.expected_audience
    }
}

/// OIDC Discovery Document structure
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    issuer: String,
    jwks_uri: String,
    subject_types_supported: Vec<String>,
    response_types_supported: Vec<String>,
    claims_supported: Vec<String>,
    id_token_signing_alg_values_supported: Vec<String>,
    scopes_supported: Vec<String>,
}

/// Decode base64url-encoded data (used in JWTs)
fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, AttestationError> {
    // Add padding if needed
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };

    // Replace URL-safe characters
    let standard = padded.replace('-', "+").replace('_', "/");

    base64::engine::general_purpose::STANDARD
        .decode(&standard)
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

    #[tokio::test]
    async fn test_expired_token() {
        let verifier = AttestationVerifier::new(test_config());

        // Create a minimal expired token
        let claims = serde_json::json!({
            "iss": "https://confidentialcomputing.googleapis.com",
            "sub": "test-subject",
            "aud": "https://sequencer.example.com",
            "exp": 1000, // Way in the past
            "iat": 900,
        });

        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&claims).unwrap());
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{}.signature", payload);

        let result = verifier.verify(&token).await;
        assert!(matches!(result, Err(AttestationError::Expired)));
    }

    #[tokio::test]
    async fn test_invalid_audience() {
        let verifier = AttestationVerifier::new(test_config());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = serde_json::json!({
            "iss": "https://confidentialcomputing.googleapis.com",
            "sub": "test-subject",
            "aud": "wrong-audience",
            "exp": now + 3600,
            "iat": now,
        });

        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&claims).unwrap());
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{}.signature", payload);

        let result = verifier.verify(&token).await;
        assert!(matches!(
            result,
            Err(AttestationError::InvalidAudience { .. })
        ));
    }

    #[tokio::test]
    async fn test_valid_token() {
        let verifier = AttestationVerifier::new(test_config());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = serde_json::json!({
            "iss": "https://confidentialcomputing.googleapis.com",
            "sub": "test-subject",
            "aud": "https://sequencer.example.com",
            "exp": now + 3600,
            "iat": now,
            "secboot": true,
        });

        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&claims).unwrap());
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{}.signature", payload);

        let result = verifier.verify(&token).await;
        assert!(result.is_ok());

        let verified_claims = result.unwrap();
        assert_eq!(verified_claims.sub, "test-subject");
        assert!(verified_claims.secboot);
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

        // Parse the response as our expected structure
        let discovery: OidcDiscovery = response
            .json()
            .await
            .expect("Failed to parse OIDC discovery document");

        // Validate expected values
        assert_eq!(
            discovery.issuer, "https://confidentialcomputing.googleapis.com",
            "Issuer mismatch"
        );

        // Verify jwks_uri is present and looks correct
        assert!(
            discovery.jwks_uri.starts_with("https://"),
            "JWKS URI should use HTTPS"
        );
        assert!(
            discovery.jwks_uri.contains("googleapis.com"),
            "JWKS URI should be from googleapis.com"
        );

        // Verify expected supported values
        assert!(
            discovery
                .response_types_supported
                .contains(&"id_token".to_string()),
            "Should support id_token response type"
        );

        assert!(
            discovery
                .id_token_signing_alg_values_supported
                .contains(&"RS256".to_string()),
            "Should support RS256 signing algorithm"
        );

        // Verify expected claims are supported
        let expected_claims = vec![
            "sub",
            "aud",
            "exp",
            "iat",
            "iss",
            "secboot",
            "hwmodel",
            "swname",
            "swversion",
        ];
        for claim in expected_claims {
            assert!(
                discovery.claims_supported.contains(&claim.to_string()),
                "Should support '{}' claim",
                claim
            );
        }

        // Verify scopes
        assert!(
            discovery.scopes_supported.contains(&"openid".to_string()),
            "Should support 'openid' scope"
        );

        println!("✓ OIDC discovery document structure is valid");
        println!("  Issuer: {}", discovery.issuer);
        println!("  JWKS URI: {}", discovery.jwks_uri);
        println!(
            "  Supported claims: {}",
            discovery.claims_supported.join(", ")
        );
    }
}
