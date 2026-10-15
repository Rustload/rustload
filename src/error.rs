//! Error type returned by HTTP handlers: `{ "error": "...", "code": "..." }`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", format!("{what} not found"))
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "authentication required")
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!("internal error: {error:#}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", "internal server error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message, "code": self.code }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(format!("{error:#}"))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => Self::not_found("resource"),
            other => Self::internal(other),
        }
    }
}

/// Converts `Option<T>` into a 404 with a descriptive message.
pub trait OrNotFound<T> {
    fn or_not_found(self, what: &str) -> ApiResult<T>;
}

impl<T> OrNotFound<T> for Option<T> {
    fn or_not_found(self, what: &str) -> ApiResult<T> {
        self.ok_or_else(|| ApiError::not_found(what))
    }
}
