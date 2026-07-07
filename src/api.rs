use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::model::{
    BuildRequest, DEFAULT_TTL_SECONDS, MAX_TTL_SECONDS, MIN_TTL_SECONDS, VERSION, format_rfc3339,
    json_escape, now_unix,
};
use crate::state::{AppState, get_cached_toolchain};
use crate::store::{
    constant_time_eq, is_valid_build_id, is_valid_download_token, load_metadata, resolve_download,
    safe_filename, sha256_hex,
};

pub async fn health_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let auth_header = headers.get("authorization");
    let is_authed = auth_header
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|token| constant_time_eq(token.as_bytes(), state.config.auth_token.as_bytes()))
        .unwrap_or(false);

    match auth_header {
        Some(_) if !is_authed => ApiError::unauthorized().into_response(),
        Some(_) => {
            let toolchain = get_cached_toolchain(&state).await;
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                format!(
                    r#"{{"ok":true,"data":{{"version":"{}","status":"ok","auth_required":true,"cherri":"{}","shortcuts_sign":"{}","cache_ttl_seconds":{}}}}}"#,
                    json_escape(VERSION),
                    json_escape(&toolchain.cherri),
                    json_escape(&toolchain.shortcuts_sign),
                    state.config.health_cache_ttl.as_secs()
                ),
            )
                .into_response()
        }
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            format!(
                r#"{{"ok":true,"data":{{"version":"{}","status":"ok","auth_required":true}}}}"#,
                json_escape(VERSION)
            ),
        )
            .into_response(),
    }
}

pub async fn build_handler(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request = match parse_build_request(&body, state.config.max_source_bytes) {
        Ok(request) => request,
        Err(err) => return err.into_response(),
    };
    match state.builds.submit(request).await {
        Ok(response) => {
            let body = format!(
                r#"{{"id":"{}","download_url":"{}","expires_at":"{}"}}"#,
                json_escape(&response.id),
                json_escape(&response.download_url),
                json_escape(&format_rfc3339(response.expires_at))
            );
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(err) => err.into_response(),
    }
}

pub async fn metadata_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if !is_valid_build_id(&id) {
        return ApiError::not_found().into_response();
    }
    match load_metadata(&state.config.storage, &id) {
        Ok(Some(metadata)) => {
            let body = metadata.to_api_json(now_unix());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Ok(None) => ApiError::not_found().into_response(),
        Err(_) => ApiError::internal_error("failed to read metadata").into_response(),
    }
}

pub async fn download_handler(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    if !is_valid_download_token(&token) {
        return ApiError::not_found().into_response();
    }
    let token_hash = sha256_hex(token.as_bytes());
    match resolve_download(&state.config.storage, &token_hash, now_unix()) {
        Ok(Some(download)) => match std::fs::read(&download.artifact_path) {
            Ok(bytes) => {
                let filename = format!("{}.shortcut", safe_filename(&download.name));
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/octet-stream"),
                );
                headers.insert(
                    header::CONTENT_DISPOSITION,
                    axum::http::HeaderValue::from_str(&format!(
                        r#"attachment; filename="{filename}""#
                    ))
                    .unwrap(),
                );
                (StatusCode::OK, headers, bytes).into_response()
            }
            Err(_) => ApiError::not_found().into_response(),
        },
        Ok(None) => ApiError::not_found().into_response(),
        Err(_) => ApiError::internal_error("failed to read metadata").into_response(),
    }
}

#[derive(serde::Deserialize, Debug)]
struct RawBuildRequest {
    name: String,
    source_format: String,
    source: String,
    #[serde(default)]
    sign_mode: Option<String>,
    #[serde(default)]
    ttl_seconds: Option<i64>,
}

pub(crate) fn parse_build_request(
    body: &[u8],
    max_source_bytes: usize,
) -> Result<BuildRequest, ApiError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| ApiError::validation_failed(format!("invalid JSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::validation_failed("request body must be a JSON object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "name" | "source_format" | "source" | "sign_mode" | "ttl_seconds"
        ) {
            return Err(ApiError::validation_failed("unknown request field"));
        }
    }
    let raw: RawBuildRequest = serde_json::from_slice(body)
        .map_err(|e| ApiError::validation_failed(format!("invalid request: {e}")))?;
    let name = raw.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::validation_failed("name must be 1-80 characters"));
    }
    if raw.source_format != "cherri" {
        return Err(ApiError::validation_failed("source_format must be cherri"));
    }
    if raw.source.is_empty() {
        return Err(ApiError::validation_failed("source must be non-empty"));
    }
    if raw.source.len() > max_source_bytes {
        return Err(ApiError::payload_too_large(
            "source exceeds configured limit",
        ));
    }
    let sign_mode = raw.sign_mode.unwrap_or_else(|| "anyone".to_string());
    if sign_mode != "anyone" {
        return Err(ApiError::validation_failed("sign_mode must be anyone"));
    }
    let ttl_seconds = raw
        .ttl_seconds
        .map(|v| if v < 0 { 0 } else { v as u64 })
        .unwrap_or(DEFAULT_TTL_SECONDS);
    if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl_seconds) {
        return Err(ApiError::validation_failed(
            "ttl_seconds must be between 60 and 2592000",
        ));
    }
    Ok(BuildRequest {
        name,
        source_format: raw.source_format,
        source: raw.source,
        sign_mode,
        ttl_seconds,
    })
}
