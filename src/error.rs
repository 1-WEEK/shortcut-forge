use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum BuildError {
    #[error("{0}")]
    Api(#[from] ApiError),
    #[error("toolchain unavailable")]
    ToolUnavailable,
    #[error("compile timed out")]
    CompileTimeout,
    #[error("sign timed out")]
    SignTimeout,
    #[error("cherri compile failed")]
    CompileFailed,
    #[error("shortcuts sign failed")]
    SignFailed,
    #[error("cherri did not produce shortcut output")]
    MissingUnsignedOutput,
    #[error("shortcuts sign did not produce output")]
    MissingSignedOutput,
    #[error("failed to create build directory")]
    BuildDirFailed(#[source] io::Error),
    #[error("failed to write source")]
    WriteSourceFailed(#[source] io::Error),
    #[error("failed to persist artifact")]
    PersistArtifactFailed(#[source] io::Error),
    #[error("failed to generate token")]
    TokenGenFailed(#[source] io::Error),
    #[error("failed to persist metadata")]
    PersistMetadataFailed(#[source] io::Error),
    #[error("truncated build id collision detected")]
    IdCollision,
    #[error("failed to read metadata")]
    ReadMetadataFailed(#[source] io::Error),
    #[error("server busy")]
    ServerBusy,
}

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum StoreError {
    #[error("io error")]
    Io(#[from] io::Error),
    #[error("invalid metadata")]
    InvalidMetadata(String),
    #[error("failed to read metadata")]
    ReadMetadataFailed(#[source] io::Error),
}

#[derive(Error, Debug, Clone)]
#[error("{message}")]
pub struct ApiError {
    pub code: &'static str,
    pub status: u16,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &'static str, status: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }

    pub fn into_response(self) -> Response {
        let body = format!(
            r#"{{"ok":false,"error":{{"code":"{}","message":"{}"}}}}"#,
            self.code,
            crate::model::json_escape(&self.message)
        );
        (
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::OK),
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response()
    }

    pub fn unauthorized() -> Self {
        Self::new("UNAUTHORIZED", 401, "missing or invalid bearer token")
    }

    pub fn not_found() -> Self {
        Self::new("NOT_FOUND", 404, "not found")
    }

    pub fn validation_failed(message: impl Into<String>) -> Self {
        Self::new("VALIDATION_FAILED", 400, message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new("PAYLOAD_TOO_LARGE", 413, message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", 500, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("TIMEOUT", 504, message)
    }

    pub fn build_failed(message: impl Into<String>) -> Self {
        Self::new("BUILD_FAILED", 422, message)
    }

    pub fn sign_failed(message: impl Into<String>) -> Self {
        Self::new("SIGN_FAILED", 422, message)
    }

    pub fn tool_unavailable() -> Self {
        Self::new(
            "TOOL_UNAVAILABLE",
            503,
            "required external tool is unavailable",
        )
    }

    pub fn server_busy() -> Self {
        Self::new("SERVER_BUSY", 503, "build concurrency limit reached")
    }
}
