use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

pub async fn api_key_auth(
    expected_key: Option<String>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(expected) = expected_key else {
        return Ok(next.run(request).await);
    };

    let auth_header = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(key) if key == expected => Ok(next.run(request).await),
        Some(_) => Err(StatusCode::UNAUTHORIZED),
        None => Err(StatusCode::UNAUTHORIZED),
    }
}
