use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::routes::{ApiError, AppState};

/// Middleware verifying API token authentication on protected endpoints.
///
/// If `scenario.network.http.api_key` is `None` or empty, authentication is disabled
/// and all requests pass through. If configured, callers must provide a matching token via:
/// - `Authorization: Bearer <token>`
/// - `X-API-Key: <token>`
pub async fn require_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(expected_key) = &state.scenario.network.http.api_key {
        let expected = expected_key.trim();
        if !expected.is_empty() {
            let auth_header = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|val| val.to_str().ok());

            let custom_header = req
                .headers()
                .get("x-api-key")
                .and_then(|val| val.to_str().ok());

            let provided_token = auth_header
                .and_then(|auth| auth.strip_prefix("Bearer ").map(str::trim))
                .or_else(|| custom_header.map(str::trim));

            let authorized = match provided_token {
                Some(token) => bool::from(token.as_bytes().ct_eq(expected.as_bytes())),
                None => false,
            };

            if !authorized {
                return Err(ApiError::Unauthorized);
            }
        }
    }

    Ok(next.run(req).await)
}
