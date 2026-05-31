use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(unix)]
use std::os::fd::AsRawFd;

use tokio::sync::Mutex;

use crate::build::{probe_toolchain, run_build_pipeline};
use crate::error::ApiError;
use crate::model::{
    BuildMetadata, BuildRequest, BuildResponse, BuildStatus, CachedToolchain, Config,
    DownloadTokenRecord, ErrorBody, Toolchain, now_unix,
};
use crate::store::{
    artifact_path, generate_download_token, load_metadata, persist_artifact, prune_tokens,
    save_metadata, sha256_hex,
};

pub struct AppState {
    pub config: Config,
    pub build_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub build_slots: Arc<tokio::sync::Semaphore>,
    pub health_cache: Mutex<Option<CachedToolchain>>,
    pub _storage_lock: StorageLock,
}

pub struct StorageLock {
    #[allow(dead_code)]
    file: std::fs::File,
}

impl StorageLock {
    pub fn acquire(storage: &Path) -> io::Result<Self> {
        fs::create_dir_all(storage)?;
        let lock_path = storage.join(".lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        #[cfg(unix)]
        {
            const LOCK_EX: i32 = 2;
            const LOCK_NB: i32 = 4;
            let result = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
            if result != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("storage lock is already held: {}", lock_path.display()),
                ));
            }
        }
        writeln!(&file, "pid={}", std::process::id())?;
        file.sync_all()?;
        Ok(Self { file })
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

pub struct BuildSlot {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl BuildSlot {
    pub fn try_acquire(state: &AppState) -> Result<Self, ApiError> {
        let permit = Arc::clone(&state.build_slots)
            .try_acquire_owned()
            .map_err(|_| ApiError::server_busy())?;
        Ok(Self { _permit: permit })
    }
}

pub async fn get_cached_toolchain(state: &AppState) -> Toolchain {
    let mut cache = state.health_cache.lock().await;
    if let Some(cached) = cache.as_ref()
        && cached.probed_at.elapsed() < state.config.health_cache_ttl
    {
        return cached.toolchain.clone();
    }
    let toolchain = probe_toolchain(&state.config);
    *cache = Some(CachedToolchain {
        probed_at: Instant::now(),
        toolchain: toolchain.clone(),
    });
    toolchain
}

pub async fn build_or_renew(
    request: BuildRequest,
    state: &AppState,
) -> Result<BuildResponse, ApiError> {
    let fingerprint_input = format!(
        "{}\n{}\n{}",
        request.source_format, request.sign_mode, request.source
    );
    let source_hash = sha256_hex(fingerprint_input.as_bytes());
    let id = source_hash[..crate::model::BUILD_ID_LEN].to_string();

    let per_id_lock = {
        let mut locks = state.build_locks.lock().await;
        locks
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = per_id_lock.lock().await;

    let now = now_unix();
    let expires_at = now.saturating_add(request.ttl_seconds as i64);
    let mut existing = load_metadata(&state.config.storage, &id)
        .map_err(|_| ApiError::internal_error("failed to read metadata"))?;
    if let Some(metadata) = existing.as_ref()
        && metadata.source_hash != source_hash
    {
        return Err(ApiError::internal_error(
            "truncated build id collision detected",
        ));
    }

    let toolchain = probe_toolchain(&state.config);
    if !toolchain.is_available() {
        return Err(ApiError::tool_unavailable());
    }

    let needs_rebuild = match existing.as_ref() {
        Some(metadata) if metadata.status == BuildStatus::Ready => {
            !artifact_path(&state.config.storage, &id).exists()
                || metadata.toolchain.fingerprint != toolchain.fingerprint
        }
        _ => true,
    };

    if needs_rebuild {
        let _slot = BuildSlot::try_acquire(state)?;
        let created_at = existing
            .as_ref()
            .map(|metadata| metadata.created_at)
            .unwrap_or(now);
        match run_build_pipeline(&request, &id, &state.config).await {
            Ok(signed_path) => {
                persist_artifact(&state.config.storage, &id, &signed_path)
                    .map_err(|_| ApiError::internal_error("failed to persist artifact"))?;
                let _ = fs::remove_file(&signed_path);
                let token = generate_download_token()
                    .map_err(|_| ApiError::internal_error("failed to generate token"))?;
                let mut tokens = existing
                    .take()
                    .map(|metadata| metadata.download_tokens)
                    .unwrap_or_default();
                prune_tokens(&mut tokens, now);
                tokens.push(DownloadTokenRecord {
                    hash: sha256_hex(token.as_bytes()),
                    expires_at,
                });
                let metadata = BuildMetadata {
                    id: id.clone(),
                    name: request.name,
                    source_format: request.source_format,
                    source_hash,
                    sign_mode: request.sign_mode,
                    status: BuildStatus::Ready,
                    download_tokens: tokens,
                    toolchain,
                    created_at,
                    updated_at: now_unix(),
                    expires_at,
                    error: None,
                };
                save_metadata(&state.config.storage, &metadata)
                    .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
                let download_url = format!("{}/s/{}", state.config.public_base_url, token);
                return Ok(BuildResponse {
                    id,
                    download_url,
                    expires_at,
                });
            }
            Err(err) => {
                let metadata = BuildMetadata {
                    id: id.clone(),
                    name: request.name,
                    source_format: request.source_format,
                    source_hash,
                    sign_mode: request.sign_mode,
                    status: BuildStatus::Failed,
                    download_tokens: Vec::new(),
                    toolchain,
                    created_at,
                    updated_at: now_unix(),
                    expires_at,
                    error: Some(ErrorBody {
                        code: err.code.to_string(),
                        message: err.message.clone(),
                    }),
                };
                let _ = save_metadata(&state.config.storage, &metadata);
                return Err(err);
            }
        }
    }

    let token = generate_download_token()
        .map_err(|_| ApiError::internal_error("failed to generate token"))?;
    let mut metadata = existing.expect("existing metadata checked above");
    prune_tokens(&mut metadata.download_tokens, now);
    metadata.name = request.name;
    metadata.expires_at = expires_at;
    metadata.updated_at = now_unix();
    metadata.download_tokens.push(DownloadTokenRecord {
        hash: sha256_hex(token.as_bytes()),
        expires_at,
    });
    metadata.status = BuildStatus::Ready;
    metadata.error = None;
    save_metadata(&state.config.storage, &metadata)
        .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
    Ok(BuildResponse {
        id,
        download_url: format!("{}/s/{}", state.config.public_base_url, token),
        expires_at,
    })
}
