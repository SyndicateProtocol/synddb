//! API key authentication middleware

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

use super::handlers::AppState;

const API_KEY_HEADER: &str = "X-API-Key";

/// Middleware that validates API key for protected endpoints.
///
/// If no API key is configured on the server, all requests are allowed.
/// If an API key is configured, requests must include a matching X-API-Key header.
pub async fn require_api_key(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // If no API key configured, allow all requests
    let Some(expected_key) = &state.api_key else {
        return Ok(next.run(request).await);
    };

    // Check for API key header
    let provided_key = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok());

    match provided_key {
        Some(key) if key == expected_key => Ok(next.run(request).await),
        Some(_) => {
            tracing::warn!("Invalid API key provided");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            tracing::warn!("Missing API key header");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, middleware, routing::get, Router};
    use tower::ServiceExt;

    use crate::{
        bridge::BridgeClient,
        config::ValidatorMode,
        signing::MessageSigner,
        state::{MessageStore, NonceStore},
        storage::providers::MemoryPublisher,
        validation::ValidationPipeline,
    };
    use std::time::Duration;

    async fn test_handler() -> &'static str {
        "ok"
    }

    fn create_test_state(api_key: Option<String>) -> Arc<AppState> {
        let message_store = Arc::new(MessageStore::new(":memory:").unwrap());
        let nonce_store = Arc::new(NonceStore::new(":memory:").unwrap());
        let pipeline = Arc::new(ValidationPipeline::new(
            message_store,
            nonce_store,
            Duration::from_secs(60),
            Duration::from_secs(3600),
        ));
        let signer = Arc::new(
            MessageSigner::new(
                "0x0000000000000000000000000000000000000000000000000000000000000001",
                1,
                alloy::primitives::Address::ZERO,
            )
            .unwrap(),
        );
        let bridge_client = Arc::new(
            BridgeClient::new(
                "http://localhost:8545",
                alloy::primitives::Address::ZERO,
                "0x0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap(),
        );
        let storage = Arc::new(MemoryPublisher::new());

        Arc::new(AppState {
            mode: ValidatorMode::Primary,
            pipeline,
            signer,
            bridge_client,
            storage,
            api_key,
        })
    }

    #[tokio::test]
    async fn test_no_api_key_configured_allows_all() {
        let state = create_test_state(None);
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_api_key,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_valid_api_key_allowed() {
        let state = create_test_state(Some("secret-key".to_string()));
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_api_key,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .header("X-API-Key", "secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_api_key_rejected() {
        let state = create_test_state(Some("secret-key".to_string()));
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_api_key,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .header("X-API-Key", "wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_missing_api_key_rejected() {
        let state = create_test_state(Some("secret-key".to_string()));
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_api_key,
            ))
            .with_state(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
