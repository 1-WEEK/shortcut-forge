use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::build::{probe_toolchain, run_build_pipeline};
use crate::error::ApiError;
use crate::model::{
    BuildMetadata, BuildRequest, BuildResponse, BuildStatus, Config, DownloadTokenRecord,
    ErrorBody, now_unix,
};
use crate::store::{
    artifact_path, generate_download_token, load_metadata, persist_artifact, prune_tokens,
    save_metadata, sha256_hex,
};

pub struct BuildLifecycle {
    config: Config,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

impl BuildLifecycle {
    pub fn new(config: Config) -> Self {
        Self {
            slots: Arc::new(tokio::sync::Semaphore::new(config.max_build_concurrency)),
            config,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn submit(&self, request: BuildRequest) -> Result<BuildResponse, ApiError> {
        let fingerprint_input = format!(
            "{}\n{}\n{}",
            request.source_format, request.sign_mode, request.source
        );
        let source_hash = sha256_hex(fingerprint_input.as_bytes());
        let id = source_hash[..crate::model::BUILD_ID_LEN].to_string();

        let per_id_lock = {
            let mut locks = self.locks.lock().await;
            locks
                .entry(id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = per_id_lock.lock().await;

        let now = now_unix();
        let expires_at = now.saturating_add(request.ttl_seconds as i64);
        let mut existing = load_metadata(&self.config.storage, &id)
            .map_err(|_| ApiError::internal_error("failed to read metadata"))?;
        if let Some(metadata) = existing.as_ref()
            && metadata.source_hash != source_hash
        {
            return Err(ApiError::internal_error(
                "truncated build id collision detected",
            ));
        }

        let toolchain = probe_toolchain(&self.config);
        if !toolchain.is_available() {
            return Err(ApiError::tool_unavailable());
        }

        let needs_rebuild = match existing.as_ref() {
            Some(metadata) if metadata.status == BuildStatus::Ready => {
                !artifact_path(&self.config.storage, &id).exists()
                    || metadata.toolchain.fingerprint != toolchain.fingerprint
            }
            _ => true,
        };

        if needs_rebuild {
            let _slot = self.try_acquire_slot()?;
            let created_at = existing
                .as_ref()
                .map(|metadata| metadata.created_at)
                .unwrap_or(now);
            match run_build_pipeline(&request, &id, &self.config).await {
                Ok(signed_path) => {
                    persist_artifact(&self.config.storage, &id, &signed_path)
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
                    save_metadata(&self.config.storage, &metadata)
                        .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
                    let download_url = format!("{}/s/{}", self.config.public_base_url, token);
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
                    let _ = save_metadata(&self.config.storage, &metadata);
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
        save_metadata(&self.config.storage, &metadata)
            .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
        Ok(BuildResponse {
            id,
            download_url: format!("{}/s/{}", self.config.public_base_url, token),
            expires_at,
        })
    }

    fn try_acquire_slot(&self) -> Result<BuildSlot, ApiError> {
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ApiError::server_busy())?;
        Ok(BuildSlot { _permit: permit })
    }
}

struct BuildSlot {
    _permit: tokio::sync::OwnedSemaphorePermit,
}
