//! GPU-accelerated SP1 proof generation service
//!
//! This service generates ZK proofs for GCP Confidential Space attestation tokens.
//! It runs outside the TEE and is called by TEE services during key bootstrapping.
//!
//! # Endpoints
//!
//! - `POST /prove` - Generate a proof for an attestation token
//! - `GET /health` - Health check

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
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

/// Application state shared across handlers
struct AppState {
    prover: AttestationProver,
    jwks_cache: JwksCache,
    /// Flag to track if prover is currently busy
    prover_busy: RwLock<bool>,
}

/// Request to generate a proof
#[derive(Debug, Deserialize)]
struct ProveRequest {
    /// Raw JWT attestation token from Confidential Space
    jwt_token: String,
    /// Expected audience claim
    expected_audience: String,
    /// TEE public key (64-byte uncompressed, hex-encoded with 0x prefix)
    tee_public_key: String,
}

/// Response from proof generation
#[derive(Debug, Serialize)]
struct ProveResponse {
    /// ABI-encoded PublicValuesStruct (hex with 0x prefix)
    public_values: String,
    /// SP1 proof bytes (hex with 0x prefix)
    proof_bytes: String,
    /// Derived TEE address (for verification)
    tee_address: String,
}

/// Error response
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    details: Option<String>,
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    prover_busy: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

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

    // Initialize prover (this loads the SP1 ELF)
    let prover = AttestationProver::new();

    // Initialize JWKS cache
    let jwks_cache = JwksCache::new(
        config.google_oidc_discovery_url.clone(),
        config.jwks_cache_ttl_secs,
    );

    let state = Arc::new(AppState {
        prover,
        jwks_cache,
        prover_busy: RwLock::new(false),
    });

    // Build router
    let app = Router::new()
        .route("/prove", post(prove_handler))
        .route("/health", get(health_handler))
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
    // Check if prover is busy
    {
        let busy = state.prover_busy.read().await;
        if *busy {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Prover is busy".into(),
                    details: Some("Another proof is currently being generated".into()),
                }),
            )
                .into_response();
        }
    }

    // Mark prover as busy
    {
        let mut busy = state.prover_busy.write().await;
        *busy = true;
    }

    // Ensure we mark prover as not busy when done
    let result = generate_proof(&state, &request).await;

    {
        let mut busy = state.prover_busy.write().await;
        *busy = false;
    }

    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(e) => {
            error!(error = %e, "Proof generation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Proof generation failed".into(),
                    details: Some(e.to_string()),
                }),
            )
                .into_response()
        }
    }
}

/// Generate proof (separated for cleaner error handling)
async fn generate_proof(state: &AppState, request: &ProveRequest) -> anyhow::Result<ProveResponse> {
    info!("Processing proof request");

    // Parse TEE public key from hex
    let pubkey_hex = request.tee_public_key.trim_start_matches("0x");
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| anyhow::anyhow!("Invalid TEE public key hex: {}", e))?;
    let tee_public_key: [u8; 64] = pubkey_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("TEE public key must be exactly 64 bytes"))?;

    // Extract key ID from JWT
    let kid = get_jwt_kid(&request.jwt_token)?;
    info!(kid = %kid, "Extracted key ID from JWT");

    // Fetch JWK for this key ID
    let jwk = state.jwks_cache.get_jwk(&kid).await?;
    info!("Found matching JWK");

    // Generate proof (this is CPU/GPU intensive and takes minutes)
    let proof = state.prover.generate_proof(
        &request.jwt_token,
        &jwk,
        &request.expected_audience,
        &tee_public_key,
    )?;

    // Decode public values to get TEE address
    let public_values = decode_public_values(proof.public_values.as_slice())?;

    // Serialize proof
    let proof_bytes = bincode::serialize(&proof.proof)
        .map_err(|e| anyhow::anyhow!("Failed to serialize proof: {}", e))?;

    Ok(ProveResponse {
        public_values: format!("0x{}", hex::encode(proof.public_values.as_slice())),
        proof_bytes: format!("0x{}", hex::encode(&proof_bytes)),
        tee_address: format!("{}", public_values.tee_signing_key),
    })
}

/// Handle health check requests
async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let busy = *state.prover_busy.read().await;
    Json(HealthResponse {
        status: "ready".into(),
        prover_busy: busy,
    })
}
