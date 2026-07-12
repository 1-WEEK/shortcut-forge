use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::build::{probe_toolchain, run_build_pipeline};
use crate::error::ApiError;
use crate::model::{
    BuildMetadata, BuildRequest, BuildResponse, BuildStatus, Config, DownloadTokenRecord,
    ErrorBody, Toolchain, now_unix,
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
        let identity = BuildIdentity::from_request(&request);
        let per_id_lock = self.per_id_lock(&identity.id).await;
        let _guard = per_id_lock.lock().await;
        let decision = self.decide(request, identity)?;

        match decision {
            BuildLifecycleDecision::Renewal(context) => {
                self.complete_ready_transition(context, None)
            }
            BuildLifecycleDecision::Rebuild(context) => {
                let _slot = self.try_acquire_slot()?;
                let outcome = self.run_rebuild(&context).await;
                self.complete_rebuild_transition(context, outcome)
            }
        }
    }

    async fn per_id_lock(&self, id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn decide(
        &self,
        request: BuildRequest,
        identity: BuildIdentity,
    ) -> Result<BuildLifecycleDecision, ApiError> {
        let now = now_unix();
        let expires_at = now.saturating_add(request.ttl_seconds as i64);
        let existing = load_metadata(&self.config.storage, &identity.id)
            .map_err(|_| ApiError::internal_error("failed to read metadata"))?;
        if let Some(metadata) = existing.as_ref()
            && metadata.source_hash != identity.source_hash
        {
            return Err(ApiError::internal_error(
                "truncated build id collision detected",
            ));
        }
        let toolchain = probe_toolchain(&self.config);
        if !toolchain.is_available() {
            return Err(ApiError::tool_unavailable());
        }
        let context = BuildLifecycleDecisionContext {
            request,
            identity,
            existing,
            toolchain,
            now,
            expires_at,
            created_at: now,
        }
        .with_created_at();
        if context.requires_rebuild(&self.config.storage) {
            Ok(BuildLifecycleDecision::Rebuild(context))
        } else {
            Ok(BuildLifecycleDecision::Renewal(context))
        }
    }

    async fn run_rebuild(&self, context: &BuildLifecycleDecisionContext) -> BuildPipelineOutcome {
        match run_build_pipeline(&context.request, &context.identity.id, &self.config).await {
            Ok(signed_path) => BuildPipelineOutcome::Succeeded { signed_path },
            Err(err) => BuildPipelineOutcome::Failed(err),
        }
    }

    fn complete_rebuild_transition(
        &self,
        context: BuildLifecycleDecisionContext,
        outcome: BuildPipelineOutcome,
    ) -> Result<BuildResponse, ApiError> {
        match outcome {
            BuildPipelineOutcome::Succeeded { signed_path } => {
                self.complete_ready_transition(context, Some(signed_path))
            }
            BuildPipelineOutcome::Failed(err) => {
                let _ = fs::remove_file(artifact_path(&self.config.storage, &context.identity.id));
                let metadata = self.build_metadata(
                    &context,
                    BuildStatus::Failed,
                    Vec::new(),
                    Some(ErrorBody {
                        code: err.code.to_string(),
                        message: err.message.clone(),
                    }),
                );
                save_metadata(&self.config.storage, &metadata)
                    .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
                Err(err)
            }
        }
    }

    fn complete_ready_transition(
        &self,
        context: BuildLifecycleDecisionContext,
        signed_path: Option<PathBuf>,
    ) -> Result<BuildResponse, ApiError> {
        if let Some(signed_path) = signed_path {
            self.persist_signed_artifact(&context.identity.id, &signed_path)?;
        }
        let token = generate_download_token()
            .map_err(|_| ApiError::internal_error("failed to generate token"))?;
        let mut download_tokens = context.pruned_download_tokens();
        download_tokens.push(DownloadTokenRecord {
            hash: sha256_hex(token.as_bytes()),
            expires_at: context.expires_at,
        });
        let metadata = self.build_metadata(&context, BuildStatus::Ready, download_tokens, None);
        save_metadata(&self.config.storage, &metadata)
            .map_err(|_| ApiError::internal_error("failed to persist metadata"))?;
        Ok(BuildResponse {
            id: context.identity.id,
            download_url: format!("{}/s/{}", self.config.public_base_url, token),
            expires_at: context.expires_at,
        })
    }

    fn persist_signed_artifact(&self, id: &str, signed_path: &Path) -> Result<(), ApiError> {
        persist_artifact(&self.config.storage, id, signed_path)
            .map_err(|_| ApiError::internal_error("failed to persist artifact"))?;
        let _ = fs::remove_file(signed_path);
        Ok(())
    }

    fn build_metadata(
        &self,
        context: &BuildLifecycleDecisionContext,
        status: BuildStatus,
        download_tokens: Vec<DownloadTokenRecord>,
        error: Option<ErrorBody>,
    ) -> BuildMetadata {
        BuildMetadata {
            id: context.identity.id.clone(),
            name: context.request.name.clone(),
            source_format: context.request.source_format.clone(),
            source_hash: context.identity.source_hash.clone(),
            sign_mode: context.request.sign_mode.clone(),
            status,
            download_tokens,
            toolchain: context.toolchain.clone(),
            created_at: context.created_at,
            updated_at: now_unix(),
            expires_at: context.expires_at,
            error,
        }
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

#[derive(Clone)]
struct BuildIdentity {
    id: String,
    source_hash: String,
}

impl BuildIdentity {
    fn from_request(request: &BuildRequest) -> Self {
        let fingerprint_input = format!(
            "{}\n{}\n{}",
            request.source_format, request.sign_mode, request.source
        );
        let source_hash = sha256_hex(fingerprint_input.as_bytes());
        let id = source_hash[..crate::model::BUILD_ID_LEN].to_string();
        Self { id, source_hash }
    }
}

enum BuildLifecycleDecision {
    Renewal(BuildLifecycleDecisionContext),
    Rebuild(BuildLifecycleDecisionContext),
}

struct BuildLifecycleDecisionContext {
    request: BuildRequest,
    identity: BuildIdentity,
    existing: Option<BuildMetadata>,
    toolchain: Toolchain,
    now: i64,
    expires_at: i64,
    created_at: i64,
}

impl BuildLifecycleDecisionContext {
    fn with_created_at(mut self) -> Self {
        self.created_at = self
            .existing
            .as_ref()
            .map(|metadata| metadata.created_at)
            .unwrap_or(self.now);
        self
    }

    fn requires_rebuild(&self, storage: &Path) -> bool {
        match self.existing.as_ref() {
            Some(metadata) if metadata.status == BuildStatus::Ready => {
                !artifact_path(storage, &self.identity.id).exists()
                    || metadata.toolchain.fingerprint != self.toolchain.fingerprint
            }
            _ => true,
        }
    }

    fn pruned_download_tokens(&self) -> Vec<DownloadTokenRecord> {
        let mut download_tokens = self
            .existing
            .as_ref()
            .map(|metadata| metadata.download_tokens.clone())
            .unwrap_or_default();
        prune_tokens(&mut download_tokens, self.now);
        download_tokens
    }
}

enum BuildPipelineOutcome {
    Succeeded { signed_path: PathBuf },
    Failed(ApiError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::store::{
        base64url_no_pad, load_metadata, random_bytes, resolve_download, sha256_hex,
    };

    #[tokio::test]
    async fn failed_rebuild_replaces_ready_build_state_with_latest_submission() {
        let root = test_temp_dir("shortcut-forge-failed-rebuild");
        let storage = root.join("data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) = install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed v1");
        let lifecycle = make_lifecycle(&storage, &cherri, &shortcuts, 1);
        let source = "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string();

        let first = lifecycle
            .submit(BuildRequest {
                name: "Minimal".to_string(),
                source_format: "cherri".to_string(),
                source: source.clone(),
                sign_mode: "anyone".to_string(),
                ttl_seconds: 120,
            })
            .await
            .unwrap();
        let first_token = first.download_url.rsplit('/').next().unwrap().to_string();
        let first_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();

        write_executable(
            &cherri,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "Cherri Compiler v2.4.0"
  exit 0
fi
echo "compile failed" >&2
exit 1
"#,
        );

        let err = lifecycle
            .submit(BuildRequest {
                name: "Minimal Rebuild Failed".to_string(),
                source_format: "cherri".to_string(),
                source,
                sign_mode: "anyone".to_string(),
                ttl_seconds: 240,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "BUILD_FAILED");

        let failed_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        assert_eq!(failed_metadata.status, BuildStatus::Failed);
        assert_eq!(failed_metadata.name, "Minimal Rebuild Failed");
        assert_eq!(failed_metadata.created_at, first_metadata.created_at);
        assert!(failed_metadata.expires_at >= first_metadata.expires_at);
        assert!(failed_metadata.download_tokens.is_empty());
        assert_eq!(
            failed_metadata
                .error
                .as_ref()
                .map(|error| error.code.as_str()),
            Some("BUILD_FAILED")
        );
        assert!(
            resolve_download(&storage, &sha256_hex(first_token.as_bytes()), now_unix())
                .unwrap()
                .is_none()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn renewal_succeeds_while_rebuild_slot_is_saturated() {
        let root = test_temp_dir("shortcut-forge-renewal-slot");
        let tools = root.join("tools");
        let storage = root.join("data");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(&storage).unwrap();
        let cherri = tools.join("fake-cherri");
        let shortcuts = tools.join("fake-shortcuts");
        let marker = root.join("build-started");
        write_executable(
            &cherri,
            &format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "Cherri Compiler v2.3.0"
  exit 0
fi
source_file="$1"
if grep -q "Slow" "$source_file"; then
  printf started > "{}"
  sleep 1
fi
out=""
for arg in "$@"; do
  case "$arg" in
    --output=*) out="${{arg#--output=}}" ;;
  esac
done
if [ -z "$out" ]; then
  exit 2
fi
printf 'unsigned shortcut' > "$out"
"#,
                marker.display()
            ),
        );
        write_executable(
            &shortcuts,
            r#"#!/bin/sh
if [ "$1" = "help" ] && [ "$2" = "sign" ]; then
  echo "sign help"
  exit 0
fi
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 2
fi
printf 'signed shortcut bytes' > "$out"
"#,
        );

        let lifecycle = Arc::new(make_lifecycle(&storage, &cherri, &shortcuts, 1));
        let source = "#define name Ready\nshowNotification(\"ok\", \"ready\")\n".to_string();
        let first = lifecycle
            .submit(BuildRequest {
                name: "Ready".to_string(),
                source_format: "cherri".to_string(),
                source: source.clone(),
                sign_mode: "anyone".to_string(),
                ttl_seconds: 120,
            })
            .await
            .unwrap();

        let slow_lifecycle = Arc::clone(&lifecycle);
        let slow = tokio::spawn(async move {
            slow_lifecycle
                .submit(BuildRequest {
                    name: "Slow".to_string(),
                    source_format: "cherri".to_string(),
                    source: "#define name Slow\nshowNotification(\"ok\", \"slow\")\n".to_string(),
                    sign_mode: "anyone".to_string(),
                    ttl_seconds: 120,
                })
                .await
        });
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(marker.exists(), "slow rebuild did not start");

        let renewed = lifecycle
            .submit(BuildRequest {
                name: "Ready Renewed".to_string(),
                source_format: "cherri".to_string(),
                source,
                sign_mode: "anyone".to_string(),
                ttl_seconds: 240,
            })
            .await
            .unwrap();
        assert_eq!(renewed.id, first.id);
        assert_ne!(renewed.download_url, first.download_url);
        assert!(slow.await.unwrap().is_ok());

        fs::remove_dir_all(root).unwrap();
    }

    fn test_temp_dir(prefix: &str) -> PathBuf {
        for _ in 0..16 {
            let suffix = base64url_no_pad(&random_bytes(8).unwrap());
            let path = env::temp_dir().join(format!("{prefix}-{suffix}"));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => panic!("failed to create test temp dir: {err}"),
            }
        }
        panic!("failed to create unique test temp dir");
    }

    fn write_executable(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    fn install_fake_tools(
        root: &Path,
        cherri_version: &str,
        signed_text: &str,
    ) -> (PathBuf, PathBuf) {
        let tools = root.join("tools");
        fs::create_dir_all(&tools).unwrap();
        let cherri = tools.join("fake-cherri");
        let shortcuts = tools.join("fake-shortcuts");
        write_executable(
            &cherri,
            &format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "{cherri_version}"
  exit 0
fi
out=""
for arg in "$@"; do
  case "$arg" in
    --output=*) out="${{arg#--output=}}" ;;
  esac
done
if [ -z "$out" ]; then
  exit 2
fi
printf 'unsigned shortcut' > "$out"
"#
            ),
        );
        write_executable(
            &shortcuts,
            &format!(
                r#"#!/bin/sh
if [ "$1" = "help" ] && [ "$2" = "sign" ]; then
  echo "sign help"
  exit 0
fi
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 2
fi
printf '{signed_text}' > "$out"
"#
            ),
        );
        (cherri, shortcuts)
    }

    fn make_lifecycle(
        storage: &Path,
        cherri: &Path,
        shortcuts: &Path,
        max_build_concurrency: usize,
    ) -> BuildLifecycle {
        BuildLifecycle::new(Config {
            host: "127.0.0.1".to_string(),
            port: 8787,
            public_base_url: "http://127.0.0.1:8787".to_string(),
            storage: storage.to_path_buf(),
            max_source_bytes: crate::model::DEFAULT_MAX_SOURCE_BYTES,
            build_timeout: Duration::from_secs(5),
            max_build_concurrency,
            auth_token: "test-token".to_string(),
            health_cache_ttl: Duration::from_secs(60),
            cherri_bin: cherri.to_string_lossy().to_string(),
            shortcuts_bin: shortcuts.to_string_lossy().to_string(),
        })
    }
}
