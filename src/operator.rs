use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::net::{TcpListener, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::model::*;
use crate::config::*;
use crate::build::probe_toolchain;

pub struct InitConfig {
    pub config_path: PathBuf,
    pub host: String,
    pub port: u16,
    pub public_base_url: Option<String>,
    pub storage: PathBuf,
    pub non_interactive: bool,
    pub yes: bool,
}

pub struct DoctorConfig {
    pub config_path: PathBuf,
    pub json: bool,
}

pub struct OperatorCommand {
    pub config_path: PathBuf,
}

pub struct StatusConfig {
    pub config_path: PathBuf,
    pub json: bool,
}

pub struct LogsConfig {
    pub config_path: PathBuf,
    pub follow: bool,
    pub lines: usize,
}

pub struct ConfigShowCommand {
    pub config_path: PathBuf,
}

pub struct ConfigSetCommand {
    pub config_path: PathBuf,
    pub key: String,
    pub value: String,
}

pub struct TokenRotateCommand {
    pub config_path: PathBuf,
    pub print: bool,
}

pub struct SmokeCommand {
    pub config_path: PathBuf,
    pub request_path: Option<PathBuf>,
    pub output_path: PathBuf,
}

pub struct BuildCliCommand {
    pub config_path: PathBuf,
    pub request_path: PathBuf,
    pub json: bool,
}

struct ColorCodes {
    bold: &'static str,
    dim: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    reset: &'static str,
}

pub fn run_init(command: &InitConfig) -> Result<(), String> {
    let interactive_stdio = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive_stdio && !command.yes && !command.non_interactive {
        return Err("init needs a terminal or --yes/--non-interactive".to_string());
    }
    let c = if interactive_stdio {
        ColorCodes {
            bold: "\x1b[1m",
            dim: "\x1b[2m",
            yellow: "\x1b[33m",
            cyan: "\x1b[36m",
            reset: "\x1b[0m",
        }
    } else {
        ColorCodes {
            bold: "",
            dim: "",
            yellow: "",
            cyan: "",
            reset: "",
        }
    };

    if crate::config::migrate_legacy_config(&command.config_path)? {
        println!("Migrated legacy config from .conf to .toml: {}", command.config_path.display());
    }

    let app_support = app_support_dir()?;
    let log_dir = default_log_dir()?;
    let launch_agent_path = default_launch_agent_path()?;
    let existing_map = if command.config_path.exists() {
        Some(load_config_map_from_path(&command.config_path)?)
    } else {
        None
    };
    if command.config_path.exists() && !command.yes {
        if command.non_interactive {
            return Err(format!(
                "config already exists at {}; rerun with --yes to overwrite",
                command.config_path.display()
            ));
        }
        if !prompt_confirm(
            &format!(
                "Config already exists at {}. Overwrite generated files?",
                command.config_path.display()
            ),
            false,
        )? {
            println!("init cancelled");
            return Ok(());
        }
    }

    let mut host = command.host.clone();
    let mut port = command.port;
    let mut public_base_url = command
        .public_base_url
        .clone()
        .unwrap_or_else(|| suggest_public_base_url(port));
    let mut storage = command.storage.clone();
    let existing_auth_token = existing_map
        .as_ref()
        .and_then(|map| map.get("auth-token"))
        .filter(|value| !value.trim().is_empty())
        .cloned();
    let auth_token = existing_auth_token
        .clone()
        .unwrap_or(generate_service_auth_token().map_err(|err| err.to_string())?);
    let cherri_bin = existing_map
        .as_ref()
        .and_then(|map| map.get("cherri-bin").cloned())
        .or_else(|| resolve_command_path("cherri").map(|path| path.display().to_string()))
        .unwrap_or_else(|| "cherri".to_string());
    let shortcuts_bin = existing_map
        .as_ref()
        .and_then(|map| map.get("shortcuts-bin").cloned())
        .unwrap_or_else(|| "/usr/bin/shortcuts".to_string());
    let expired_before = existing_map
        .as_ref()
        .and_then(|map| map.get("expired-before").cloned())
        .unwrap_or_else(|| "30d".to_string());

    if interactive_stdio && !command.non_interactive && !command.yes {
        host = prompt_value("Bind host", &host)?;
        port = prompt_value("Port", &port.to_string())?
            .parse::<u16>()
            .map_err(|_| "port must be a u16".to_string())?;
        public_base_url = prompt_value("Public base URL", &suggest_public_base_url(port))?;
        let storage_text = prompt_value("Storage directory", &storage.display().to_string())?;
        storage = PathBuf::from(storage_text);
    }

    validate_config_assignment("host", &host)?;
    validate_config_assignment("port", &port.to_string())?;
    validate_config_assignment("public-base-url", &public_base_url)?;
    validate_config_assignment("storage", &storage.display().to_string())?;
    validate_config_assignment("cherri-bin", &cherri_bin)?;
    validate_config_assignment("shortcuts-bin", &shortcuts_bin)?;
    validate_config_assignment("expired-before", &expired_before)?;

    let config = Config {
        host,
        port,
        public_base_url: normalize_public_base_url(&public_base_url, port),
        storage,
        max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
        build_timeout: Duration::from_secs(DEFAULT_BUILD_TIMEOUT_SECONDS),
        max_build_concurrency: 1,
        auth_token,
        health_cache_ttl: Duration::from_secs(DEFAULT_HEALTH_CACHE_SECONDS),
        cherri_bin,
        shortcuts_bin,
    };

    println!("Writing Shortcut Forge setup:");
    println!("  config   {}", command.config_path.display());
    println!("  storage  {}", config.storage.display());
    println!("  logs     {}", log_dir.display());
    println!("  plist    {}", launch_agent_path.display());

    fs::create_dir_all(&app_support)
        .map_err(|err| format!("failed to create {}: {err}", app_support.display()))?;
    crate::store::set_private_dir(&app_support)
        .map_err(|err| format!("failed to chmod {}: {err}", app_support.display()))?;
    fs::create_dir_all(&config.storage)
        .map_err(|err| format!("failed to create {}: {err}", config.storage.display()))?;
    fs::create_dir_all(&log_dir)
        .map_err(|err| format!("failed to create {}: {err}", log_dir.display()))?;

    let config_text = render_operator_config(&config, &expired_before);
    atomic_write_restricted_file(&command.config_path, config_text.as_bytes(), 0o600)?;
    ensure_launch_agent_file(&command.config_path)?;

    let toolchain = probe_toolchain(&config);
    if !toolchain.is_available() {
        return Err(format!(
            "tool validation failed: cherri={} shortcuts_sign={}",
            toolchain.cherri, toolchain.shortcuts_sign
        ));
    }

    let start_now = interactive_stdio
        && !command.non_interactive
        && !command.yes
        && prompt_confirm("Load and start the service now?", false)?;
    if start_now {
        run_start(&OperatorCommand {
            config_path: command.config_path.clone(),
        })?;
    }

    println!("{}{}Initialized Shortcut Forge{}", c.bold, c.cyan, c.reset);
    println!();
    println!(
        "  {}service_url{}   {}",
        c.dim, c.reset, config.public_base_url
    );
    println!(
        "  {}config{}        {}",
        c.dim, c.reset, command.config_path.display()
    );
    println!();
    if existing_auth_token.is_some() {
        println!("  {}auth_token{}    {}{}[unchanged]", c.dim, c.reset, c.dim, c.reset);
    } else {
        println!(
            "  {}Save this token. It will not be displayed again.{}",
            c.yellow, c.reset
        );
        println!();
        println!(
            "  {}auth_token{}    {}{}{}",
            c.dim, c.reset, c.cyan, config.auth_token, c.reset
        );
    }
    println!();
    println!(
        "  {}smoke{}         shortcut-forge smoke --config \"{}\"",
        c.dim, c.reset, command.config_path.display()
    );
    Ok(())
}

pub fn run_doctor(command: &DoctorConfig) -> Result<bool, String> {
    let mut checks = Vec::new();
    checks.push(DoctorCheck {
        name: "macOS",
        ok: cfg!(target_os = "macos"),
        detail: if cfg!(target_os = "macos") {
            "detected macOS".to_string()
        } else {
            "Shortcut Forge requires macOS".to_string()
        },
        fix: (!cfg!(target_os = "macos")).then_some("Run Shortcut Forge on macOS.".to_string()),
    });

    let current_binary = env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    checks.push(DoctorCheck {
        name: "binary",
        ok: current_binary != "<unknown>",
        detail: current_binary.clone(),
        fix: (current_binary == "<unknown>")
            .then_some("Re-run the binary from a local path.".to_string()),
    });

    let config_map = if command.config_path.exists() {
        match load_config_map_from_path(&command.config_path) {
            Ok(config_map) => {
                checks.push(DoctorCheck {
                    name: "config",
                    ok: true,
                    detail: format!("loaded {}", command.config_path.display()),
                    fix: None,
                });
                Some(config_map)
            }
            Err(err) => {
                checks.push(DoctorCheck {
                    name: "config",
                    ok: false,
                    detail: err,
                    fix: Some(format!(
                        "Fix {} or re-run `shortcut-forge init`.",
                        command.config_path.display()
                    )),
                });
                None
            }
        }
    } else {
        checks.push(DoctorCheck {
            name: "config",
            ok: false,
            detail: format!("missing {}", command.config_path.display()),
            fix: Some("Run `shortcut-forge init`.".to_string()),
        });
        None
    };

    let parsed_config = config_map
        .as_ref()
        .and_then(|map| build_runtime_config_from_file(map, false).ok());
    if let Some(config) = parsed_config.as_ref() {
        checks.push(DoctorCheck {
            name: "auth",
            ok: !config.auth_token.is_empty(),
            detail: if config.auth_token.is_empty() {
                "auth token missing".to_string()
            } else {
                "auth token present".to_string()
            },
            fix: config.auth_token.is_empty().then_some(
                "Run `shortcut-forge token rotate` or `shortcut-forge init`.".to_string(),
            ),
        });
        let cherri_ok = Path::new(&config.cherri_bin).exists()
            || resolve_command_path(&config.cherri_bin).is_some();
        let cherri_version = probe_command_output(&config.cherri_bin, &["--version"]);
        checks.push(DoctorCheck {
            name: "cherri",
            ok: cherri_ok && cherri_version.is_some(),
            detail: cherri_version
                .unwrap_or_else(|| format!("unavailable at {}", config.cherri_bin)),
            fix: Some("Install Cherri with mise and update cherri_bin if needed.".to_string()),
        });
        checks.push(DoctorCheck {
            name: "shortcuts sign",
            ok: probe_command_success(&config.shortcuts_bin, &["help", "sign"]),
            detail: config.shortcuts_bin.clone(),
            fix: Some("Check macOS Shortcuts CLI availability.".to_string()),
        });
        checks.push(DoctorCheck {
            name: "storage",
            ok: ensure_writable_dir(&config.storage).is_ok(),
            detail: config.storage.display().to_string(),
            fix: Some("Ensure the storage directory exists and is writable.".to_string()),
        });
        let log_dir = default_log_dir()?;
        checks.push(DoctorCheck {
            name: "logs",
            ok: ensure_writable_dir(&log_dir).is_ok(),
            detail: log_dir.display().to_string(),
            fix: Some("Ensure the log directory exists and is writable.".to_string()),
        });
        let launch_status = launch_agent_status()?;
        let bind_result = TcpListener::bind((config.host.as_str(), config.port));
        let bind_ok = match bind_result {
            Ok(listener) => {
                drop(listener);
                true
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse && launch_status.running => true,
            Err(_) => false,
        };
        checks.push(DoctorCheck {
            name: "port",
            ok: bind_ok,
            detail: format!("{}:{}", config.host, config.port),
            fix: Some("Stop the conflicting process or change host/port.".to_string()),
        });
        let url_host = extract_url_host(&config.public_base_url)?;
        let resolvable = (url_host.as_str(), config.port).to_socket_addrs().is_ok();
        checks.push(DoctorCheck {
            name: "public_base_url",
            ok: resolvable,
            detail: config.public_base_url.clone(),
            fix: Some("Choose a resolvable LAN hostname or IP in public_base_url.".to_string()),
        });
        let launch_agent_path = default_launch_agent_path()?;
        let launch_agent_ok = if launch_agent_path.exists() {
            Command::new("plutil")
                .arg("-lint")
                .arg(&launch_agent_path)
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        } else {
            false
        };
        checks.push(DoctorCheck {
            name: "LaunchAgent",
            ok: launch_agent_ok,
            detail: launch_agent_path.display().to_string(),
            fix: Some("Run `shortcut-forge init` or `shortcut-forge start`.".to_string()),
        });
        let health = if launch_status.running {
            probe_local_health(config).ok()
        } else {
            None
        };
        checks.push(DoctorCheck {
            name: "service",
            ok: health.as_ref().map(|probe| probe.ok).unwrap_or(false),
            detail: health
                .as_ref()
                .map(|probe| probe.status.clone())
                .unwrap_or_else(|| "service not running".to_string()),
            fix: Some("Run `shortcut-forge start`.".to_string()),
        });
    }

    let all_ok = checks.iter().all(|check| check.ok);
    if command.json {
        let checks_json = checks
            .iter()
            .map(|check| {
                format!(
                    r#"{{"name":"{}","ok":{},"detail":"{}","fix":{}}}"#,
                    json_escape(check.name),
                    if check.ok { "true" } else { "false" },
                    json_escape(&check.detail),
                    check.fix.as_ref().map_or_else(
                        || "null".to_string(),
                        |fix| format!(r#""{}""#, json_escape(fix))
                    )
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            r#"{{"ok":{},"checks":[{}]}}"#,
            if all_ok { "true" } else { "false" },
            checks_json
        );
    } else {
        for check in &checks {
            print!(
                "[{}] {}: {}",
                if check.ok { "pass" } else { "fail" },
                check.name,
                check.detail
            );
            if let Some(fix) = &check.fix
                && !check.ok
            {
                print!(" | fix: {fix}");
            }
            println!();
        }
    }
    Ok(all_ok)
}

pub fn run_start(command: &OperatorCommand) -> Result<(), String> {
    let config = load_runtime_config_from_path(&command.config_path, true)?;
    fs::create_dir_all(&config.storage)
        .map_err(|err| format!("failed to create {}: {err}", config.storage.display()))?;
    ensure_log_files()?;
    let launch_agent_path = ensure_launch_agent_file(&command.config_path)?;
    let uid = current_uid_string()?;
    let target = launchctl_service_target()?;
    let status = launch_agent_status()?;
    if status.loaded {
        run_launchctl_command(&["kickstart", "-k", &target])?;
    } else {
        let domain = format!("gui/{uid}");
        let launch_agent_path_text = launch_agent_path.display().to_string();
        run_launchctl_command(&["bootstrap", &domain, &launch_agent_path_text])?;
    }
    let refreshed = launch_agent_status()?;
    println!("started Shortcut Forge");
    println!("state = {}", refreshed.state);
    if let Some(pid) = refreshed.pid {
        println!("pid = {pid}");
    }
    println!("health_url = {}/health", local_service_base_url(&config));
    Ok(())
}

pub fn run_stop(_command: &OperatorCommand) -> Result<(), String> {
    let target = launchctl_service_target()?;
    let status = launch_agent_status()?;
    if !status.loaded {
        println!("Shortcut Forge is already stopped");
        return Ok(());
    }
    run_launchctl_command(&["bootout", &target])?;
    println!("stopped Shortcut Forge");
    Ok(())
}

pub fn run_restart(command: &OperatorCommand) -> Result<(), String> {
    let status = launch_agent_status()?;
    if !status.loaded {
        return run_start(command);
    }
    let target = launchctl_service_target()?;
    run_launchctl_command(&["kickstart", "-k", &target])?;
    let config = load_runtime_config_from_path(&command.config_path, true)?;
    println!("restarted Shortcut Forge");
    println!("health_url = {}/health", local_service_base_url(&config));
    Ok(())
}

pub fn run_status(command: &StatusConfig) -> Result<(), String> {
    let launch_agent_path = default_launch_agent_path()?;
    let launch_status = launch_agent_status()?;
    let file_config = command
        .config_path
        .exists()
        .then(|| load_config_map_from_path(&command.config_path))
        .transpose()?;
    let config = file_config
        .as_ref()
        .map(|map| build_runtime_config_from_file(map, false))
        .transpose()?;
    let health = if launch_status.running {
        config.as_ref().map(probe_local_health).transpose()?
    } else {
        None
    };
    let stderr_tail = if health
        .as_ref()
        .map(|probe| !probe.ok)
        .unwrap_or(!launch_status.running)
    {
        let (_, stderr_path) = log_output_paths()?;
        tail_file_lines(&stderr_path, 8)
    } else {
        Vec::new()
    };

    if command.json {
        let stderr_json = stderr_tail
            .iter()
            .map(|line| format!(r#""{}""#, json_escape(line)))
            .collect::<Vec<_>>()
            .join(",");
        let health_json = health.as_ref().map_or_else(
            || "null".to_string(),
            |probe| {
                format!(
                    r#"{{"ok":{},"status":"{}","cherri":{},"shortcuts_sign":{},"detail":{}}}"#,
                    if probe.ok { "true" } else { "false" },
                    json_escape(&probe.status),
                    probe.cherri.as_ref().map_or_else(
                        || "null".to_string(),
                        |value| format!(r#""{}""#, json_escape(value))
                    ),
                    probe.shortcuts_sign.as_ref().map_or_else(
                        || "null".to_string(),
                        |value| format!(r#""{}""#, json_escape(value))
                    ),
                    probe.detail.as_ref().map_or_else(
                        || "null".to_string(),
                        |value| format!(r#""{}""#, json_escape(value))
                    ),
                )
            },
        );
        println!(
            r#"{{"installed":{},"launch_agent":"{}","loaded":{},"running":{},"pid":{},"state":"{}","config_path":"{}","configured_url":{},"storage":{},"health":{},"stderr_tail":[{}]}}"#,
            if command.config_path.exists() || launch_agent_path.exists() {
                "true"
            } else {
                "false"
            },
            json_escape(&launch_agent_path.display().to_string()),
            if launch_status.loaded {
                "true"
            } else {
                "false"
            },
            if launch_status.running {
                "true"
            } else {
                "false"
            },
            launch_status
                .pid
                .map_or_else(|| "null".to_string(), |pid| pid.to_string()),
            json_escape(&launch_status.state),
            json_escape(&command.config_path.display().to_string()),
            config.as_ref().map_or_else(
                || "null".to_string(),
                |config| format!(r#""{}""#, json_escape(&config.public_base_url))
            ),
            config.as_ref().map_or_else(
                || "null".to_string(),
                |config| format!(
                    r#""{}""#,
                    json_escape(&config.storage.display().to_string())
                )
            ),
            health_json,
            stderr_json
        );
        return Ok(());
    }

    println!(
        "state = {}",
        if !command.config_path.exists() && !launch_agent_path.exists() {
            "not-installed"
        } else if launch_status.running {
            "running"
        } else if launch_status.loaded {
            "stopped"
        } else {
            "not-loaded"
        }
    );
    if let Some(pid) = launch_status.pid {
        println!("pid = {pid}");
    }
    println!("config = {}", command.config_path.display());
    if let Some(config) = &config {
        println!("configured_url = {}", config.public_base_url);
        println!("storage = {}", config.storage.display());
    }
    println!("launch_agent = {}", launch_agent_path.display());
    if let Some(health) = &health {
        println!("health = {}", health.status);
        if let Some(cherri) = &health.cherri {
            println!("cherri = {cherri}");
        }
        if let Some(shortcuts_sign) = &health.shortcuts_sign {
            println!("shortcuts_sign = {shortcuts_sign}");
        }
    } else {
        println!("health = unavailable");
    }
    if !stderr_tail.is_empty() {
        println!("stderr_tail:");
        for line in &stderr_tail {
            println!("  {line}");
        }
    }
    Ok(())
}

pub fn run_logs(command: &LogsConfig) -> Result<(), String> {
    let _ = &command.config_path;
    let (stdout_path, stderr_path) = ensure_log_files()?;
    if command.follow {
        let status = Command::new("tail")
            .arg("-n")
            .arg(command.lines.to_string())
            .arg("-f")
            .arg(&stdout_path)
            .arg(&stderr_path)
            .status()
            .map_err(|err| format!("failed to run tail: {err}"))?;
        if !status.success() {
            return Err("tail --follow failed".to_string());
        }
        return Ok(());
    }
    let output = Command::new("tail")
        .arg("-n")
        .arg(command.lines.to_string())
        .arg(&stdout_path)
        .arg(&stderr_path)
        .output()
        .map_err(|err| format!("failed to run tail: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

pub fn run_config_show(command: &ConfigShowCommand) -> Result<(), String> {
    let raw = load_config_map_from_path(&command.config_path)?;
    let config = build_runtime_config_from_file(&raw, false)?;
    println!(
        "{}",
        redacted_effective_config(&config, raw.get("expired-before").map(String::as_str))
    );
    Ok(())
}

pub fn run_config_set(command: &ConfigSetCommand) -> Result<(), String> {
    if !is_supported_config_set_key(&command.key) {
        return Err(format!("unsupported config key: {}", command.key));
    }
    validate_config_assignment(&command.key, &command.value)?;
    update_config_file_value(&command.config_path, &command.key, &command.value)?;
    println!(
        "Updated {} in {}",
        command.key,
        command.config_path.display()
    );
    if should_restart_for_config_key(&command.key) {
        println!("Run `shortcut-forge restart` to apply this change.");
    }
    Ok(())
}

pub fn run_token_rotate(command: &TokenRotateCommand) -> Result<(), String> {
    if !command.config_path.exists() {
        return Err(format!(
            "config file not found: {}; run `shortcut-forge init` first",
            command.config_path.display()
        ));
    }
    let token = generate_service_auth_token().map_err(|err| err.to_string())?;
    update_config_file_value(&command.config_path, "auth-token", &token)?;
    println!(
        "Rotated service auth token in {}",
        command.config_path.display()
    );
    if command.print {
        println!("auth_token = {token}");
    } else {
        println!("auth_token = [redacted]");
    }
    println!("Update callers before restarting.");
    println!("Run `shortcut-forge restart` to apply this change.");
    Ok(())
}

pub fn run_smoke(command: &SmokeCommand) -> Result<(), String> {
    let config = load_runtime_config_from_path(&command.config_path, true)?;
    let request_path = find_smoke_request_path(command.request_path.as_deref())?;
    let build_url = format!("{}/api/builds", local_service_base_url(&config));
    let response = curl_json_request(
        "POST",
        &build_url,
        Some(&config.auth_token),
        Some(&request_path),
    )?;
    if response.status != 200 {
        return Err(api_message_from_json(&response.body)
            .unwrap_or_else(|| format!("build request failed with HTTP {}", response.status)));
    }
    let build = parse_build_api_response(&response.body)?;
    let local_download_url = localize_service_url(&build.download_url, &config)?;
    let download_status = curl_download(&local_download_url, &command.output_path)?;
    if download_status != 200 {
        return Err(format!("download failed with HTTP {download_status}"));
    }
    println!("Smoke build succeeded");
    println!("id = {}", build.id);
    println!("expires_at = {}", build.expires_at);
    println!("output = {}", command.output_path.display());
    Ok(())
}

pub fn run_build_command(command: &BuildCliCommand) -> Result<(), String> {
    let config = load_runtime_config_from_path(&command.config_path, true)?;
    let build_url = format!("{}/api/builds", local_service_base_url(&config));
    let response = curl_json_request(
        "POST",
        &build_url,
        Some(&config.auth_token),
        Some(&command.request_path),
    )?;
    if command.json {
        println!("{}", response.body);
    }
    if response.status != 200 {
        return Err(api_message_from_json(&response.body)
            .unwrap_or_else(|| format!("build request failed with HTTP {}", response.status)));
    }
    if !command.json {
        let build = parse_build_api_response(&response.body)?;
        println!("Build submitted");
        println!("id = {}", build.id);
        println!("download_url = {}", build.download_url);
        println!("expires_at = {}", build.expires_at);
    }
    Ok(())
}

pub fn launch_agent_status() -> Result<LaunchAgentStatus, String> {
    let target = launchctl_service_target()?;
    let output = Command::new("launchctl")
        .arg("print")
        .arg(&target)
        .output()
        .map_err(|err| format!("failed to run launchctl: {err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let raw = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    if !output.status.success() {
        let lowered = raw.to_ascii_lowercase();
        if lowered.contains("could not find service")
            || lowered.contains("not found")
            || lowered.contains("could not find domain")
        {
            return Ok(LaunchAgentStatus {
                loaded: false,
                running: false,
                pid: None,
                state: "stopped".to_string(),
                raw,
            });
        }
        return Err(format!("launchctl print failed: {}", raw.trim()));
    }
    let pid = raw.lines().find_map(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .and_then(|value| value.trim().parse::<u32>().ok())
    });
    let state = raw
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("state = ")
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_else(|| {
            if pid.is_some() {
                "running".to_string()
            } else {
                "loaded".to_string()
            }
        });
    Ok(LaunchAgentStatus {
        loaded: true,
        running: pid.is_some() || state == "running",
        pid,
        state,
        raw,
    })
}

pub fn ensure_launch_agent_file(config_path: &Path) -> Result<PathBuf, String> {
    let launch_agent_path = default_launch_agent_path()?;
    let working_dir = app_support_dir()?;
    let log_dir = default_log_dir()?;
    fs::create_dir_all(&working_dir)
        .map_err(|err| format!("failed to create {}: {err}", working_dir.display()))?;
    fs::create_dir_all(&log_dir)
        .map_err(|err| format!("failed to create {}: {err}", log_dir.display()))?;
    let binary_path =
        env::current_exe().map_err(|err| format!("failed to locate binary: {err}"))?;
    let content = render_launch_agent_plist(&binary_path, config_path, &working_dir, &log_dir);
    atomic_write_restricted_file(&launch_agent_path, content.as_bytes(), 0o600)?;
    let output = Command::new("plutil")
        .arg("-lint")
        .arg(&launch_agent_path)
        .output()
        .map_err(|err| format!("failed to run plutil: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "plutil -lint failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(launch_agent_path)
}

pub fn curl_json_request(
    method: &str,
    url: &str,
    auth_token: Option<&str>,
    body_file: Option<&Path>,
) -> Result<CurlResponse, String> {
    const MARKER: &str = "__SHORTCUT_FORGE_HTTP_STATUS__";
    let mut command = Command::new("curl");
    command.arg("-sS").arg("-X").arg(method);
    if let Some(auth_token) = auth_token {
        command
            .arg("-H")
            .arg(format!("Authorization: Bearer {auth_token}"));
    }
    if let Some(body_file) = body_file {
        command
            .arg("-H")
            .arg("Content-Type: application/json")
            .arg("--data-binary")
            .arg(format!("@{}", body_file.display()));
    }
    command.arg("-w").arg(format!("\n{MARKER}%{{http_code}}"));
    command.arg(url);
    let output = command
        .output()
        .map_err(|err| format!("failed to run curl: {err}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "curl request failed".to_string()
        } else {
            format!("curl request failed: {detail}")
        });
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let Some((body, status)) = text.rsplit_once(MARKER) else {
        return Err("curl response did not include an HTTP status".to_string());
    };
    let status = status
        .trim()
        .parse::<u16>()
        .map_err(|_| "curl returned an invalid HTTP status".to_string())?;
    Ok(CurlResponse {
        status,
        body: body.trim_end_matches('\n').to_string(),
    })
}

pub fn curl_download(url: &str, output_path: &Path) -> Result<u16, String> {
    const MARKER: &str = "__SHORTCUT_FORGE_HTTP_STATUS__";
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let output = Command::new("curl")
        .arg("-sS")
        .arg("-L")
        .arg("-o")
        .arg(output_path)
        .arg("-w")
        .arg(format!("{MARKER}%{{http_code}}"))
        .arg(url)
        .output()
        .map_err(|err| format!("failed to run curl: {err}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "curl download failed".to_string()
        } else {
            format!("curl download failed: {detail}")
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(status) = text.rsplit(MARKER).next() else {
        return Err("curl download did not include an HTTP status".to_string());
    };
    status
        .trim()
        .parse::<u16>()
        .map_err(|_| "curl returned an invalid HTTP status".to_string())
}

pub fn probe_local_health(config: &Config) -> Result<HealthProbe, String> {
    let url = format!("{}/health", local_service_base_url(config));
    let response = curl_json_request(
        "GET",
        &url,
        (!config.auth_token.is_empty()).then_some(config.auth_token.as_str()),
        None,
    )?;
    if response.status != 200 {
        return Ok(HealthProbe {
            ok: false,
            status: "error".to_string(),
            cherri: None,
            shortcuts_sign: None,
            detail: Some(
                api_message_from_json(&response.body)
                    .unwrap_or_else(|| format!("HTTP {}", response.status)),
            ),
        });
    }
    let value: serde_json::Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("invalid /health response: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "health response must be a JSON object".to_string())?;
    let data = object
        .get("data")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "health response missing data".to_string())?;
    Ok(HealthProbe {
        ok: true,
        status: data
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ok")
            .to_string(),
        cherri: data
            .get("cherri")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        shortcuts_sign: data
            .get("shortcuts_sign")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        detail: None,
    })
}

pub fn api_message_from_json(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    object
        .get("error")?
        .as_object()?
        .get("message")?
        .as_str()
        .map(ToString::to_string)
}

pub fn parse_build_api_response(body: &str) -> Result<BuildApiResult, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| format!("invalid build response: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "build response must be a JSON object".to_string())?;
    Ok(BuildApiResult {
        id: json_required_string(object, "id")?,
        download_url: json_required_string(object, "download_url")?,
        expires_at: json_required_string(object, "expires_at")?,
    })
}

pub fn tail_file_lines(path: &Path, lines: usize) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| {
            let mut lines_vec = text
                .lines()
                .map(|line| line.to_string())
                .collect::<Vec<_>>();
            if lines_vec.len() > lines {
                lines_vec = lines_vec.split_off(lines_vec.len() - lines);
            }
            lines_vec
        })
        .unwrap_or_default()
}

pub fn find_smoke_request_path(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let default = PathBuf::from("docs/examples/minimal-request.json");
    if default.exists() {
        Ok(default)
    } else {
        Err("smoke request file not found; pass --request <json-file>".to_string())
    }
}

pub fn ensure_writable_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    let probe = path.join(format!(".write-test-{}", std::process::id()));
    fs::write(&probe, b"ok")
        .map_err(|err| format!("failed to write {}: {err}", probe.display()))?;
    fs::remove_file(&probe)
        .map_err(|err| format!("failed to remove {}: {err}", probe.display()))?;
    Ok(())
}

pub fn update_config_file_value(path: &Path, key: &str, value: &str) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let file_key = config_key_for_file(key);
    let replacement = format!("{file_key} = {}", format_config_value_for_key(key, value));
    let mut out = Vec::new();
    let mut found = false;
    for raw_line in text.lines() {
        let line = strip_config_comment(raw_line).trim();
        if !line.is_empty()
            && let Some((raw_key, _)) = line.split_once('=')
            && normalize_config_key(raw_key.trim()) == key
        {
            out.push(replacement.clone());
            found = true;
        } else {
            out.push(raw_line.to_string());
        }
    }
    if !found {
        if !out.is_empty() && !out.last().is_some_and(|line| line.is_empty()) {
            out.push(String::new());
        }
        out.push(replacement);
    }
    let mut rendered = out.join("\n");
    rendered.push('\n');
    atomic_write_restricted_file(path, rendered.as_bytes(), 0o600)
}

pub fn atomic_write_restricted_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)
            .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
        file.write_all(bytes)
            .map_err(|err| format!("failed to write {}: {err}", tmp.display()))?;
        file.sync_all()
            .map_err(|err| format!("failed to sync {}: {err}", tmp.display()))?;
    }
    #[cfg(unix)]
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))
        .map_err(|err| format!("failed to chmod {}: {err}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    crate::store::sync_parent_dir(path).map_err(|err| format!("failed to sync {}: {err}", path.display()))?;
    Ok(())
}

pub fn ensure_log_files() -> Result<(PathBuf, PathBuf), String> {
    let (stdout_path, stderr_path) = log_output_paths()?;
    if let Some(parent) = stdout_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .map_err(|err| format!("failed to open {}: {err}", stdout_path.display()))?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .map_err(|err| format!("failed to open {}: {err}", stderr_path.display()))?;
    Ok((stdout_path, stderr_path))
}

pub fn log_output_paths() -> Result<(PathBuf, PathBuf), String> {
    let dir = default_log_dir()?;
    Ok((dir.join("stdout.log"), dir.join("stderr.log")))
}

pub fn prompt_value(prompt: &str, default: &str) -> Result<String, String> {
    print!("{prompt} [{default}]: ");
    io::stdout()
        .flush()
        .map_err(|err| format!("failed to flush stdout: {err}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| format!("failed to read input: {err}"))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

pub fn prompt_confirm(prompt: &str, default: bool) -> Result<bool, String> {
    let suffix = if default { "Y/n" } else { "y/N" };
    print!("{prompt} [{suffix}]: ");
    io::stdout()
        .flush()
        .map_err(|err| format!("failed to flush stdout: {err}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| format!("failed to read input: {err}"))?;
    let trimmed = line.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(default);
    }
    match trimmed.as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Err("please answer yes or no".to_string()),
    }
}

pub fn run_launchctl_command(args: &[&str]) -> Result<String, String> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|err| format!("failed to run launchctl: {err}"))?;
    if !output.status.success() {
        let detail = if output.stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        };
        return Err(if detail.is_empty() {
            format!("launchctl {} failed", args.join(" "))
        } else {
            detail
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn render_launch_agent_plist(binary_path: &Path, config_path: &Path, working_dir: &Path, log_dir: &Path) -> String {
    let stdout_path = log_dir.join("stdout.log");
    let stderr_path = log_dir.join("stderr.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{}</string>

  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>serve</string>
    <string>--config</string>
    <string>{}</string>
  </array>

  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>

  <key>WorkingDirectory</key>
  <string>{}</string>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(LAUNCH_AGENT_LABEL),
        xml_escape(&binary_path.display().to_string()),
        xml_escape(&config_path.display().to_string()),
        xml_escape(&working_dir.display().to_string()),
        xml_escape(&stdout_path.display().to_string()),
        xml_escape(&stderr_path.display().to_string()),
    )
}

pub fn generate_service_auth_token() -> io::Result<String> {
    let bytes = crate::store::random_bytes(32)?;
    Ok(crate::store::base64url_no_pad(&bytes))
}

// Private helpers to preserve original flat-config-file business logic.

fn load_config_map_from_path(path: &Path) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Err(format!("config file not found: {}", path.display()));
    }
    crate::config::load_config_file(path)
        .map_err(|err| format!("failed to load config {}: {err}", path.display()))
}

fn strip_config_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

fn normalize_config_key(key: &str) -> String {
    key.trim().replace('_', "-")
}

fn config_key_for_file(key: &str) -> String {
    key.replace('-', "_")
}

fn json_required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| format!("{key} is required"))
}
