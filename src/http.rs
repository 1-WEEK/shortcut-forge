use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::{DefaultBodyLimit, Request};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::{
    Router,
    routing::{get, post},
};
use tokio::net::TcpListener;

use crate::api;
use crate::error::ApiError;
use crate::model::Config;
use crate::state::{AppState, StorageLock};
use crate::store::constant_time_eq;

pub async fn serve(config: Config) -> std::io::Result<()> {
    fs::create_dir_all(&config.storage)?;
    let storage_lock = StorageLock::acquire(&config.storage)?;
    let state = Arc::new(AppState {
        config: config.clone(),
        build_locks: tokio::sync::Mutex::new(HashMap::new()),
        build_slots: Arc::new(tokio::sync::Semaphore::new(config.max_build_concurrency)),
        health_cache: tokio::sync::Mutex::new(None),
        _storage_lock: storage_lock,
    });

    let bind_addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&bind_addr).await?;
    eprintln!(
        "listening on http://{} with public_base_url={} storage={}",
        bind_addr,
        config.public_base_url,
        config.storage.display()
    );

    let body_limit = config
        .max_source_bytes
        .saturating_mul(4)
        .saturating_add(64 * 1024);

    let app = Router::new()
        .route("/health", get(api::health_handler))
        .route("/s/{token}", get(api::download_handler))
        .nest(
            "/api",
            Router::new()
                .route("/builds", post(api::build_handler))
                .route("/builds/{id}", get(api::metadata_handler))
                .route_layer(middleware::from_fn_with_state(
                    Arc::clone(&state),
                    auth_middleware,
                )),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());
    let is_authorized = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|token| constant_time_eq(token.as_bytes(), state.config.auth_token.as_bytes()))
        .unwrap_or(false);

    if !is_authorized {
        return ApiError::unauthorized().into_response();
    }

    next.run(request).await
}
