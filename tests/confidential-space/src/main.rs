//! Confidential Space Attestation Sample Capture
//!
//! This workload runs inside GCP Confidential Space and captures attestation tokens
//! for use in RISC Zero on-chain verification development.
//!
//! Output: JSON containing raw JWT tokens, decoded claims, and Google's JWKS public keys.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, error, info, warn};

const ATTESTATION_SOCKET_PATH: &str = "/run/container_launcher/teeserver.sock";
const GOOGLE_OIDC_DISCOVERY_URL: &str =
    "https://confidentialcomputing.googleapis.com/.well-known/openid-configuration";

/// Token type for attestation requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum TokenType {
    Oidc,
}

/// Request to the attestation service
#[derive(Debug, Serialize)]
struct AttestationRequest {
    audience: String,
    token_type: TokenType,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nonces: Vec<String>,
}

/// Response from the attestation service
/// Note: The service may return either a JSON object with a "token" field,
/// or the raw JWT string directly.
#[derive(Debug, Deserialize)]
struct AttestationResponse {
    token: String,
}

impl AttestationResponse {
    /// Parse the response body, handling both JSON wrapper and raw JWT formats
    fn parse(body: &[u8]) -> Result<Self> {
        let body_str = String::from_utf8_lossy(body);

        // First, try to parse as JSON object with "token" field
        if let Ok(response) = serde_json::from_slice::<AttestationResponse>(body) {
            debug!("Parsed response as JSON object with token field");
            return Ok(response);
        }

        // If that fails, check if the body is a raw JWT (starts with eyJ which is base64 for {"alg)
        let trimmed = body_str.trim();
        if trimmed.starts_with("eyJ") && trimmed.contains('.') {
            debug!("Response appears to be a raw JWT, using directly");
            return Ok(AttestationResponse {
                token: trimmed.to_string(),
            });
        }

        // Neither format worked
        anyhow::bail!(
            "Response is neither JSON with 'token' field nor a raw JWT. Body preview: {}",
            if body_str.len() > 100 {
                &body_str[..100]
            } else {
                &body_str
            }
        )
    }
}

/// A captured attestation sample with all data needed for RISC Zero verification
#[derive(Debug, Serialize)]
struct AttestationSample {
    /// The raw JWT token (header.payload.signature)
    raw_token: String,
    /// Decoded JWT header
    header: serde_json::Value,
    /// Decoded JWT claims/payload
    claims: serde_json::Value,
    /// Raw signature bytes (base64url decoded)
    #[serde(with = "hex_bytes")]
    signature_bytes: Vec<u8>,
    /// The signing input: base64url(header) + "." + base64url(payload)
    signing_input: String,
    /// Unix timestamp when this sample was captured
    captured_at: u64,
    /// Audience used for this token
    audience: String,
    /// Nonces used (if any)
    nonces: Vec<String>,
}

/// Complete output bundle for RISC Zero development
#[derive(Debug, Serialize)]
struct AttestationBundle {
    /// Captured attestation samples
    samples: Vec<AttestationSample>,
    /// Google's JWKS (JSON Web Key Set) for signature verification
    jwks: serde_json::Value,
    /// OIDC discovery document
    oidc_discovery: serde_json::Value,
    /// Instructions for developers
    instructions: Instructions,
}

#[derive(Debug, Serialize)]
struct Instructions {
    summary: String,
    verification_steps: Vec<String>,
    important_claims: Vec<String>,
}

/// Hex serialization for signature bytes
mod hex_bytes {
    use serde::Serializer;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    info!("Confidential Space Attestation Sample Capture starting...");
    info!(version = env!("CARGO_PKG_VERSION"), "Binary version");

    // Log comprehensive system diagnostics
    log_system_diagnostics();

    if !is_confidential_space() {
        error!(
            socket_path = ATTESTATION_SOCKET_PATH,
            "Attestation socket not found. This binary must run inside GCP Confidential Space."
        );
        log_socket_troubleshooting();
        std::process::exit(1);
    }

    info!("Running in Confidential Space, attestation socket found");

    // Comprehensive socket diagnostics
    log_socket_diagnostics();

    let audience = std::env::var("ATTESTATION_AUDIENCE")
        .unwrap_or_else(|_| "https://synddb-sequencer.example.com".to_string());

    info!(audience = %audience, "Capturing attestation samples");

    let mut samples = Vec::new();

    // Capture standard token (no nonces)
    info!("Capturing standard attestation token...");
    match capture_attestation(&audience, vec![]).await {
        Ok(sample) => {
            info!("Standard token captured successfully");
            samples.push(sample);
        }
        Err(e) => {
            error!(error = %e, "Failed to capture standard token");
        }
    }

    // Capture token with nonce (for replay protection demos)
    let nonce = format!("risc0-test-nonce-{}", now_unix());
    info!(nonce = %nonce, "Capturing attestation token with nonce...");
    match capture_attestation(&audience, vec![nonce]).await {
        Ok(sample) => {
            info!("Token with nonce captured successfully");
            samples.push(sample);
        }
        Err(e) => {
            error!(error = %e, "Failed to capture token with nonce");
        }
    }

    if samples.is_empty() {
        error!("No attestation samples captured");
        std::process::exit(1);
    }

    // Fetch Google's OIDC discovery document and JWKS
    info!("Fetching Google OIDC discovery document...");
    let (oidc_discovery, jwks) = fetch_google_keys().await?;
    info!("JWKS fetched successfully");

    // Build the complete bundle
    let bundle = AttestationBundle {
        samples,
        jwks,
        oidc_discovery,
        instructions: Instructions {
            summary:
                "Use these samples to develop and test RISC Zero on-chain attestation verification"
                    .to_string(),
            verification_steps: vec![
                "1. Parse the JWT: split raw_token by '.' into [header, payload, signature]"
                    .to_string(),
                "2. Decode header and payload from base64url".to_string(),
                "3. Find the signing key in jwks.keys where kid matches header.kid".to_string(),
                "4. Verify RS256 signature: RSA_PKCS1_SHA256(signing_input, signature, public_key)"
                    .to_string(),
                "5. Validate claims: iss, aud, exp, iat".to_string(),
                "6. Check TEE claims: secboot, swname, image_digest".to_string(),
            ],
            important_claims: vec![
                "iss - must be https://confidentialcomputing.googleapis.com".to_string(),
                "aud - must match expected audience".to_string(),
                "exp - token expiration (Unix timestamp)".to_string(),
                "secboot - secure boot enabled (should be true)".to_string(),
                "swname - should be CONFIDENTIAL_SPACE".to_string(),
                "submods.container.image_digest - container image hash".to_string(),
            ],
        },
    };

    // Output the bundle
    info!(
        sample_count = bundle.samples.len(),
        "Serializing attestation bundle"
    );
    let output = serde_json::to_string_pretty(&bundle)
        .context("Failed to serialize attestation bundle to JSON")?;

    info!(output_len = output.len(), "Bundle serialized successfully");

    // Print to stdout (will be captured in Cloud Logging)
    println!("=== ATTESTATION BUNDLE START ===");
    println!("{}", output);
    println!("=== ATTESTATION BUNDLE END ===");

    // Also write to /tmp for potential retrieval
    let output_path = "/tmp/attestation_samples.json";
    info!(
        path = output_path,
        size = output.len(),
        "Writing samples to file"
    );

    match std::fs::write(output_path, &output) {
        Ok(()) => {
            info!(path = output_path, "Samples written to file successfully");

            // Verify the file was written correctly
            match std::fs::metadata(output_path) {
                Ok(meta) => {
                    info!(
                        path = output_path,
                        size = meta.len(),
                        permissions = format!("{:o}", meta.permissions().mode()),
                        "Output file metadata"
                    );
                }
                Err(e) => warn!(error = %e, "Could not read output file metadata"),
            }
        }
        Err(e) => {
            error!(
                path = output_path,
                error = %e,
                error_kind = ?e.kind(),
                "Failed to write samples to file"
            );
            // Don't fail completely, the stdout output is still available
            warn!("Continuing despite file write failure - output was printed to stdout");
        }
    }

    // Optionally upload to GCS if configured
    #[cfg(feature = "gcs")]
    if let Ok(bucket) = std::env::var("OUTPUT_BUCKET") {
        info!(bucket = %bucket, "Uploading to GCS...");
        if let Err(e) = upload_to_gcs(&bucket, &output).await {
            error!(
                error = %e,
                error_chain = ?e,
                "Failed to upload to GCS"
            );
        } else {
            info!("Uploaded to GCS successfully");
        }
    }

    info!(
        sample_count = bundle.samples.len(),
        "Attestation sample capture complete"
    );
    Ok(())
}

fn is_confidential_space() -> bool {
    std::path::Path::new(ATTESTATION_SOCKET_PATH).exists()
}

/// Log comprehensive system diagnostics for debugging
fn log_system_diagnostics() {
    info!("=== System Diagnostics ===");

    // Process info
    info!(pid = std::process::id(), "Process ID");
    info!(
        uid = unsafe { libc::getuid() },
        gid = unsafe { libc::getgid() },
        "User/Group IDs"
    );
    info!(
        euid = unsafe { libc::geteuid() },
        egid = unsafe { libc::getegid() },
        "Effective User/Group IDs"
    );

    // Current working directory
    match std::env::current_dir() {
        Ok(cwd) => info!(cwd = %cwd.display(), "Current working directory"),
        Err(e) => warn!(error = %e, "Failed to get current working directory"),
    }

    // Relevant environment variables
    let env_vars = [
        "ATTESTATION_AUDIENCE",
        "OUTPUT_BUCKET",
        "RUST_LOG",
        "RUST_LOG_FORMAT",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "HOME",
        "USER",
        "PATH",
    ];
    for var in env_vars {
        match std::env::var(var) {
            Ok(value) => {
                // Truncate long values like PATH
                let display_value = if value.len() > 100 {
                    format!("{}...(truncated)", &value[..100])
                } else {
                    value
                };
                info!(var = var, value = %display_value, "Environment variable");
            }
            Err(_) => debug!(var = var, "Environment variable not set"),
        }
    }

    // Check for GCP metadata availability
    if std::path::Path::new("/run/container_launcher").exists() {
        info!("Container launcher directory exists at /run/container_launcher");
        if let Ok(entries) = std::fs::read_dir("/run/container_launcher") {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => {
                        if ft.is_dir() {
                            "dir"
                        } else if ft.is_file() {
                            "file"
                        } else if ft.is_symlink() {
                            "symlink"
                        } else {
                            "other"
                        }
                    }
                    Err(_) => "unknown",
                };
                info!(
                    path = %path.display(),
                    file_type = file_type,
                    "Container launcher entry"
                );
            }
        }
    } else {
        warn!("Container launcher directory NOT found at /run/container_launcher");
    }

    // Check /run directory
    debug!("Checking /run directory contents...");
    if let Ok(entries) = std::fs::read_dir("/run") {
        let entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        debug!(count = entries.len(), "Entries in /run");
        for entry in entries.iter().take(20) {
            debug!(path = %entry.display(), "Entry in /run");
        }
    }

    info!("=== End System Diagnostics ===");
}

/// Log detailed socket diagnostics
fn log_socket_diagnostics() {
    info!("=== Socket Diagnostics ===");

    let socket_path = std::path::Path::new(ATTESTATION_SOCKET_PATH);

    // Check if path exists
    if !socket_path.exists() {
        error!(path = ATTESTATION_SOCKET_PATH, "Socket path does not exist");
        return;
    }

    // Get metadata
    match std::fs::metadata(socket_path) {
        Ok(meta) => {
            let file_type = meta.file_type();
            info!(
                path = ATTESTATION_SOCKET_PATH,
                is_file = meta.is_file(),
                is_dir = meta.is_dir(),
                is_symlink = file_type.is_symlink(),
                is_socket = std::os::unix::fs::FileTypeExt::is_socket(&file_type),
                permissions = format!("{:o}", meta.permissions().mode()),
                len = meta.len(),
                "Socket metadata"
            );

            // Check if it's actually a socket
            if !std::os::unix::fs::FileTypeExt::is_socket(&file_type) {
                error!(
                    path = ATTESTATION_SOCKET_PATH,
                    "Path exists but is NOT a Unix socket!"
                );
            }
        }
        Err(e) => {
            error!(
                path = ATTESTATION_SOCKET_PATH,
                error = %e,
                error_kind = ?e.kind(),
                "Failed to get socket metadata"
            );
        }
    }

    // Try to get symlink metadata if it's a symlink
    match std::fs::symlink_metadata(socket_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                match std::fs::read_link(socket_path) {
                    Ok(target) => info!(
                        path = ATTESTATION_SOCKET_PATH,
                        target = %target.display(),
                        "Socket is a symlink"
                    ),
                    Err(e) => warn!(error = %e, "Failed to read symlink target"),
                }
            }
        }
        Err(e) => debug!(error = %e, "Failed to get symlink metadata"),
    }

    // Check parent directory permissions
    if let Some(parent) = socket_path.parent() {
        match std::fs::metadata(parent) {
            Ok(meta) => {
                info!(
                    path = %parent.display(),
                    permissions = format!("{:o}", meta.permissions().mode()),
                    "Parent directory permissions"
                );
            }
            Err(e) => warn!(
                path = %parent.display(),
                error = %e,
                "Failed to get parent directory metadata"
            ),
        }
    }

    info!("=== End Socket Diagnostics ===");
}

/// Log troubleshooting steps when socket is not found
fn log_socket_troubleshooting() {
    error!("=== Troubleshooting Steps ===");
    error!("1. Ensure this workload is running inside a GCP Confidential Space VM");
    error!("2. The VM must be created with Confidential Computing enabled");
    error!("3. Check that the container is launched via the Confidential Space launcher");
    error!("4. Verify the image policy allows attestation token requests");
    error!("5. Check Cloud Logging for launcher errors: gcloud logging read 'resource.type=\"gce_instance\"'");
    error!("=== End Troubleshooting Steps ===");

    // List what's available in /run for debugging
    warn!("Listing /run directory to help diagnose...");
    match std::fs::read_dir("/run") {
        Ok(entries) => {
            for entry in entries.flatten() {
                warn!(path = %entry.path().display(), "Found in /run");
            }
        }
        Err(e) => error!(error = %e, "Cannot list /run directory"),
    }
}

async fn capture_attestation(audience: &str, nonces: Vec<String>) -> Result<AttestationSample> {
    debug!(
        audience = %audience,
        nonce_count = nonces.len(),
        "Starting attestation capture"
    );

    let raw_token = fetch_attestation_token(audience, nonces.clone()).await?;

    debug!(
        token_len = raw_token.len(),
        token_preview = %if raw_token.len() > 50 {
            format!("{}...", &raw_token[..50])
        } else {
            raw_token.clone()
        },
        "Received raw token"
    );

    // Split JWT into parts
    let parts: Vec<&str> = raw_token.split('.').collect();
    if parts.len() != 3 {
        error!(
            parts_count = parts.len(),
            token_preview = %if raw_token.len() > 100 {
                format!("{}...", &raw_token[..100])
            } else {
                raw_token.clone()
            },
            "Invalid JWT format"
        );
        anyhow::bail!(
            "Invalid JWT format: expected 3 parts (header.payload.signature), got {}. Token starts with: {}",
            parts.len(),
            if raw_token.len() > 50 { &raw_token[..50] } else { &raw_token }
        );
    }

    debug!(
        header_len = parts[0].len(),
        payload_len = parts[1].len(),
        signature_len = parts[2].len(),
        "JWT parts parsed"
    );

    let header: serde_json::Value = decode_base64url_json(parts[0])
        .with_context(|| format!("Failed to decode JWT header (length: {})", parts[0].len()))?;

    debug!(header = %header, "JWT header decoded");

    let claims: serde_json::Value = decode_base64url_json(parts[1])
        .with_context(|| format!("Failed to decode JWT payload (length: {})", parts[1].len()))?;

    // Log important claims for debugging
    if let Some(iss) = claims.get("iss") {
        debug!(iss = %iss, "Token issuer");
    }
    if let Some(aud) = claims.get("aud") {
        debug!(aud = %aud, "Token audience");
    }
    if let Some(exp) = claims.get("exp") {
        debug!(exp = %exp, "Token expiration");
    }
    if let Some(iat) = claims.get("iat") {
        debug!(iat = %iat, "Token issued at");
    }

    let signature_bytes = decode_base64url(parts[2]).with_context(|| {
        format!(
            "Failed to decode JWT signature (length: {})",
            parts[2].len()
        )
    })?;

    debug!(
        signature_len = signature_bytes.len(),
        "JWT signature decoded"
    );

    // The signing input is header.payload (without decoding)
    let signing_input = format!("{}.{}", parts[0], parts[1]);

    info!(
        audience = %audience,
        nonce_count = nonces.len(),
        claims_keys = ?claims.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        "Attestation captured successfully"
    );

    Ok(AttestationSample {
        raw_token,
        header,
        claims,
        signature_bytes,
        signing_input,
        captured_at: now_unix(),
        audience: audience.to_string(),
        nonces,
    })
}

async fn fetch_attestation_token(audience: &str, nonces: Vec<String>) -> Result<String> {
    use http_body_util::{BodyExt, Full};
    use hyper::{body::Bytes, Request};
    use hyper_util::{client::legacy::Client, rt::TokioExecutor};
    use hyperlocal::{UnixConnector, Uri};

    let request_body = AttestationRequest {
        audience: audience.to_string(),
        token_type: TokenType::Oidc,
        nonces: nonces.clone(),
    };

    let body_json =
        serde_json::to_string(&request_body).context("Failed to serialize attestation request")?;

    debug!(
        socket_path = ATTESTATION_SOCKET_PATH,
        endpoint = "/v1/token",
        request_body = %body_json,
        "Preparing attestation request"
    );

    let url = Uri::new(ATTESTATION_SOCKET_PATH, "/v1/token");
    let client: Client<UnixConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build(UnixConnector);

    debug!("Creating HTTP request to Unix socket");

    let req = Request::builder()
        .method("POST")
        .uri(url)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body_json.clone())))
        .context("Failed to build HTTP request")?;

    debug!("Sending request to attestation service...");

    let response = client.request(req).await.map_err(|e| {
        // Detailed error logging for connection failures
        error!(
            socket_path = ATTESTATION_SOCKET_PATH,
            error = %e,
            error_debug = ?e,
            "Failed to connect to attestation service"
        );

        // Check for common error types
        let error_hint = if e.to_string().contains("connection refused") {
            "Connection refused - the attestation service may not be running"
        } else if e.to_string().contains("permission denied") {
            "Permission denied - check socket permissions and user/group"
        } else if e.to_string().contains("no such file") {
            "Socket file not found - container launcher may not have started"
        } else {
            "Unknown connection error"
        };

        error!(hint = error_hint, "Connection error hint");

        anyhow::anyhow!(
            "Failed to send request to attestation service at {}: {} ({})",
            ATTESTATION_SOCKET_PATH,
            e,
            error_hint
        )
    })?;

    let status = response.status();
    debug!(status = %status, "Received response from attestation service");

    if !status.is_success() {
        // Try to read error body for more details
        let error_body = response
            .into_body()
            .collect()
            .await
            .map(|b| String::from_utf8_lossy(&b.to_bytes()).to_string())
            .unwrap_or_else(|_| "<failed to read error body>".to_string());

        error!(
            status = %status,
            status_code = status.as_u16(),
            error_body = %error_body,
            audience = %audience,
            nonce_count = nonces.len(),
            "Attestation service returned error"
        );

        anyhow::bail!(
            "Attestation service returned HTTP {}: {}",
            status,
            error_body
        );
    }

    let body_bytes = response
        .into_body()
        .collect()
        .await
        .context("Failed to read response body from attestation service")?
        .to_bytes();

    debug!(response_len = body_bytes.len(), "Read response body");

    // Log raw response for debugging (but truncate if very long)
    let body_str = String::from_utf8_lossy(&body_bytes);
    if body_str.len() > 500 {
        debug!(
            response_preview = %format!("{}...", &body_str[..500]),
            full_len = body_str.len(),
            "Attestation response (truncated)"
        );
    } else {
        debug!(response = %body_str, "Attestation response");
    }

    let attestation_response = AttestationResponse::parse(&body_bytes).map_err(|e| {
        error!(
            error = %e,
            body_preview = %if body_str.len() > 200 {
                format!("{}...", &body_str[..200])
            } else {
                body_str.to_string()
            },
            "Failed to parse attestation response"
        );
        e
    })?;

    info!(
        token_len = attestation_response.token.len(),
        "Successfully received attestation token"
    );

    Ok(attestation_response.token)
}

async fn fetch_google_keys() -> Result<(serde_json::Value, serde_json::Value)> {
    info!("Fetching Google OIDC configuration and JWKS...");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    // Fetch OIDC discovery document
    debug!(
        url = GOOGLE_OIDC_DISCOVERY_URL,
        "Fetching OIDC discovery document"
    );

    let discovery_response = client
        .get(GOOGLE_OIDC_DISCOVERY_URL)
        .send()
        .await
        .map_err(|e| {
            error!(
                url = GOOGLE_OIDC_DISCOVERY_URL,
                error = %e,
                is_connect = e.is_connect(),
                is_timeout = e.is_timeout(),
                is_request = e.is_request(),
                "Failed to fetch OIDC discovery document"
            );
            e
        })
        .context("Failed to send request for OIDC discovery document")?;

    let discovery_status = discovery_response.status();
    debug!(status = %discovery_status, "OIDC discovery response status");

    if !discovery_status.is_success() {
        let error_body = discovery_response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        error!(
            status = %discovery_status,
            body = %error_body,
            "OIDC discovery request failed"
        );
        anyhow::bail!(
            "OIDC discovery request failed with status {}: {}",
            discovery_status,
            error_body
        );
    }

    let discovery: serde_json::Value = discovery_response
        .json()
        .await
        .context("Failed to parse OIDC discovery document as JSON")?;

    debug!(
        issuer = %discovery.get("issuer").unwrap_or(&serde_json::Value::Null),
        "OIDC discovery document fetched"
    );

    let jwks_uri = discovery["jwks_uri"]
        .as_str()
        .context("Missing jwks_uri in discovery document")?;

    info!(jwks_uri = %jwks_uri, "Fetching JWKS from discovery URI");

    // Fetch JWKS
    let jwks_response = client
        .get(jwks_uri)
        .send()
        .await
        .map_err(|e| {
            error!(
                url = %jwks_uri,
                error = %e,
                is_connect = e.is_connect(),
                is_timeout = e.is_timeout(),
                "Failed to fetch JWKS"
            );
            e
        })
        .context("Failed to send request for JWKS")?;

    let jwks_status = jwks_response.status();
    debug!(status = %jwks_status, "JWKS response status");

    if !jwks_status.is_success() {
        let error_body = jwks_response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        error!(
            status = %jwks_status,
            body = %error_body,
            "JWKS request failed"
        );
        anyhow::bail!(
            "JWKS request failed with status {}: {}",
            jwks_status,
            error_body
        );
    }

    let jwks: serde_json::Value = jwks_response
        .json()
        .await
        .context("Failed to parse JWKS as JSON")?;

    // Log key info for debugging
    if let Some(keys) = jwks.get("keys").and_then(|k| k.as_array()) {
        info!(
            key_count = keys.len(),
            key_ids = ?keys.iter()
                .filter_map(|k| k.get("kid").and_then(|v| v.as_str()))
                .collect::<Vec<_>>(),
            "JWKS fetched successfully"
        );
    } else {
        warn!("JWKS does not contain expected 'keys' array");
    }

    Ok((discovery, jwks))
}

fn decode_base64url(input: &str) -> Result<Vec<u8>> {
    use base64::Engine;

    // Add padding if needed
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };

    // Convert from URL-safe to standard base64
    let standard = padded.replace('-', "+").replace('_', "/");

    base64::engine::general_purpose::STANDARD
        .decode(&standard)
        .map_err(|e| {
            // Log some context about the input that failed
            let preview = if input.len() > 50 {
                format!("{}...(len={})", &input[..50], input.len())
            } else {
                input.to_string()
            };
            debug!(
                error = %e,
                input_preview = %preview,
                input_len = input.len(),
                "Base64 decode failed"
            );
            anyhow::anyhow!(
                "Base64 decode failed for input of length {}: {}",
                input.len(),
                e
            )
        })
}

fn decode_base64url_json(input: &str) -> Result<serde_json::Value> {
    let bytes = decode_base64url(input)?;

    serde_json::from_slice(&bytes).map_err(|e| {
        // Try to show what we got
        let text_preview = String::from_utf8_lossy(&bytes);
        let preview = if text_preview.len() > 100 {
            format!("{}...", &text_preview[..100])
        } else {
            text_preview.to_string()
        };
        debug!(
            error = %e,
            error_line = e.line(),
            error_column = e.column(),
            decoded_preview = %preview,
            decoded_len = bytes.len(),
            "JSON parse of base64url content failed"
        );
        anyhow::anyhow!(
            "JSON parse failed at line {} column {}: {}. Content preview: {}",
            e.line(),
            e.column(),
            e,
            preview
        )
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Format current UTC time as YYYYMMDD_HH_MM_SS for readable filenames
#[cfg(feature = "gcs")]
fn format_timestamp_utc() -> String {
    use chrono::Utc;
    Utc::now().format("%Y%m%d_%H_%M_%S").to_string()
}

fn init_logging() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    // Use JSON format by default in containers (for Cloud Logging)
    // Set RUST_LOG_FORMAT=text to use human-readable format
    let use_json = std::env::var("RUST_LOG_FORMAT")
        .map(|v| v.to_lowercase() != "text")
        .unwrap_or(true);

    // Default to DEBUG level for comprehensive diagnostics
    // Can be overridden with RUST_LOG env var (e.g., RUST_LOG=info)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // If RUST_LOG is not set, default to debug for this crate, info for dependencies
        EnvFilter::new("cs_attestation_sample=debug,info")
    });

    if use_json {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer())
            .init();
    }
}

#[cfg(feature = "gcs")]
async fn upload_to_gcs(bucket: &str, content: &str) -> Result<()> {
    use bytes::Bytes;
    use google_cloud_storage::client::Storage;

    let storage = Storage::builder()
        .build()
        .await
        .context("Failed to create GCS Storage client")?;

    let timestamp = format_timestamp_utc();
    let path = format!("attestation-samples/samples_{}.json", timestamp);

    storage
        .write_object(bucket, &path, Bytes::from(content.to_string()))
        .send_buffered()
        .await
        .context("Failed to upload to GCS")?;

    info!(path = %path, "Uploaded attestation samples to gs://{}/{}", bucket, path);
    Ok(())
}
