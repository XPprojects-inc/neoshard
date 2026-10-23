//! API error type. Every error is JSON `{ "code": "...", "message": "..." }`
//! with a stable machine-readable `code` (spec/openapi.yaml#/components/schemas/Error).

use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use neoshard_proto::permissions::Permissions;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("missing permissions")]
    MissingPermissions(Permissions),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("rate limited")]
    RateLimited(Duration),
    #[error("{0}")]
    Unavailable(&'static str),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e {
            match db.code().as_deref() {
                Some("23505") => return ApiError::Conflict("already exists"),
                Some("23503") => {
                    return ApiError::BadRequest("referenced object does not exist".into());
                }
                Some("23514") => return ApiError::BadRequest("value violates a constraint".into()),
                _ => {}
            }
        }
        ApiError::Internal(e.into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::BadRequest(msg.into())
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            ApiError::MissingPermissions(_) => (StatusCode::FORBIDDEN, "missing_permissions"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            ApiError::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            ApiError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let mut body = json!({ "code": code, "message": self.to_string() });
        if let ApiError::MissingPermissions(p) = &self {
            body["missing_permissions"] = json!(p.bits().to_string());
        }
        let mut resp = (status, axum::Json(body)).into_response();
        if let ApiError::RateLimited(d) = &self {
            let secs = d.as_secs_f64().ceil().max(1.0) as u64;
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        resp
    }
}
