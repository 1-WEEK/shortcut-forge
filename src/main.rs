mod api;
mod build;
mod cli;
mod config;
mod error;
mod http;
mod model;
mod operator;
mod state;
mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::cli::*;
use crate::config::{
    build_runtime_config, config_value, default_config_path, default_storage_dir,
    load_config_file, operator_config_path, parse_age,
};
use crate::model::*;
use crate::operator::*;
use crate::store::run_gc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = if args.len() > 1 && args[1].starts_with("--") {
        let mut new_args = vec![args[0].clone(), "serve".to_string()];
        new_args.extend(args.into_iter().skip(1));
        Cli::parse_from(&new_args)
    } else {
        Cli::parse()
    };

    let result = match cli.command {
        Commands::Serve(args) => run_serve(args).await,
        Commands::Gc(args) => run_gc_command(args),
        Commands::Init(args) => run_init_command(args),
        Commands::Doctor(args) => run_doctor_command(args),
        Commands::Start(args) => run_start_command(args),
        Commands::Stop(args) => run_stop_command(args),
        Commands::Restart(args) => run_restart_command(args),
        Commands::Status(args) => run_status_command(args),
        Commands::Logs(args) => run_logs_command(args),
        Commands::Config(cmd) => run_config_command(cmd),
        Commands::Token(cmd) => run_token_command(cmd),
        Commands::Smoke(args) => run_smoke_command(args),
        Commands::Build(args) => run_build_cli_command(args),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run_serve(args: ServeArgs) -> Result<(), String> {
    let flags = serve_args_to_flags(&args);
    let file_config = load_config_for_serve(&args)?;
    let config = build_runtime_config(&flags, &file_config, true)?;
    crate::http::serve(config).await.map_err(|e| format!("startup failed: {e}"))
}

fn serve_args_to_flags(args: &ServeArgs) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    if let Some(ref v) = args.host {
        flags.insert("host".to_string(), v.clone());
    }
    if let Some(v) = args.port {
        flags.insert("port".to_string(), v.to_string());
    }
    if let Some(ref v) = args.public_base_url {
        flags.insert("public-base-url".to_string(), v.clone());
    }
    if let Some(ref v) = args.storage {
        flags.insert("storage".to_string(), v.display().to_string());
    }
    if let Some(v) = args.max_source_bytes {
        flags.insert("max-source-bytes".to_string(), v.to_string());
    }
    if let Some(v) = args.build_timeout_seconds {
        flags.insert("build-timeout-seconds".to_string(), v.to_string());
    }
    if let Some(v) = args.max_build_concurrency {
        flags.insert("max-build-concurrency".to_string(), v.to_string());
    }
    if let Some(ref v) = args.auth_token {
        flags.insert("auth-token".to_string(), v.clone());
    }
    if let Some(v) = args.health_cache_ttl_seconds {
        flags.insert("health-cache-ttl-seconds".to_string(), v.to_string());
    }
    if let Some(ref v) = args.cherri_bin {
        flags.insert("cherri-bin".to_string(), v.clone());
    }
    if let Some(ref v) = args.shortcuts_bin {
        flags.insert("shortcuts-bin".to_string(), v.clone());
    }
    if let Some(ref v) = args.config {
        flags.insert("config".to_string(), v.display().to_string());
    }
    flags
}

fn load_config_for_serve(args: &ServeArgs) -> Result<HashMap<String, String>, String> {
    if let Some(ref path) = args.config {
        return load_config_file(path)
            .map_err(|e| format!("failed to load config {}: {e}", path.display()));
    }
    if let Ok(path) = std::env::var("SHORTCUT_SERVER_CONFIG") {
        return load_config_file(PathBuf::from(path).as_path())
            .map_err(|e| format!("failed to load config: {e}"));
    }
    Ok(HashMap::new())
}

fn run_gc_command(args: GcArgs) -> Result<(), String> {
    let flags = gc_args_to_flags(&args);
    let file_config = if let Some(ref path) = args.config {
        load_config_file(path).unwrap_or_default()
    } else {
        HashMap::new()
    };
    let storage = config_value(&flags, &file_config, "storage", "SHORTCUT_SERVER_STORAGE")
        .unwrap_or_else(|| "./data".to_string());
    let expired_before_age = config_value(
        &flags,
        &file_config,
        "expired-before",
        "SHORTCUT_SERVER_GC_EXPIRED_BEFORE",
    )
    .as_deref()
    .map(parse_age)
    .transpose()?
    .unwrap_or(Duration::from_secs(0));
    run_gc(&GcConfig {
        storage: PathBuf::from(storage),
        expired_before_age,
    })
    .map_err(|e| format!("gc failed: {e}"))
}

fn gc_args_to_flags(args: &GcArgs) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    if let Some(ref v) = args.storage {
        flags.insert("storage".to_string(), v.display().to_string());
    }
    if let Some(ref v) = args.expired_before {
        flags.insert("expired-before".to_string(), v.clone());
    }
    if let Some(ref v) = args.config {
        flags.insert("config".to_string(), v.display().to_string());
    }
    flags
}

fn run_init_command(args: InitArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_init(&InitConfig {
        config_path,
        host: args.host,
        port: args.port,
        public_base_url: args.public_base_url,
        storage: args.storage.unwrap_or_else(|| default_storage_dir().unwrap()),
        non_interactive: args.non_interactive,
        yes: args.yes,
    })
}

fn run_doctor_command(args: DoctorArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    match run_doctor(&DoctorConfig {
        config_path,
        json: args.json,
    }) {
        Ok(true) => Ok(()),
        Ok(false) => Err("doctor checks failed".to_string()),
        Err(err) => Err(format!("doctor failed: {err}")),
    }
}

fn run_start_command(args: OperatorArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_start(&OperatorCommand { config_path })
}

fn run_stop_command(args: OperatorArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_stop(&OperatorCommand { config_path })
}

fn run_restart_command(args: OperatorArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_restart(&OperatorCommand { config_path })
}

fn run_status_command(args: StatusArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_status(&StatusConfig {
        config_path,
        json: args.json,
    })
}

fn run_logs_command(args: LogsArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_logs(&LogsConfig {
        config_path,
        follow: args.follow,
        lines: args.lines,
    })
}

fn run_config_command(cmd: ConfigCmd) -> Result<(), String> {
    match cmd {
        ConfigCmd::Show(args) => {
            let config_path = operator_config_path(
                args.config.map(|p| p.display().to_string()),
                default_config_path()?,
            )?;
            run_config_show(&ConfigShowCommand { config_path })
        }
        ConfigCmd::Set(args) => {
            let config_path = operator_config_path(
                args.config.map(|p| p.display().to_string()),
                default_config_path()?,
            )?;
            run_config_set(&ConfigSetCommand {
                config_path,
                key: args.key,
                value: args.value,
            })
        }
    }
}

fn run_token_command(cmd: TokenCmd) -> Result<(), String> {
    match cmd {
        TokenCmd::Rotate(args) => {
            let config_path = operator_config_path(
                args.config.map(|p| p.display().to_string()),
                default_config_path()?,
            )?;
            run_token_rotate(&TokenRotateCommand {
                config_path,
                print: args.print,
            })
        }
    }
}

fn run_smoke_command(args: SmokeArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    run_smoke(&SmokeCommand {
        config_path,
        request_path: args.request,
        output_path: args.output,
    })
}

fn run_build_cli_command(args: BuildArgs) -> Result<(), String> {
    let config_path = operator_config_path(
        args.config.map(|p| p.display().to_string()),
        default_config_path()?,
    )?;
    crate::operator::run_build_command(&BuildCliCommand {
        config_path,
        request_path: args.request_path,
        json: args.json,
    })
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::api::parse_build_request;
    use crate::config::{
        authority_has_port, build_runtime_config_from_file, first_sanitized_line,
        load_config_file, normalize_public_base_url,
    };
    use crate::operator::update_config_file_value;
    use crate::model::{
        BuildMetadata, BuildRequest, BuildStatus, Config, DownloadTokenRecord, GcConfig, Toolchain,
        DEFAULT_MAX_SOURCE_BYTES, now_unix, format_rfc3339, parse_rfc3339,
    };
    use crate::state::{AppState, BuildSlot, StorageLock, build_or_renew};
    use crate::store::{
        artifact_path, base64url_no_pad, build_dir, is_valid_build_id, is_valid_download_token,
        load_metadata, metadata_path, random_bytes, resolve_download, run_gc, save_metadata,
        scan_metadata, sha256_hex,
    };
    use crate::build::run_build_pipeline;

    static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn json_parser_handles_escaped_source() {
        let body = br#"{"name":"T","source_format":"cherri","source":"showNotification(\"ok\", \"x\")\n","ttl_seconds":60}"#;
        let request = parse_build_request(body, 1024).unwrap();
        assert_eq!(request.source, "showNotification(\"ok\", \"x\")\n");
        assert_eq!(request.sign_mode, "anyone");
    }

    #[test]
    fn build_id_is_stable_32_hex() {
        let input = "cherri\nanyone\nshowNotification(\"ok\", \"x\")\n";
        let hash = sha256_hex(input.as_bytes());
        let id = &hash[..crate::model::BUILD_ID_LEN];
        assert_eq!(id.len(), 32);
        assert!(is_valid_build_id(id));
    }

    #[test]
    fn token_format_is_url_safe_and_not_build_id_like() {
        let token = crate::store::generate_download_token().unwrap();
        assert!(is_valid_download_token(&token));
        assert!(token.starts_with("dl_"));
        assert!(!is_valid_build_id(&token));
    }

    #[test]
    fn metadata_does_not_emit_plaintext_token_or_download_url() {
        let token = "dl_plaintext-secret-token";
        let metadata = BuildMetadata {
            id: "0123456789abcdef0123456789abcdef".to_string(),
            name: "Name".to_string(),
            source_format: "cherri".to_string(),
            source_hash: "a".repeat(64),
            sign_mode: "anyone".to_string(),
            status: BuildStatus::Ready,
            download_tokens: vec![DownloadTokenRecord {
                hash: sha256_hex(token.as_bytes()),
                expires_at: 1_800_000_000,
            }],
            toolchain: Toolchain {
                cherri: "Cherri Compiler v2.3.0".to_string(),
                shortcuts_sign: "available".to_string(),
                fingerprint: "b".repeat(64),
            },
            created_at: 1_700_000_000,
            updated_at: 1_700_000_001,
            expires_at: 1_800_000_000,
            error: None,
        };
        let storage_json = metadata.to_storage_json();
        assert!(!storage_json.contains(token));
        assert!(!storage_json.contains("download_url"));
        assert!(storage_json.contains(&sha256_hex(token.as_bytes())));

        let api_json = metadata.to_api_json(1_700_000_002);
        assert!(api_json.contains(r#""download_url":null"#));
        assert!(api_json.contains(r#""download_token_count":1"#));
        assert!(!api_json.contains(&sha256_hex(token.as_bytes())));
    }

    #[test]
    fn expired_ready_metadata_reports_expired() {
        let metadata = BuildMetadata {
            id: "0123456789abcdef0123456789abcdef".to_string(),
            name: "Name".to_string(),
            source_format: "cherri".to_string(),
            source_hash: "a".repeat(64),
            sign_mode: "anyone".to_string(),
            status: BuildStatus::Ready,
            download_tokens: Vec::new(),
            toolchain: Toolchain {
                cherri: "available".to_string(),
                shortcuts_sign: "available".to_string(),
                fingerprint: "b".repeat(64),
            },
            created_at: 100,
            updated_at: 100,
            expires_at: 120,
            error: None,
        };
        assert_eq!(metadata.status_for_api(121), "expired");
    }

    #[test]
    fn rfc3339_round_trip() {
        let ts = 1_764_000_000;
        let text = format_rfc3339(ts);
        assert_eq!(parse_rfc3339(&text), Some(ts));
    }

    #[test]
    fn tool_probe_sanitizes_ansi_escape_sequences() {
        assert_eq!(
            first_sanitized_line("Cherri Compiler \u{1b}[32mv2.3.0\u{1b}[0m\n"),
            Some("Cherri Compiler v2.3.0".to_string())
        );
    }

    #[test]
    fn config_file_parses_toml_values_and_comments() {
        let root = test_temp_dir("shortcut-forge-config-file");
        let path = root.join("shortcut-forge.conf");
        fs::write(
            &path,
            r#"
# TOML config.
host = "0.0.0.0"
port = 8787
public_base_url = "http://mac-mini.local:8787"
storage = "/Users/test/Library/Application Support/ShortcutForge/data"
auth_token = "token#with-hash"
cherri_bin = "/opt/cherri"
"#,
        )
        .unwrap();

        let config = load_config_file(&path).unwrap();
        assert_eq!(config.get("host").unwrap(), "0.0.0.0");
        assert_eq!(config.get("port").unwrap(), "8787");
        assert_eq!(
            config.get("public-base-url").unwrap(),
            "http://mac-mini.local:8787"
        );
        assert_eq!(
            config.get("storage").unwrap(),
            "/Users/test/Library/Application Support/ShortcutForge/data"
        );
        assert_eq!(config.get("auth-token").unwrap(), "token#with-hash");
        assert_eq!(config.get("cherri-bin").unwrap(), "/opt/cherri");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_values_allow_flag_override() {
        let mut flags = HashMap::new();
        flags.insert("port".to_string(), "9999".to_string());
        let mut file_config = HashMap::new();
        file_config.insert("port".to_string(), "8787".to_string());

        assert_eq!(
            crate::config::parse_u16_config(&flags, &file_config, "port", "SHORTCUT_FORGE_TEST_PORT").unwrap(),
            Some(9999)
        );
    }

    #[test]
    fn config_update_rewrites_matching_key() {
        let root = test_temp_dir("shortcut-forge-config-update");
        let path = root.join("shortcut-forge.conf");
        fs::write(
            &path,
            r#"# comment
port = 8787
public_base_url = "http://old.local:8787"
"#,
        )
        .unwrap();

        update_config_file_value(&path, "public-base-url", "http://new.local:8787").unwrap();

        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.contains("# comment"));
        assert!(updated.contains("port = 8787"));
        assert!(updated.contains(r#"public_base_url = "http://new.local:8787""#));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_config_from_file_ignores_env_overrides() {
        let _guard = CURRENT_DIR_LOCK.lock().unwrap();
        let mut file_config = HashMap::new();
        file_config.insert("port".to_string(), "8787".to_string());
        file_config.insert("auth-token".to_string(), "from-file".to_string());
        unsafe {
            env::set_var("SHORTCUT_SERVER_PORT", "9999");
        }

        let config = build_runtime_config_from_file(&file_config, true).unwrap();

        unsafe {
            env::remove_var("SHORTCUT_SERVER_PORT");
        }
        assert_eq!(config.port, 8787);
    }

    #[tokio::test]
    async fn build_renewal_rotates_download_url_without_plaintext_token_storage() {
        let root = test_temp_dir("shortcut-forge-build-renewal");
        let tools = root.join("tools");
        let storage = root.join("data");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(&storage).unwrap();
        let cherri = tools.join("fake-cherri");
        let shortcuts = tools.join("fake-shortcuts");
        write_executable(
            &cherri,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "Cherri Compiler v2.3.0"
  exit 0
fi
out=""
for arg in "$@"; do
  case "$arg" in
    --output=*) out="${arg#--output=}" ;;
  esac
done
if [ -z "$out" ]; then
  exit 2
fi
printf 'unsigned shortcut' > "$out"
"#,
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

        let state = AppState {
            config: Config {
                host: "127.0.0.1".to_string(),
                port: 8787,
                public_base_url: "http://127.0.0.1:8787".to_string(),
                storage: storage.clone(),
                max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
                build_timeout: Duration::from_secs(5),
                max_build_concurrency: 1,
                auth_token: "test-token".to_string(),
                health_cache_ttl: Duration::from_secs(60),
                cherri_bin: cherri.to_string_lossy().to_string(),
                shortcuts_bin: shortcuts.to_string_lossy().to_string(),
            },
            build_locks: tokio::sync::Mutex::new(HashMap::new()),
            build_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            health_cache: tokio::sync::Mutex::new(None),
            _storage_lock: StorageLock::acquire(&storage).unwrap(),
        };

        let request = BuildRequest {
            name: "Minimal".to_string(),
            source_format: "cherri".to_string(),
            source: "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string(),
            sign_mode: "anyone".to_string(),
            ttl_seconds: 120,
        };
        let first = build_or_renew(request, &state).await.unwrap();
        let first_token = first.download_url.rsplit('/').next().unwrap().to_string();
        let first_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        assert_eq!(first.id.len(), crate::model::BUILD_ID_LEN);
        assert!(is_valid_download_token(&first_token));
        assert_eq!(
            fs::read_to_string(artifact_path(&storage, &first.id)).unwrap(),
            "signed shortcut bytes"
        );

        let request = BuildRequest {
            name: "Minimal Updated".to_string(),
            source_format: "cherri".to_string(),
            source: "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string(),
            sign_mode: "anyone".to_string(),
            ttl_seconds: 240,
        };
        let second = build_or_renew(request, &state).await.unwrap();
        let second_token = second.download_url.rsplit('/').next().unwrap().to_string();
        assert_eq!(first.id, second.id);
        assert_ne!(first.download_url, second.download_url);
        assert!(is_valid_download_token(&second_token));

        let metadata_text = fs::read_to_string(metadata_path(&storage, &second.id)).unwrap();
        assert!(!metadata_text.contains(&first_token));
        assert!(!metadata_text.contains(&second_token));
        assert!(metadata_text.contains(&sha256_hex(first_token.as_bytes())));
        assert!(metadata_text.contains(&sha256_hex(second_token.as_bytes())));
        assert!(!metadata_text.contains("download_url"));

        let metadata = load_metadata(&storage, &second.id).unwrap().unwrap();
        assert_eq!(metadata.name, "Minimal Updated");
        assert_eq!(metadata.download_tokens.len(), 2);
        assert_eq!(metadata.created_at, first_metadata.created_at);
        assert!(metadata.expires_at >= first_metadata.expires_at);
        assert!(
            resolve_download(&storage, &sha256_hex(first_token.as_bytes()), now_unix())
                .unwrap()
                .is_some()
        );
        assert!(
            resolve_download(&storage, &sha256_hex(second_token.as_bytes()), now_unix())
                .unwrap()
                .is_some()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn expired_download_stops_and_repost_refreshes_same_id() {
        let root = test_temp_dir("shortcut-forge-expiry-renewal");
        let storage = root.join("data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) = install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed v1");
        let state = make_test_state(&storage, &cherri, &shortcuts);
        let source = "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string();

        let first = build_or_renew(
            BuildRequest {
                name: "Minimal".to_string(),
                source_format: "cherri".to_string(),
                source: source.clone(),
                sign_mode: "anyone".to_string(),
                ttl_seconds: 120,
            },
            &state,
        )
        .await
        .unwrap();
        let first_token = first.download_url.rsplit('/').next().unwrap().to_string();
        let mut metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        metadata.expires_at = now_unix() - 1;
        for token in &mut metadata.download_tokens {
            token.expires_at = metadata.expires_at;
        }
        save_metadata(&storage, &metadata).unwrap();

        let expired_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        assert_eq!(expired_metadata.status_for_api(now_unix()), "expired");
        assert!(
            resolve_download(&storage, &sha256_hex(first_token.as_bytes()), now_unix())
                .unwrap()
                .is_none()
        );

        let renewed = build_or_renew(
            BuildRequest {
                name: "Minimal Renewed".to_string(),
                source_format: "cherri".to_string(),
                source,
                sign_mode: "anyone".to_string(),
                ttl_seconds: 240,
            },
            &state,
        )
        .await
        .unwrap();
        let renewed_token = renewed.download_url.rsplit('/').next().unwrap().to_string();
        assert_eq!(first.id, renewed.id);
        assert_ne!(first.download_url, renewed.download_url);
        let renewed_metadata = load_metadata(&storage, &renewed.id).unwrap().unwrap();
        assert_eq!(renewed_metadata.status_for_api(now_unix()), "ready");
        assert_eq!(renewed_metadata.download_tokens.len(), 1);
        assert!(
            resolve_download(&storage, &sha256_hex(renewed_token.as_bytes()), now_unix())
                .unwrap()
                .is_some()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn toolchain_fingerprint_change_rebuilds_same_id() {
        let root = test_temp_dir("shortcut-forge-toolchain-rebuild");
        let storage = root.join("data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) = install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed v1");
        let state = make_test_state(&storage, &cherri, &shortcuts);
        let source = "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string();

        let first = build_or_renew(
            BuildRequest {
                name: "Minimal".to_string(),
                source_format: "cherri".to_string(),
                source: source.clone(),
                sign_mode: "anyone".to_string(),
                ttl_seconds: 120,
            },
            &state,
        )
        .await
        .unwrap();
        let first_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(artifact_path(&storage, &first.id)).unwrap(),
            "signed v1"
        );

        install_fake_tools(&root, "Cherri Compiler v2.4.0", "signed v2");
        let second = build_or_renew(
            BuildRequest {
                name: "Minimal".to_string(),
                source_format: "cherri".to_string(),
                source,
                sign_mode: "anyone".to_string(),
                ttl_seconds: 240,
            },
            &state,
        )
        .await
        .unwrap();
        let second_metadata = load_metadata(&storage, &second.id).unwrap().unwrap();
        assert_eq!(first.id, second.id);
        assert_ne!(
            first_metadata.toolchain.fingerprint,
            second_metadata.toolchain.fingerprint
        );
        assert_eq!(
            fs::read_to_string(artifact_path(&storage, &second.id)).unwrap(),
            "signed v2"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn gc_removes_expired_build_directories() {
        let root = test_temp_dir("shortcut-forge-gc");
        let storage = root.join("data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) = install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed v1");
        {
            let state = make_test_state(&storage, &cherri, &shortcuts);
            let built = build_or_renew(
                BuildRequest {
                    name: "Minimal".to_string(),
                    source_format: "cherri".to_string(),
                    source: "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string(),
                    sign_mode: "anyone".to_string(),
                    ttl_seconds: 120,
                },
                &state,
            )
            .await
            .unwrap();
            let mut metadata = load_metadata(&storage, &built.id).unwrap().unwrap();
            metadata.expires_at = now_unix() - 10;
            save_metadata(&storage, &metadata).unwrap();
            assert!(build_dir(&storage, &built.id).exists());
        }

        run_gc(&GcConfig {
            storage: storage.clone(),
            expired_before_age: Duration::from_secs(0),
        })
        .unwrap();
        assert!(scan_metadata(&storage).unwrap().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn build_slot_saturation_returns_server_busy() {
        let root = test_temp_dir("shortcut-forge-server-busy");
        let storage = root.join("data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) = install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed v1");
        let state = make_test_state(&storage, &cherri, &shortcuts);

        let first = BuildSlot::try_acquire(&state).unwrap();
        match BuildSlot::try_acquire(&state) {
            Ok(_) => panic!("second build slot should not be acquired"),
            Err(err) => {
                assert_eq!(err.code, "SERVER_BUSY");
                assert_eq!(err.status, 503);
            }
        }
        drop(first);
        assert!(BuildSlot::try_acquire(&state).is_ok());

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn build_pipeline_handles_relative_storage_paths() {
        let _guard = CURRENT_DIR_LOCK.lock().unwrap();
        let root = test_temp_dir("shortcut-forge-relative-storage");
        let original_dir = env::current_dir().unwrap();
        let relative_root = root.join("work");
        let storage = relative_root.join("relative-data");
        fs::create_dir_all(&storage).unwrap();
        let (cherri, shortcuts) =
            install_fake_tools(&root, "Cherri Compiler v2.3.0", "signed relative");

        env::set_current_dir(&relative_root).unwrap();
        let result = run_build_pipeline(
            &BuildRequest {
                name: "Minimal".to_string(),
                source_format: "cherri".to_string(),
                source: "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string(),
                sign_mode: "anyone".to_string(),
                ttl_seconds: 120,
            },
            "0123456789abcdef0123456789abcdef",
            &Config {
                host: "127.0.0.1".to_string(),
                port: 8787,
                public_base_url: "http://127.0.0.1:8787".to_string(),
                storage: PathBuf::from("./relative-data"),
                max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
                build_timeout: Duration::from_secs(5),
                max_build_concurrency: 1,
                auth_token: "test-token".to_string(),
                health_cache_ttl: Duration::from_secs(60),
                cherri_bin: cherri.to_string_lossy().to_string(),
                shortcuts_bin: shortcuts.to_string_lossy().to_string(),
            },
        )
        .await;
        env::set_current_dir(original_dir).unwrap();

        let signed_path = result.unwrap();
        assert_eq!(fs::read_to_string(&signed_path).unwrap(), "signed relative");
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
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
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

    fn make_test_state(storage: &Path, cherri: &Path, shortcuts: &Path) -> AppState {
        AppState {
            config: Config {
                host: "127.0.0.1".to_string(),
                port: 8787,
                public_base_url: "http://127.0.0.1:8787".to_string(),
                storage: storage.to_path_buf(),
                max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
                build_timeout: Duration::from_secs(5),
                max_build_concurrency: 1,
                auth_token: "test-token".to_string(),
                health_cache_ttl: Duration::from_secs(60),
                cherri_bin: cherri.to_string_lossy().to_string(),
                shortcuts_bin: shortcuts.to_string_lossy().to_string(),
            },
            build_locks: tokio::sync::Mutex::new(HashMap::new()),
            build_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            health_cache: tokio::sync::Mutex::new(None),
            _storage_lock: StorageLock::acquire(storage).unwrap(),
        }
    }

    #[test]
    fn normalize_public_base_url_appends_non_default_port() {
        assert_eq!(
            normalize_public_base_url("http://mac-mini.home", 8787),
            "http://mac-mini.home:8787"
        );
    }

    #[test]
    fn normalize_public_base_url_preserves_existing_port() {
        assert_eq!(
            normalize_public_base_url("http://mac-mini.home:8787", 8787),
            "http://mac-mini.home:8787"
        );
        assert_eq!(
            normalize_public_base_url("http://mac-mini.home:8080", 8787),
            "http://mac-mini.home:8080"
        );
    }

    #[test]
    fn normalize_public_base_url_skips_default_ports() {
        assert_eq!(
            normalize_public_base_url("http://mac-mini.home", 80),
            "http://mac-mini.home"
        );
        assert_eq!(
            normalize_public_base_url("https://mac-mini.home", 443),
            "https://mac-mini.home"
        );
    }

    #[test]
    fn normalize_public_base_url_handles_ipv6() {
        assert_eq!(
            normalize_public_base_url("http://[::1]", 8787),
            "http://[::1]:8787"
        );
        assert_eq!(
            normalize_public_base_url("http://[::1]:8787", 8787),
            "http://[::1]:8787"
        );
    }

    #[test]
    fn normalize_public_base_url_trims_trailing_slash() {
        assert_eq!(
            normalize_public_base_url("http://mac-mini.home/", 8787),
            "http://mac-mini.home:8787"
        );
    }

    #[test]
    fn authority_has_port_detects_ports_correctly() {
        assert!(!authority_has_port("mac-mini.home"));
        assert!(authority_has_port("mac-mini.home:8787"));
        assert!(!authority_has_port("[::1]"));
        assert!(authority_has_port("[::1]:8787"));
        assert!(!authority_has_port("user:pass@mac-mini.home"));
        assert!(authority_has_port("user:pass@mac-mini.home:8787"));
    }
}
