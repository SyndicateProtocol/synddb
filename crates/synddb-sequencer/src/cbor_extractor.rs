//! CBOR request body extractor for axum

use axum::{
    extract::{rejection::BytesRejection, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;

/// Rejection type for CBOR extraction failures
#[derive(Debug)]
pub enum CborRejection {
    /// Failed to read request body
    BytesRejection(BytesRejection),
    /// Invalid Content-Type header (not application/cbor)
    InvalidContentType,
    /// Failed to deserialize CBOR body
    DeserializationError(String),
}

impl IntoResponse for CborRejection {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BytesRejection(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            Self::InvalidContentType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Expected Content-Type: application/cbor".to_string(),
            ),
            Self::DeserializationError(e) => {
                (StatusCode::BAD_REQUEST, format!("Invalid CBOR: {e}"))
            }
        };
        (status, message).into_response()
    }
}

impl From<BytesRejection> for CborRejection {
    fn from(e: BytesRejection) -> Self {
        Self::BytesRejection(e)
    }
}

/// Extractor for CBOR request bodies
///
/// Requires `Content-Type: application/cbor` header and deserializes
/// the body using ciborium.
#[derive(Debug, Clone)]
pub struct Cbor<T>(pub T);

impl<T, S> FromRequest<S> for Cbor<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = CborRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Check Content-Type header
        let content_type = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok());

        match content_type {
            Some(ct) if ct.starts_with("application/cbor") => {}
            _ => return Err(CborRejection::InvalidContentType),
        }

        // Extract body bytes
        let bytes = axum::body::Bytes::from_request(req, state).await?;

        // Deserialize CBOR
        let value: T = ciborium::from_reader(bytes.as_ref())
            .map_err(|e| CborRejection::DeserializationError(e.to_string()))?;

        Ok(Self(value))
    }
}
