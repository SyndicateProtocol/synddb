//! RISC Zero GPU proof generation service
//!
//! This service generates ZK proofs for GCP Confidential Space attestation tokens
//! using RISC Zero's native GPU proving (CUDA). RISC Zero compiles to a native
//! binary that works directly on Cloud Run with L4 GPUs.
//!
//! # Endpoints
//!
//! - `POST /prove` - Generate a proof for an attestation token
//! - `GET /health` - Health check
//! - `GET /image-id` - Get the RISC Zero image ID for contract configuration

mod config;
mod prover;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use config::Config;
use prover::{decode_public_values, get_jwt_kid, AttestationProver, JwksCache};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

/// Application state shared across handlers
struct AppState {
    prover: AttestationProver,
    jwks_cache: JwksCache,
}

/// Request to generate a proof
#[derive(Debug, Deserialize)]
struct ProveRequest {
    /// Raw JWT attestation token from Confidential Space
    jwt_token: String,
    /// Expected audience claim
    expected_audience: String,
    /// EVM public key (64-byte uncompressed secp256k1, hex-encoded with 0x prefix)
    evm_public_key: String,
    /// Image signature (65 bytes: r || s || v, hex-encoded with 0x prefix)
    /// This is a secp256k1 ECDSA signature over keccak256(image_digest) for on-chain ecrecover
    image_signature: String,
}

/// Response from proof generation
#[derive(Debug, Serialize)]
struct ProveResponse {
    /// ABI-encoded PublicValuesStruct / journal (hex with 0x prefix)
    public_values: String,
    /// RISC Zero Groth16 proof bytes / seal (hex with 0x prefix)
    proof_bytes: String,
    /// Derived TEE address (for verification)
    tee_address: String,
}

/// Response for image ID endpoint
#[derive(Debug, Serialize)]
struct ImageIdResponse {
    /// RISC Zero image ID as bytes32 (hex with 0x prefix)
    image_id: String,
}

/// Error response
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    details: Option<String>,
    /// Whether this error is permanent (should not retry)
    #[serde(skip_serializing_if = "Option::is_none")]
    permanent: Option<bool>,
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

    // Handle --print-image-id flag (for CI/deployment)
    if config.print_image_id {
        let prover = AttestationProver::new();
        println!("0x{}", hex::encode(prover.image_id_bytes32()));
        return Ok(());
    }

    // Initialize logging
    if config.log_json {
        tracing_subscriber::fmt().json().init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            )
            .init();
    }

    info!("Initializing proof service");

    // Initialize prover (this loads the RISC Zero ELF)
    let prover = AttestationProver::new();

    // Initialize JWKS cache
    let jwks_cache = JwksCache::new(
        config.google_oidc_discovery_url.clone(),
        config.jwks_cache_ttl_secs,
    );

    let state = Arc::new(AppState { prover, jwks_cache });

    // Build router
    let app = Router::new()
        .route("/prove", post(prove_handler))
        .route("/health", get(health_handler))
        .route("/image-id", get(image_id_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    info!(address = %config.bind_address, "Starting proof service");

    axum::serve(listener, app).await?;

    Ok(())
}

/// Handle proof generation requests
async fn prove_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProveRequest>,
) -> impl IntoResponse {
    let result = generate_proof(&state, &request).await;

    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            let is_permanent = is_permanent_error(&error_msg);

            if is_permanent {
                error!(error = %e, "Permanent error encountered when requesting proof");
            } else {
                error!(error = %e, "Proof generation failed (transient, may retry)");
            }

            // Return 400 Bad Request for permanent errors (don't retry)
            // Return 503 Service Unavailable for transient errors (may retry)
            let status = if is_permanent {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };

            (
                status,
                Json(ErrorResponse {
                    error: "Proof generation failed".into(),
                    details: Some(error_msg),
                    permanent: Some(is_permanent),
                }),
            )
                .into_response()
        }
    }
}

/// Check if an error message indicates a permanent failure that should not be retried.
///
/// Permanent errors include:
/// - Invalid inputs that won't change on retry
/// - JWT/attestation errors
fn is_permanent_error(error_msg: &str) -> bool {
    let error_lower = error_msg.to_lowercase();

    // Input validation errors
    if error_lower.contains("invalid") && error_lower.contains("key") {
        return true;
    }
    if error_lower.contains("invalid") && error_lower.contains("signature") {
        return true;
    }

    // JWT/attestation errors that won't change
    if error_lower.contains("jwt") && error_lower.contains("expired") {
        return true;
    }
    if error_lower.contains("jwk not found") {
        return true;
    }

    // Attestation verification failures
    if error_lower.contains("invalid gcp") {
        return true;
    }

    false
}

/// Generate proof (separated for cleaner error handling)
async fn generate_proof(state: &AppState, request: &ProveRequest) -> anyhow::Result<ProveResponse> {
    info!("Processing proof request");

    // Parse EVM public key from hex
    let pubkey_hex = request.evm_public_key.trim_start_matches("0x");
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| anyhow::anyhow!("Invalid EVM public key hex: {}", e))?;
    let evm_public_key: [u8; 64] = pubkey_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("EVM public key must be exactly 64 bytes"))?;

    // Parse image signature from hex (must be exactly 65 bytes: r || s || v)
    let image_sig_hex = request.image_signature.trim_start_matches("0x");
    let image_signature = hex::decode(image_sig_hex)
        .map_err(|e| anyhow::anyhow!("Invalid image signature hex: {}", e))?;
    if image_signature.len() != 65 {
        anyhow::bail!(
            "Image signature must be exactly 65 bytes (r || s || v), got {}",
            image_signature.len()
        );
    }

    // Extract key ID from JWT
    let kid = get_jwt_kid(&request.jwt_token)?;
    info!(kid = %kid, "Extracted key ID from JWT");

    // Fetch JWK for this key ID
    let jwk = state.jwks_cache.get_jwk(&kid).await?;
    info!("Found matching JWK");

    // Log attestation sample for debugging/capture (query: "attestation_sample")
    // This data is not sensitive - tokens contain only TEE metadata, no secrets.
    info!(
        event = "attestation_sample",
        source = "proof_service",
        raw_token = %request.jwt_token,
        jwk_kid = %jwk.kid,
        jwk_n = %jwk.n,
        jwk_e = %jwk.e,
        audience = %request.expected_audience,
        "Attestation sample for proof generation"
    );

    // Generate proof (this is CPU/GPU intensive and takes minutes)
    let proof_output = state.prover.generate_proof(
        &request.jwt_token,
        &jwk,
        &request.expected_audience,
        &evm_public_key,
        &image_signature,
    )?;

    // Decode public values to get TEE address
    let public_values = decode_public_values(&proof_output.public_values)?;

    Ok(ProveResponse {
        public_values: format!("0x{}", hex::encode(&proof_output.public_values)),
        proof_bytes: format!("0x{}", hex::encode(&proof_output.proof_bytes)),
        tee_address: format!("{}", public_values.tee_signing_key),
    })
}

/// Handle image ID requests (for contract configuration)
async fn image_id_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let image_id_bytes = state.prover.image_id_bytes32();
    Json(ImageIdResponse {
        image_id: format!("0x{}", hex::encode(image_id_bytes)),
    })
}

/// Handle health check requests
async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ready".into(),
    })
}
