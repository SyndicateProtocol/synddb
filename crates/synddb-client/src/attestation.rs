//! GCP Confidential Space TEE attestation support
//!
//! This module provides functionality to obtain attestation tokens from
//! GCP Confidential Space's attestation service via the local Unix domain socket.
//!
//! Attestation tokens are JWT tokens that prove the workload is running in a
//! trusted execution environment (TEE) and include claims about the hardware,
//! software, and container image.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Path to the GCP Confidential Space attestation socket
const ATTESTATION_SOCKET_PATH: &str = "/run/container_launcher/teeserver.sock";

/// Default token cache duration (50 minutes, as tokens expire after 1 hour)
const DEFAULT_CACHE_DURATION: Duration = Duration::from_secs(50 * 60);

/// Token type for attestation requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TokenType {
    /// `OpenID` Connect token (default)
    #[default]
    Oidc,
    /// PKI token with certificate chain
    Pki,
    /// AWS principal tags token
    AwsPrincipaltags,
}

/// Request to the attestation service
#[derive(Debug, Serialize)]
struct AttestationRequest {
    /// Audience for the token (max 512 bytes)
    audience: String,
    /// Type of token to request
    token_type: TokenType,
    /// Optional nonces (up to 6, each 10-74 bytes)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nonces: Vec<String>,
}

/// Response from the attestation service
#[derive(Debug, Deserialize)]
struct AttestationResponse {
    /// The attestation token (JWT)
    token: String,
}

/// Shared state for token caching across clones
#[derive(Debug)]
struct TokenCache {
    token: Option<String>,
    expires_at: Option<Instant>,
}

/// Client for obtaining attestation tokens from GCP Confidential Space
///
/// This client can be safely cloned and shared across threads. All clones share
/// the same token cache via Arc<Mutex<_>>, so tokens are only fetched when needed.
#[derive(Clone, Debug)]
pub struct AttestationClient {
    /// Audience for attestation tokens
    audience: String,
    /// Token type to request
    token_type: TokenType,
    /// Shared token cache (synchronized across clones)
    cache: Arc<Mutex<TokenCache>>,
    /// Token cache duration
    cache_duration: Duration,
}

impl AttestationClient {
    /// Create a new attestation client
    ///
    /// # Arguments
    ///
    /// * `audience` - The audience for attestation tokens (typically the sequencer URL)
    /// * `token_type` - The type of token to request (OIDC, PKI, or `AWS_PRINCIPALTAGS`)
    ///
    /// # Returns
    ///
    /// Returns `Ok(AttestationClient)` if the client can be created successfully.
    /// Returns `Err` if running outside GCP Confidential Space (socket not available).
    pub fn new(audience: impl Into<String>, token_type: TokenType) -> Result<Self> {
        // Check if the socket exists (only available in Confidential Space)
        if !std::path::Path::new(ATTESTATION_SOCKET_PATH).exists() {
            return Err(anyhow::anyhow!(
                "Confidential Space attestation socket not found at {}. \
                 Are you running in GCP Confidential Space?",
                ATTESTATION_SOCKET_PATH
            ));
        }

        let audience_string = audience.into();

        info!(
            "Initialized Confidential Space attestation client (audience: {}, type: {:?})",
            audience_string, token_type
        );

        Ok(Self {
            audience: audience_string,
            token_type,
            cache: Arc::new(Mutex::new(TokenCache {
                token: None,
                expires_at: None,
            })),
            cache_duration: DEFAULT_CACHE_DURATION,
        })
    }

    /// Get an attestation token, using cached token if still valid
    ///
    /// Tokens are cached for 50 minutes (they expire after 1 hour).
    /// A fresh token is fetched if the cache is empty or expired.
    ///
    /// This method is thread-safe and can be called concurrently from multiple threads.
    pub async fn get_token(&self) -> Result<String> {
        // Check if we have a valid cached token
        {
            let cache = self.cache.lock().unwrap();
            if let (Some(token), Some(expires_at)) = (&cache.token, cache.expires_at) {
                if Instant::now() < expires_at {
                    debug!("Using cached attestation token");
                    return Ok(token.clone());
                }
            }
        }

        debug!("Fetching new attestation token");

        // Fetch new token
        let token = self.fetch_token_internal(vec![]).await?;

        // Cache the token
        {
            let mut cache = self.cache.lock().unwrap();
            cache.token = Some(token.clone());
            cache.expires_at = Some(Instant::now() + self.cache_duration);
        }

        Ok(token)
    }

    /// Get an attestation token with custom nonces
    ///
    /// Nonces are used to prevent replay attacks. Each nonce must be 10-74 bytes.
    /// Up to 6 nonces can be provided.
    ///
    /// Tokens with nonces are not cached.
    pub async fn get_token_with_nonces(&self, nonces: Vec<String>) -> Result<String> {
        // Validate nonces
        if nonces.len() > 6 {
            return Err(anyhow::anyhow!("Too many nonces: {} (max 6)", nonces.len()));
        }

        for (i, nonce) in nonces.iter().enumerate() {
            let len = nonce.len();
            if !(10..=74).contains(&len) {
                return Err(anyhow::anyhow!(
                    "Nonce {} has invalid length: {} (must be 10-74 bytes)",
                    i,
                    len
                ));
            }
        }

        self.fetch_token_internal(nonces).await
    }

    /// Internal method to fetch token from attestation service
    async fn fetch_token_internal(&self, nonces: Vec<String>) -> Result<String> {
        debug!(
            "Fetching attestation token from Confidential Space (audience: {}, type: {:?}, nonces: {})",
            self.audience,
            self.token_type,
            nonces.len()
        );

        let request = AttestationRequest {
            audience: self.audience.clone(),
            token_type: self.token_type,
            nonces,
        };

        // Make request to attestation service via Unix domain socket
        #[cfg(unix)]
        let attestation_response = {
            use hyper::{Body, Client, Request};
            use hyperlocal::{UnixClientExt, UnixConnector, Uri};

            let url = Uri::new(ATTESTATION_SOCKET_PATH, "/v1/token");

            let client: Client<UnixConnector> = Client::unix();

            let body_json = serde_json::to_string(&request)
                .context("Failed to serialize attestation request")?;

            let req = Request::builder()
                .method("POST")
                .uri(url)
                .header("Content-Type", "application/json")
                .body(Body::from(body_json))
                .context("Failed to build attestation request")?;

            let response = client
                .request(req)
                .await
                .context("Failed to send attestation request")?;

            if !response.status().is_success() {
                return Err(anyhow::anyhow!(
                    "Attestation service returned error: {}",
                    response.status()
                ));
            }

            let body_bytes = hyper::body::to_bytes(response.into_body())
                .await
                .context("Failed to read attestation response body")?;

            serde_json::from_slice::<AttestationResponse>(&body_bytes)
                .context("Failed to parse attestation response")?
        };

        #[cfg(not(unix))]
        let attestation_response: AttestationResponse =
            { unreachable!("Unix domain socket support checked in new()") };

        info!("Successfully obtained attestation token");
        debug!("Token: {}", attestation_response.token);

        Ok(attestation_response.token)
    }

    /// Invalidate the cached token, forcing a fresh token on next request
    pub fn invalidate_cache(&self) {
        debug!("Invalidating cached attestation token");
        let mut cache = self.cache.lock().unwrap();
        cache.token = None;
        cache.expires_at = None;
    }

    /// Get the audience this client is configured for
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Get the token type this client is configured for
    pub const fn token_type(&self) -> TokenType {
        self.token_type
    }
}

/// Check if running in GCP Confidential Space
///
/// This is a lightweight check that only verifies the attestation socket exists.
pub fn is_confidential_space() -> bool {
    std::path::Path::new(ATTESTATION_SOCKET_PATH).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_type_serialization() {
        assert_eq!(serde_json::to_string(&TokenType::Oidc).unwrap(), "\"OIDC\"");
        assert_eq!(serde_json::to_string(&TokenType::Pki).unwrap(), "\"PKI\"");
        assert_eq!(
            serde_json::to_string(&TokenType::AwsPrincipaltags).unwrap(),
            "\"AWS_PRINCIPALTAGS\""
        );
    }

    #[test]
    fn test_nonce_validation() {
        // Test would require mock attestation service
        // In real Confidential Space, this would work:
        // let client = AttestationClient::new("test-audience", TokenType::Oidc).unwrap();
        // let result = client.get_token_with_nonces(vec!["too_short".to_string()]);
        // assert!(result.is_err());
    }

    #[test]
    fn test_is_confidential_space() {
        // This will return false in normal test environment
        // In Confidential Space, it would return true
        let is_cs = is_confidential_space();
        println!("Running in Confidential Space: {}", is_cs);
    }
}
