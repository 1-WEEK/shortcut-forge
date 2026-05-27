use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_ID_LEN: usize = 32;
const DEFAULT_PORT: u16 = 8787;
const DEFAULT_MAX_SOURCE_BYTES: usize = 524_288;
const DEFAULT_BUILD_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_HEALTH_CACHE_SECONDS: u64 = 60;
const DEFAULT_TTL_SECONDS: u64 = 2_592_000;
const MIN_TTL_SECONDS: u64 = 60;
const MAX_TTL_SECONDS: u64 = 2_592_000;
const HEADER_LIMIT: usize = 32 * 1024;
const LAUNCH_AGENT_LABEL: &str = "com.shortcut-forge";
const DEFAULT_INIT_HOST: &str = "0.0.0.0";
const DEFAULT_LOG_LINES: usize = 80;

fn main() {
    match parse_cli(env::args().skip(1).collect()) {
        Ok(CommandMode::Serve(config)) => {
            if let Err(err) = serve(config) {
                eprintln!("startup failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Gc(gc)) => {
            if let Err(err) = run_gc(&gc) {
                eprintln!("gc failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Init(init)) => {
            if let Err(err) = run_init(&init) {
                eprintln!("init failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Doctor(doctor)) => match run_doctor(&doctor) {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(err) => {
                eprintln!("doctor failed: {err}");
                std::process::exit(1);
            }
        },
        Ok(CommandMode::Start(command)) => {
            if let Err(err) = run_start(&command) {
                eprintln!("start failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Stop(command)) => {
            if let Err(err) = run_stop(&command) {
                eprintln!("stop failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Restart(command)) => {
            if let Err(err) = run_restart(&command) {
                eprintln!("restart failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Status(command)) => {
            if let Err(err) = run_status(&command) {
                eprintln!("status failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Logs(command)) => {
            if let Err(err) = run_logs(&command) {
                eprintln!("logs failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::ConfigShow(command)) => {
            if let Err(err) = run_config_show(&command) {
                eprintln!("config show failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::ConfigSet(command)) => {
            if let Err(err) = run_config_set(&command) {
                eprintln!("config set failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::TokenRotate(command)) => {
            if let Err(err) = run_token_rotate(&command) {
                eprintln!("token rotate failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Smoke(command)) => {
            if let Err(err) = run_smoke(&command) {
                eprintln!("smoke failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Build(command)) => {
            if let Err(err) = run_build_command(&command) {
                eprintln!("build failed: {err}");
                std::process::exit(1);
            }
        }
        Ok(CommandMode::Help) => {
            print_help();
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!();
            print_help();
            std::process::exit(2);
        }
    }
}

#[derive(Clone, Debug)]
struct Config {
    host: String,
    port: u16,
    public_base_url: String,
    storage: PathBuf,
    max_source_bytes: usize,
    build_timeout: Duration,
    max_build_concurrency: usize,
    auth_token: String,
    health_cache_ttl: Duration,
    cherri_bin: String,
    shortcuts_bin: String,
}

#[derive(Debug)]
struct GcConfig {
    storage: PathBuf,
    expired_before_age: Duration,
}

#[derive(Debug)]
struct InitConfig {
    config_path: PathBuf,
    host: String,
    port: u16,
    public_base_url: Option<String>,
    storage: PathBuf,
    non_interactive: bool,
    yes: bool,
}

#[derive(Debug)]
struct OperatorCommand {
    config_path: PathBuf,
}

#[derive(Debug)]
struct DoctorConfig {
    config_path: PathBuf,
    json: bool,
}

#[derive(Debug)]
struct StatusConfig {
    config_path: PathBuf,
    json: bool,
}

#[derive(Debug)]
struct LogsConfig {
    config_path: PathBuf,
    follow: bool,
    lines: usize,
}

#[derive(Debug)]
struct ConfigShowCommand {
    config_path: PathBuf,
}

#[derive(Debug)]
struct ConfigSetCommand {
    config_path: PathBuf,
    key: String,
    value: String,
}

#[derive(Debug)]
struct TokenRotateCommand {
    config_path: PathBuf,
    print: bool,
}

#[derive(Debug)]
struct SmokeCommand {
    config_path: PathBuf,
    request_path: Option<PathBuf>,
    output_path: PathBuf,
}

#[derive(Debug)]
struct BuildCliCommand {
    config_path: PathBuf,
    request_path: PathBuf,
    json: bool,
}

enum CommandMode {
    Serve(Config),
    Gc(GcConfig),
    Init(InitConfig),
    Doctor(DoctorConfig),
    Start(OperatorCommand),
    Stop(OperatorCommand),
    Restart(OperatorCommand),
    Status(StatusConfig),
    Logs(LogsConfig),
    ConfigShow(ConfigShowCommand),
    ConfigSet(ConfigSetCommand),
    TokenRotate(TokenRotateCommand),
    Smoke(SmokeCommand),
    Build(BuildCliCommand),
    Help,
}

fn parse_cli(args: Vec<String>) -> Result<CommandMode, String> {
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(CommandMode::Help);
    }
    match args[0].as_str() {
        "serve" => {
            let (flags, positionals) = parse_option_flags(
                &args[1..],
                &[
                    "config",
                    "host",
                    "port",
                    "public-base-url",
                    "storage",
                    "max-source-bytes",
                    "build-timeout-seconds",
                    "max-build-concurrency",
                    "auth-token",
                    "health-cache-ttl-seconds",
                    "cherri-bin",
                    "shortcuts-bin",
                ],
                &[],
            )?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            let file_config = load_config_for_flags(&flags)?;
            Ok(CommandMode::Serve(build_runtime_config(
                &flags,
                &file_config,
                true,
            )?))
        }
        "gc" => {
            let (flags, positionals) =
                parse_option_flags(&args[1..], &["config", "storage", "expired-before"], &[])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            let file_config = load_config_for_flags(&flags)?;
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
            Ok(CommandMode::Gc(GcConfig {
                storage: PathBuf::from(storage),
                expired_before_age,
            }))
        }
        "init" => {
            let (flags, positionals) = parse_option_flags(
                &args[1..],
                &["config", "host", "port", "public-base-url", "storage"],
                &["non-interactive", "yes"],
            )?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            let config_path =
                operator_config_path(flags.get("config").cloned(), default_config_path()?)?;
            let host = flags
                .get("host")
                .cloned()
                .unwrap_or_else(|| DEFAULT_INIT_HOST.to_string());
            if host.trim().is_empty() {
                return Err("--host must not be empty".to_string());
            }
            let port = parse_u16_value(flags.get("port").cloned(), "port")?.unwrap_or(DEFAULT_PORT);
            let storage = flags
                .get("storage")
                .cloned()
                .map(PathBuf::from)
                .unwrap_or(default_storage_dir()?);
            Ok(CommandMode::Init(InitConfig {
                config_path,
                host,
                port,
                public_base_url: flags.get("public-base-url").cloned(),
                storage,
                non_interactive: has_bool_flag(&flags, "non-interactive"),
                yes: has_bool_flag(&flags, "yes"),
            }))
        }
        "doctor" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &["json"])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            Ok(CommandMode::Doctor(DoctorConfig {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                json: has_bool_flag(&flags, "json"),
            }))
        }
        "start" => Ok(CommandMode::Start(parse_operator_command(&args[1..])?)),
        "stop" => Ok(CommandMode::Stop(parse_operator_command(&args[1..])?)),
        "restart" => Ok(CommandMode::Restart(parse_operator_command(&args[1..])?)),
        "status" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &["json"])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            Ok(CommandMode::Status(StatusConfig {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                json: has_bool_flag(&flags, "json"),
            }))
        }
        "logs" => {
            let (flags, positionals) =
                parse_option_flags(&args[1..], &["config", "lines"], &["follow"])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            let lines = parse_usize_value(flags.get("lines").cloned(), "lines")?
                .unwrap_or(DEFAULT_LOG_LINES);
            if lines == 0 {
                return Err("--lines must be at least 1".to_string());
            }
            Ok(CommandMode::Logs(LogsConfig {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                follow: has_bool_flag(&flags, "follow"),
                lines,
            }))
        }
        "config" => parse_config_command(&args[1..]),
        "token" => parse_token_command(&args[1..]),
        "smoke" => {
            let (flags, positionals) =
                parse_option_flags(&args[1..], &["config", "request", "output"], &[])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            Ok(CommandMode::Smoke(SmokeCommand {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                request_path: flags.get("request").map(PathBuf::from),
                output_path: flags
                    .get("output")
                    .cloned()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/tmp/minimal.signed.shortcut")),
            }))
        }
        "build" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &["json"])?;
            if positionals.len() != 1 {
                return Err("build requires exactly one JSON request file".to_string());
            }
            Ok(CommandMode::Build(BuildCliCommand {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                request_path: PathBuf::from(&positionals[0]),
                json: has_bool_flag(&flags, "json"),
            }))
        }
        "help" => Ok(CommandMode::Help),
        other if other.starts_with("--") => {
            let (flags, positionals) = parse_option_flags(
                &args,
                &[
                    "config",
                    "host",
                    "port",
                    "public-base-url",
                    "storage",
                    "max-source-bytes",
                    "build-timeout-seconds",
                    "max-build-concurrency",
                    "auth-token",
                    "health-cache-ttl-seconds",
                    "cherri-bin",
                    "shortcuts-bin",
                ],
                &[],
            )?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            let file_config = load_config_for_flags(&flags)?;
            Ok(CommandMode::Serve(build_runtime_config(
                &flags,
                &file_config,
                true,
            )?))
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn parse_option_flags(
    args: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> Result<(HashMap<String, String>, Vec<String>), String> {
    let value_flags = HashSet::<&str>::from_iter(value_flags.iter().copied());
    let bool_flags = HashSet::<&str>::from_iter(bool_flags.iter().copied());
    let mut flags = HashMap::new();
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if !arg.starts_with("--") {
            positionals.push(arg.clone());
            index += 1;
            continue;
        }
        let raw = &arg[2..];
        if raw.is_empty() {
            return Err("unexpected bare --".to_string());
        }
        if let Some((key, value)) = raw.split_once('=') {
            if value_flags.contains(key) {
                flags.insert(key.to_string(), value.to_string());
            } else if bool_flags.contains(key) {
                return Err(format!("--{key} does not take a value"));
            } else {
                return Err(format!("unknown flag: --{key}"));
            }
            index += 1;
            continue;
        }
        if value_flags.contains(raw) {
            if index + 1 >= args.len() {
                return Err(format!("missing value for --{raw}"));
            }
            flags.insert(raw.to_string(), args[index + 1].clone());
            index += 2;
            continue;
        }
        if bool_flags.contains(raw) {
            flags.insert(raw.to_string(), "true".to_string());
            index += 1;
            continue;
        }
        return Err(format!("unknown flag: --{raw}"));
    }
    Ok((flags, positionals))
}

fn load_config_for_flags(
    flags: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    let Some(path) = flags
        .get("config")
        .cloned()
        .or_else(|| env::var("SHORTCUT_SERVER_CONFIG").ok())
    else {
        return Ok(HashMap::new());
    };
    parse_config_file(Path::new(&path))
        .map_err(|err| format!("failed to load config {path}: {err}"))
}

fn config_value(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Option<String> {
    config_value_with_env(flags, file_config, flag, Some(env_name))
}

fn config_value_with_env(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: Option<&str>,
) -> Option<String> {
    flags
        .get(flag)
        .cloned()
        .or_else(|| env_name.and_then(|name| env::var(name).ok()))
        .or_else(|| file_config.get(flag).cloned())
}

fn parse_u16_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<u16>, String> {
    parse_u16_value(config_value(flags, file_config, flag, env_name), flag)
}

fn parse_u64_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<u64>, String> {
    parse_u64_value(config_value(flags, file_config, flag, env_name), flag)
}

fn parse_usize_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<usize>, String> {
    parse_usize_value(config_value(flags, file_config, flag, env_name), flag)
}

fn parse_u16_value(value: Option<String>, flag: &str) -> Result<Option<u16>, String> {
    value
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| format!("{flag} must be a u16"))
        })
        .transpose()
}

fn parse_u64_value(value: Option<String>, flag: &str) -> Result<Option<u64>, String> {
    value
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{flag} must be a number"))
        })
        .transpose()
}

fn parse_usize_value(value: Option<String>, flag: &str) -> Result<Option<usize>, String> {
    value
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{flag} must be a number"))
        })
        .transpose()
}

fn build_runtime_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    require_auth: bool,
) -> Result<Config, String> {
    let host = config_value(&flags, file_config, "host", "SHORTCUT_SERVER_HOST")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if host.trim().is_empty() {
        return Err("host must not be empty".to_string());
    }
    let port = parse_u16_config(flags, file_config, "port", "SHORTCUT_SERVER_PORT")?
        .unwrap_or(DEFAULT_PORT);
    let public_base_url = config_value(
        flags,
        file_config,
        "public-base-url",
        "SHORTCUT_SERVER_PUBLIC_BASE_URL",
    )
    .unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
    .trim_end_matches('/')
    .to_string();
    let storage = config_value(flags, file_config, "storage", "SHORTCUT_SERVER_STORAGE")
        .unwrap_or_else(|| "./data".to_string());
    let max_source_bytes = parse_usize_config(
        flags,
        file_config,
        "max-source-bytes",
        "SHORTCUT_SERVER_MAX_SOURCE_BYTES",
    )?
    .unwrap_or(DEFAULT_MAX_SOURCE_BYTES);
    let build_timeout_seconds = parse_u64_config(
        flags,
        file_config,
        "build-timeout-seconds",
        "SHORTCUT_SERVER_BUILD_TIMEOUT_SECONDS",
    )?
    .unwrap_or(DEFAULT_BUILD_TIMEOUT_SECONDS);
    let max_build_concurrency = parse_usize_config(
        flags,
        file_config,
        "max-build-concurrency",
        "SHORTCUT_SERVER_MAX_BUILD_CONCURRENCY",
    )?
    .unwrap_or(1);
    if max_build_concurrency == 0 {
        return Err("--max-build-concurrency must be at least 1".to_string());
    }
    let health_cache_seconds = parse_u64_config(
        flags,
        file_config,
        "health-cache-ttl-seconds",
        "SHORTCUT_SERVER_HEALTH_CACHE_TTL_SECONDS",
    )?
    .unwrap_or(DEFAULT_HEALTH_CACHE_SECONDS);
    let auth_token = config_value(
        flags,
        file_config,
        "auth-token",
        "SHORTCUT_SERVER_AUTH_TOKEN",
    )
    .or_else(|| env::var("SERVER_AUTH_TOKEN").ok())
    .unwrap_or_default();
    if require_auth && auth_token.is_empty() {
        return Err(
            "auth token is required: pass --auth-token, set SHORTCUT_SERVER_AUTH_TOKEN, or set auth_token in --config"
                .to_string(),
        );
    }
    if !auth_token.is_empty() && auth_token.trim().is_empty() {
        return Err("auth token must not be empty".to_string());
    }
    let cherri_bin = config_value(
        flags,
        file_config,
        "cherri-bin",
        "SHORTCUT_SERVER_CHERRI_BIN",
    )
    .unwrap_or_else(|| "cherri".to_string());
    let shortcuts_bin = config_value(
        flags,
        file_config,
        "shortcuts-bin",
        "SHORTCUT_SERVER_SHORTCUTS_BIN",
    )
    .unwrap_or_else(|| "shortcuts".to_string());
    Ok(Config {
        host,
        port,
        public_base_url,
        storage: PathBuf::from(storage),
        max_source_bytes,
        build_timeout: Duration::from_secs(build_timeout_seconds),
        max_build_concurrency,
        auth_token,
        health_cache_ttl: Duration::from_secs(health_cache_seconds),
        cherri_bin,
        shortcuts_bin,
    })
}

fn build_runtime_config_from_file(
    file_config: &HashMap<String, String>,
    require_auth: bool,
) -> Result<Config, String> {
    let flags = HashMap::new();
    let host = config_value_with_env(&flags, file_config, "host", None)
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if host.trim().is_empty() {
        return Err("host must not be empty".to_string());
    }
    let port = parse_u16_value(
        config_value_with_env(&flags, file_config, "port", None),
        "port",
    )?
    .unwrap_or(DEFAULT_PORT);
    let public_base_url = config_value_with_env(&flags, file_config, "public-base-url", None)
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
        .trim_end_matches('/')
        .to_string();
    let storage = config_value_with_env(&flags, file_config, "storage", None)
        .unwrap_or_else(|| "./data".to_string());
    let max_source_bytes = parse_usize_value(
        config_value_with_env(&flags, file_config, "max-source-bytes", None),
        "max-source-bytes",
    )?
    .unwrap_or(DEFAULT_MAX_SOURCE_BYTES);
    let build_timeout_seconds = parse_u64_value(
        config_value_with_env(&flags, file_config, "build-timeout-seconds", None),
        "build-timeout-seconds",
    )?
    .unwrap_or(DEFAULT_BUILD_TIMEOUT_SECONDS);
    let max_build_concurrency = parse_usize_value(
        config_value_with_env(&flags, file_config, "max-build-concurrency", None),
        "max-build-concurrency",
    )?
    .unwrap_or(1);
    if max_build_concurrency == 0 {
        return Err("--max-build-concurrency must be at least 1".to_string());
    }
    let health_cache_seconds = parse_u64_value(
        config_value_with_env(&flags, file_config, "health-cache-ttl-seconds", None),
        "health-cache-ttl-seconds",
    )?
    .unwrap_or(DEFAULT_HEALTH_CACHE_SECONDS);
    let auth_token =
        config_value_with_env(&flags, file_config, "auth-token", None).unwrap_or_default();
    if require_auth && auth_token.is_empty() {
        return Err("config is missing auth_token".to_string());
    }
    let cherri_bin = config_value_with_env(&flags, file_config, "cherri-bin", None)
        .unwrap_or_else(|| "cherri".to_string());
    let shortcuts_bin = config_value_with_env(&flags, file_config, "shortcuts-bin", None)
        .unwrap_or_else(|| "shortcuts".to_string());
    Ok(Config {
        host,
        port,
        public_base_url,
        storage: PathBuf::from(storage),
        max_source_bytes,
        build_timeout: Duration::from_secs(build_timeout_seconds),
        max_build_concurrency,
        auth_token,
        health_cache_ttl: Duration::from_secs(health_cache_seconds),
        cherri_bin,
        shortcuts_bin,
    })
}

fn parse_operator_command(args: &[String]) -> Result<OperatorCommand, String> {
    let (flags, positionals) = parse_option_flags(args, &["config"], &[])?;
    if let Some(arg) = positionals.first() {
        return Err(format!("unexpected argument: {arg}"));
    }
    Ok(OperatorCommand {
        config_path: operator_config_path(flags.get("config").cloned(), default_config_path()?)?,
    })
}

fn parse_config_command(args: &[String]) -> Result<CommandMode, String> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Err("config requires a subcommand: show or set".to_string());
    };
    match subcommand {
        "show" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &[])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            Ok(CommandMode::ConfigShow(ConfigShowCommand {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
            }))
        }
        "set" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &[])?;
            if positionals.len() != 2 {
                return Err("config set requires <key> <value>".to_string());
            }
            Ok(CommandMode::ConfigSet(ConfigSetCommand {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                key: normalize_config_key(&positionals[0]),
                value: positionals[1].clone(),
            }))
        }
        _ => Err(format!("unknown config subcommand: {subcommand}")),
    }
}

fn parse_token_command(args: &[String]) -> Result<CommandMode, String> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Err("token requires a subcommand: rotate".to_string());
    };
    match subcommand {
        "rotate" => {
            let (flags, positionals) = parse_option_flags(&args[1..], &["config"], &["print"])?;
            if let Some(arg) = positionals.first() {
                return Err(format!("unexpected argument: {arg}"));
            }
            Ok(CommandMode::TokenRotate(TokenRotateCommand {
                config_path: operator_config_path(
                    flags.get("config").cloned(),
                    default_config_path()?,
                )?,
                print: has_bool_flag(&flags, "print"),
            }))
        }
        _ => Err(format!("unknown token subcommand: {subcommand}")),
    }
}

fn has_bool_flag(flags: &HashMap<String, String>, key: &str) -> bool {
    flags.contains_key(key)
}

fn operator_config_path(explicit: Option<String>, default: PathBuf) -> Result<PathBuf, String> {
    Ok(explicit
        .map(PathBuf::from)
        .or_else(|| env::var("SHORTCUT_SERVER_CONFIG").ok().map(PathBuf::from))
        .unwrap_or(default))
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())
}

fn app_support_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join("ShortcutForge"))
}

fn default_storage_dir() -> Result<PathBuf, String> {
    Ok(app_support_dir()?.join("data"))
}

fn default_config_path() -> Result<PathBuf, String> {
    Ok(app_support_dir()?.join("shortcut-forge.conf"))
}

fn default_log_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Logs")
        .join("ShortcutForge"))
}

fn default_launch_agent_path() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

fn normalize_config_key(key: &str) -> String {
    key.trim().replace('_', "-")
}

fn parse_config_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut values = HashMap::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = strip_config_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {} must use key = value", index + 1))?;
        let key = raw_key.trim().replace('_', "-");
        if !is_known_config_key(&key) {
            return Err(format!("line {} has unknown key: {key}", index + 1));
        }
        let value = parse_config_value(raw_value.trim())
            .map_err(|err| format!("line {}: {err}", index + 1))?;
        values.insert(key, value);
    }
    Ok(values)
}

fn is_known_config_key(key: &str) -> bool {
    matches!(
        key,
        "host"
            | "port"
            | "public-base-url"
            | "storage"
            | "max-source-bytes"
            | "build-timeout-seconds"
            | "max-build-concurrency"
            | "auth-token"
            | "health-cache-ttl-seconds"
            | "cherri-bin"
            | "shortcuts-bin"
            | "expired-before"
    )
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

fn parse_config_value(raw: &str) -> Result<String, String> {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        if (bytes[0] == b'"' && bytes[raw.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[raw.len() - 1] == b'\'')
        {
            let inner = &raw[1..raw.len() - 1];
            if bytes[0] == b'\'' {
                return Ok(inner.to_string());
            }
            return unescape_double_quoted_config_value(inner);
        }
    }
    if raw.starts_with('"') || raw.starts_with('\'') {
        return Err("unterminated quoted value".to_string());
    }
    Ok(raw.trim().to_string())
}

fn unescape_double_quoted_config_value(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| "trailing escape in quoted value".to_string())?;
        match escaped {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            other => return Err(format!("unsupported escape: \\{other}")),
        }
    }
    Ok(out)
}

fn parse_age(value: &str) -> Result<Duration, String> {
    if value == "now" {
        return Ok(Duration::from_secs(0));
    }
    let (digits, unit) = value.trim().split_at(
        value
            .trim()
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(value.trim().len()),
    );
    if digits.is_empty() {
        return Err("age must look like 30d, 12h, 60m, or 10s".to_string());
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "age amount must be numeric".to_string())?;
    let seconds = match unit {
        "d" => amount.saturating_mul(86_400),
        "h" => amount.saturating_mul(3_600),
        "m" => amount.saturating_mul(60),
        "s" | "" => amount,
        _ => return Err("age unit must be d, h, m, or s".to_string()),
    };
    Ok(Duration::from_secs(seconds))
}

fn print_help() {
    println!(
        "Shortcut Forge {VERSION}

USAGE:
  shortcut-forge init [options]
  shortcut-forge start [--config <file>]
  shortcut-forge status [--config <file>] [--json]
  shortcut-forge smoke [--config <file>] [--request <json>] [--output <file>]
  shortcut-forge build <json-file> [--config <file>] [--json]
  shortcut-forge doctor [--config <file>] [--json]
  shortcut-forge logs [--config <file>] [--lines <n>] [--follow]
  shortcut-forge config show [--config <file>]
  shortcut-forge config set [--config <file>] <key> <value>
  shortcut-forge token rotate [--config <file>] [--print]
  shortcut-forge serve [options]
  shortcut-forge gc [--config <file>] [--storage ./data] [--expired-before 30d]

COMMANDS:
  init                                   create config, logs, and LaunchAgent files
  doctor                                 run local environment and deployment checks
  start                                  bootstrap or kickstart the LaunchAgent
  stop                                   boot out the LaunchAgent
  restart                                restart the LaunchAgent
  status                                 show service, config, and health status
  logs                                   tail stdout/stderr logs
  config show                            print effective config with secrets redacted
  config set                             safely update a config value
  token rotate                           rotate the service auth token
  smoke                                  submit the sample build and download it locally
  build                                  submit a JSON build request to the running server
  serve                                  start the low-level HTTP build/sign server
  gc                                     remove expired local build artifacts

COMMON OPTIONS:
  --config <file>                       flat key = value config file

INIT OPTIONS:
  --host <host>                         default 0.0.0.0
  --port <port>                         default 8787
  --public-base-url <url>               default http://<hostname>.local:<port>
  --storage <dir>                       default ~/Library/Application Support/ShortcutForge/data
  --non-interactive                     fail instead of prompting
  --yes                                 accept defaults and overwrite generated files where safe

SERVE OPTIONS:
  --host <host>                         default 127.0.0.1
  --port <port>                         default 8787
  --public-base-url <url>               default http://127.0.0.1:<port>
  --storage <dir>                       default ./data
  --max-source-bytes <bytes>            default 524288
  --build-timeout-seconds <seconds>     default 30
  --max-build-concurrency <n>           default 1
  --auth-token <token>                  required
  --health-cache-ttl-seconds <seconds>  default 60

GC OPTIONS:
  --storage <dir>                       default ./data
  --expired-before <age>                example: 30d, 12h, 60m, now

LOG OPTIONS:
  --lines <n>                           default 80
  --follow                              stream appended log lines

Precedence: CLI flags > SHORTCUT_SERVER_* environment variables > config file > defaults.
SERVER_AUTH_TOKEN is also accepted for local smoke-test compatibility."
    );
}

struct AppState {
    config: Config,
    build_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    build_slots: Mutex<usize>,
    health_cache: Mutex<Option<CachedToolchain>>,
    _storage_lock: StorageLock,
}

struct CachedToolchain {
    probed_at: Instant,
    toolchain: Toolchain,
}

struct StorageLock {
    #[allow(dead_code)]
    file: File,
}

fn serve(config: Config) -> io::Result<()> {
    fs::create_dir_all(&config.storage)?;
    let storage_lock = StorageLock::acquire(&config.storage)?;
    let state = Arc::new(AppState {
        config: config.clone(),
        build_locks: Mutex::new(HashMap::new()),
        build_slots: Mutex::new(0),
        health_cache: Mutex::new(None),
        _storage_lock: storage_lock,
    });

    let bind_addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&bind_addr)?;
    eprintln!(
        "listening on http://{} with public_base_url={} storage={}",
        bind_addr,
        config.public_base_url,
        config.storage.display()
    );

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(err) = handle_connection(stream, state) {
                        eprintln!("request failed: {err}");
                    }
                });
            }
            Err(err) => eprintln!("accept failed: {err}"),
        }
    }
    Ok(())
}

impl StorageLock {
    fn acquire(storage: &Path) -> io::Result<Self> {
        fs::create_dir_all(storage)?;
        let lock_path = storage.join(".lock");
        let file = OpenOptions::new()
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

#[derive(Debug, Clone)]
struct LaunchAgentStatus {
    loaded: bool,
    running: bool,
    pid: Option<u32>,
    state: String,
    #[allow(dead_code)]
    raw: String,
}

#[derive(Debug, Clone)]
struct HealthProbe {
    ok: bool,
    status: String,
    cherri: Option<String>,
    shortcuts_sign: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Clone)]
struct DoctorCheck {
    name: &'static str,
    ok: bool,
    detail: String,
    fix: Option<String>,
}

#[derive(Debug)]
struct CurlResponse {
    status: u16,
    body: String,
}

#[derive(Debug)]
struct BuildApiResult {
    id: String,
    download_url: String,
    expires_at: String,
}

fn load_config_map_from_path(path: &Path) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Err(format!("config file not found: {}", path.display()));
    }
    parse_config_file(path)
        .map_err(|err| format!("failed to load config {}: {err}", path.display()))
}

fn load_runtime_config_from_path(path: &Path, require_auth: bool) -> Result<Config, String> {
    let file_config = load_config_map_from_path(path)?;
    build_runtime_config_from_file(&file_config, require_auth)
}

fn current_uid_string() -> Result<String, String> {
    env::var("UID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| probe_command_output("id", &["-u"]))
        .ok_or_else(|| "failed to determine current user id".to_string())
}

fn launchctl_service_target() -> Result<String, String> {
    Ok(format!(
        "gui/{}/{}",
        current_uid_string()?,
        LAUNCH_AGENT_LABEL
    ))
}

fn local_service_base_url(config: &Config) -> String {
    let host = match config.host.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    format!("http://{host}:{}", config.port)
}

fn suggest_public_base_url(port: u16) -> String {
    detect_local_hostname()
        .map(|hostname| format!("http://{hostname}.local:{port}"))
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
}

fn detect_local_hostname() -> Option<String> {
    let raw = probe_command_output("scutil", &["--get", "LocalHostName"])
        .or_else(|| probe_command_output("hostname", &[]))?;
    let short = raw.trim().split('.').next().unwrap_or("").trim();
    let sanitized = short
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn resolve_command_path(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.exists().then(|| candidate.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|path| path.exists())
}

fn generate_service_auth_token() -> io::Result<String> {
    random_bytes(32).map(|bytes| base64url_no_pad(&bytes))
}

fn render_operator_config(config: &Config, expired_before: &str) -> String {
    format!(
        r#"# Shortcut Forge config file.
# Format: flat key = value. Quotes are recommended for strings.

host = {}
port = {}
public_base_url = {}
storage = {}

max_source_bytes = {}
build_timeout_seconds = {}
max_build_concurrency = {}
health_cache_ttl_seconds = {}

auth_token = {}
cherri_bin = {}
shortcuts_bin = {}

# Used by `shortcut-forge gc` when --expired-before is omitted.
expired_before = {}
"#,
        format_config_value_for_key("host", &config.host),
        format_config_value_for_key("port", &config.port.to_string()),
        format_config_value_for_key("public-base-url", &config.public_base_url),
        format_config_value_for_key("storage", &config.storage.display().to_string()),
        format_config_value_for_key("max-source-bytes", &config.max_source_bytes.to_string()),
        format_config_value_for_key(
            "build-timeout-seconds",
            &config.build_timeout.as_secs().to_string()
        ),
        format_config_value_for_key(
            "max-build-concurrency",
            &config.max_build_concurrency.to_string()
        ),
        format_config_value_for_key(
            "health-cache-ttl-seconds",
            &config.health_cache_ttl.as_secs().to_string()
        ),
        format_config_value_for_key("auth-token", &config.auth_token),
        format_config_value_for_key("cherri-bin", &config.cherri_bin),
        format_config_value_for_key("shortcuts-bin", &config.shortcuts_bin),
        format_config_value_for_key("expired-before", expired_before),
    )
}

fn render_launch_agent_plist(
    binary_path: &Path,
    config_path: &Path,
    working_dir: &Path,
    log_dir: &Path,
) -> String {
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

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn config_key_for_file(key: &str) -> String {
    key.replace('-', "_")
}

fn format_config_value_for_key(key: &str, value: &str) -> String {
    if matches!(
        key,
        "port"
            | "max-source-bytes"
            | "build-timeout-seconds"
            | "max-build-concurrency"
            | "health-cache-ttl-seconds"
    ) {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn validate_config_assignment(key: &str, value: &str) -> Result<(), String> {
    match key {
        "host" | "storage" | "cherri-bin" | "shortcuts-bin" => {
            if value.trim().is_empty() {
                return Err(format!("{key} must not be empty"));
            }
        }
        "port" => {
            let port = value
                .parse::<u16>()
                .map_err(|_| "port must be a u16".to_string())?;
            if port == 0 {
                return Err("port must be greater than zero".to_string());
            }
        }
        "public-base-url" => validate_httpish_url(value)?,
        "max-source-bytes" => {
            let size = value
                .parse::<usize>()
                .map_err(|_| "max-source-bytes must be a number".to_string())?;
            if size == 0 {
                return Err("max-source-bytes must be at least 1".to_string());
            }
        }
        "build-timeout-seconds" => {
            let secs = value
                .parse::<u64>()
                .map_err(|_| "build-timeout-seconds must be a number".to_string())?;
            if secs == 0 {
                return Err("build-timeout-seconds must be at least 1".to_string());
            }
        }
        "max-build-concurrency" => {
            let count = value
                .parse::<usize>()
                .map_err(|_| "max-build-concurrency must be a number".to_string())?;
            if count == 0 {
                return Err("max-build-concurrency must be at least 1".to_string());
            }
        }
        "health-cache-ttl-seconds" => {
            value
                .parse::<u64>()
                .map_err(|_| "health-cache-ttl-seconds must be a number".to_string())?;
        }
        "expired-before" => {
            parse_age(value)?;
        }
        "auth-token" => {
            if value.trim().is_empty() {
                return Err("auth-token must not be empty".to_string());
            }
        }
        _ => return Err(format!("unsupported config key: {key}")),
    }
    Ok(())
}

fn validate_httpish_url(url: &str) -> Result<(), String> {
    if let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    {
        if rest.trim().is_empty() {
            return Err("public-base-url must include a host".to_string());
        }
        if rest.starts_with('/') {
            return Err("public-base-url must include a host".to_string());
        }
        Ok(())
    } else {
        Err("public-base-url must start with http:// or https://".to_string())
    }
}

fn redacted_effective_config(config: &Config, expired_before: Option<&str>) -> String {
    let mut lines = vec![
        format!(
            "host = {}",
            format_config_value_for_key("host", &config.host)
        ),
        format!("port = {}", config.port),
        format!(
            "public_base_url = {}",
            format_config_value_for_key("public-base-url", &config.public_base_url)
        ),
        format!(
            "storage = {}",
            format_config_value_for_key("storage", &config.storage.display().to_string())
        ),
        format!("max_source_bytes = {}", config.max_source_bytes),
        format!("build_timeout_seconds = {}", config.build_timeout.as_secs()),
        format!("max_build_concurrency = {}", config.max_build_concurrency),
        format!(
            "health_cache_ttl_seconds = {}",
            config.health_cache_ttl.as_secs()
        ),
        r#"auth_token = "[redacted]""#.to_string(),
        format!(
            "cherri_bin = {}",
            format_config_value_for_key("cherri-bin", &config.cherri_bin)
        ),
        format!(
            "shortcuts_bin = {}",
            format_config_value_for_key("shortcuts-bin", &config.shortcuts_bin)
        ),
    ];
    if let Some(expired_before) = expired_before {
        lines.push(format!(
            "expired_before = {}",
            format_config_value_for_key("expired-before", expired_before)
        ));
    }
    lines.join("\n")
}

fn atomic_write_restricted_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
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
    sync_parent_dir(path).map_err(|err| format!("failed to sync {}: {err}", path.display()))?;
    Ok(())
}

fn launch_agent_status() -> Result<LaunchAgentStatus, String> {
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

fn ensure_launch_agent_file(config_path: &Path) -> Result<PathBuf, String> {
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

fn curl_json_request(
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

fn curl_download(url: &str, output_path: &Path) -> Result<u16, String> {
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

fn probe_local_health(config: &Config) -> Result<HealthProbe, String> {
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
    let body = JsonParser::new(response.body.as_bytes())
        .parse()
        .map_err(|err| format!("invalid /health response: {err}"))?;
    let object = body
        .as_object()
        .ok_or_else(|| "health response must be a JSON object".to_string())?;
    let data = object
        .get("data")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "health response missing data".to_string())?;
    Ok(HealthProbe {
        ok: true,
        status: data
            .get("status")
            .and_then(JsonValue::as_string)
            .unwrap_or("ok")
            .to_string(),
        cherri: data
            .get("cherri")
            .and_then(JsonValue::as_string)
            .map(ToString::to_string),
        shortcuts_sign: data
            .get("shortcuts_sign")
            .and_then(JsonValue::as_string)
            .map(ToString::to_string),
        detail: None,
    })
}

fn api_message_from_json(body: &str) -> Option<String> {
    let value = JsonParser::new(body.as_bytes()).parse().ok()?;
    let object = value.as_object()?;
    object
        .get("error")
        .and_then(JsonValue::as_object)
        .and_then(|error| error.get("message"))
        .and_then(JsonValue::as_string)
        .map(ToString::to_string)
}

fn parse_build_api_response(body: &str) -> Result<BuildApiResult, String> {
    let value = JsonParser::new(body.as_bytes())
        .parse()
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

fn tail_file_lines(path: &Path, lines: usize) -> Vec<String> {
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

fn find_smoke_request_path(explicit: Option<&Path>) -> Result<PathBuf, String> {
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

fn extract_url_path(url: &str) -> Result<&str, String> {
    let scheme_end = url
        .find("://")
        .ok_or_else(|| "URL must include a scheme".to_string())?;
    let rest = &url[scheme_end + 3..];
    let slash = rest
        .find('/')
        .ok_or_else(|| "URL must include a path".to_string())?;
    Ok(&rest[slash..])
}

fn extract_url_host(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| "URL must start with http:// or https://".to_string())?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err("URL must include a host".to_string());
    }
    if let Some(host) = authority.strip_prefix('[') {
        return host
            .split(']')
            .next()
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| "URL host is invalid".to_string());
    }
    Ok(authority.split(':').next().unwrap_or("").to_string())
}

fn localize_service_url(url: &str, config: &Config) -> Result<String, String> {
    Ok(format!(
        "{}{}",
        local_service_base_url(config),
        extract_url_path(url)?
    ))
}

fn ensure_writable_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    let probe = path.join(format!(".write-test-{}", std::process::id()));
    fs::write(&probe, b"ok")
        .map_err(|err| format!("failed to write {}: {err}", probe.display()))?;
    fs::remove_file(&probe)
        .map_err(|err| format!("failed to remove {}: {err}", probe.display()))?;
    Ok(())
}

fn update_config_file_value(path: &Path, key: &str, value: &str) -> Result<(), String> {
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

fn is_supported_config_set_key(key: &str) -> bool {
    matches!(
        key,
        "host"
            | "port"
            | "public-base-url"
            | "storage"
            | "max-source-bytes"
            | "build-timeout-seconds"
            | "max-build-concurrency"
            | "health-cache-ttl-seconds"
            | "cherri-bin"
            | "shortcuts-bin"
            | "expired-before"
    )
}

fn should_restart_for_config_key(key: &str) -> bool {
    key != "expired-before"
}

fn log_output_paths() -> Result<(PathBuf, PathBuf), String> {
    let dir = default_log_dir()?;
    Ok((dir.join("stdout.log"), dir.join("stderr.log")))
}

fn ensure_log_files() -> Result<(PathBuf, PathBuf), String> {
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

fn prompt_value(prompt: &str, default: &str) -> Result<String, String> {
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

fn prompt_confirm(prompt: &str, default: bool) -> Result<bool, String> {
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

fn run_launchctl_command(args: &[&str]) -> Result<String, String> {
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

struct ColorCodes {
    bold: &'static str,
    dim: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    reset: &'static str,
}

fn run_init(command: &InitConfig) -> Result<(), String> {
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
        public_base_url: public_base_url.trim_end_matches('/').to_string(),
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
    set_private_dir(&app_support)
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

fn run_doctor(command: &DoctorConfig) -> Result<bool, String> {
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

fn run_start(command: &OperatorCommand) -> Result<(), String> {
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

fn run_stop(command: &OperatorCommand) -> Result<(), String> {
    let _ = command;
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

fn run_restart(command: &OperatorCommand) -> Result<(), String> {
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

fn run_status(command: &StatusConfig) -> Result<(), String> {
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

fn run_logs(command: &LogsConfig) -> Result<(), String> {
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

fn run_config_show(command: &ConfigShowCommand) -> Result<(), String> {
    let raw = load_config_map_from_path(&command.config_path)?;
    let config = build_runtime_config_from_file(&raw, false)?;
    println!(
        "{}",
        redacted_effective_config(&config, raw.get("expired-before").map(String::as_str))
    );
    Ok(())
}

fn run_config_set(command: &ConfigSetCommand) -> Result<(), String> {
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

fn run_token_rotate(command: &TokenRotateCommand) -> Result<(), String> {
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

fn run_smoke(command: &SmokeCommand) -> Result<(), String> {
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

fn run_build_command(command: &BuildCliCommand) -> Result<(), String> {
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

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

fn handle_connection(mut stream: TcpStream, state: Arc<AppState>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    let (header_bytes, leftover) = match read_http_headers(&mut stream) {
        Ok(parts) => parts,
        Err(err) => {
            write_response(&mut stream, err.into_response())?;
            return Ok(());
        }
    };
    let request = match HttpRequestHead::parse(&header_bytes) {
        Ok(request) => request,
        Err(err) => {
            write_response(&mut stream, err.into_response())?;
            return Ok(());
        }
    };
    let path = request.path_without_query();
    let route = route_pattern(&request.method, path);

    let response = match (&request.method[..], path) {
        ("GET", "/health") => handle_health(&request, &state),
        ("POST", "/api/builds") => {
            if !is_authorized(&request, &state.config.auth_token) {
                let response = api_error("UNAUTHORIZED", 401, "missing or invalid bearer token")
                    .into_response();
                let status = response.status;
                write_response(&mut stream, response)?;
                let _ = drain_small_body_after_rejection(&mut stream, &request, leftover);
                eprintln!(
                    "request method={} route={} status={}",
                    request.method, route, status
                );
                return Ok(());
            } else {
                match read_request_body(&mut stream, &request, leftover, body_limit(&state.config))
                {
                    Ok(body) => handle_post_build(&body, &state),
                    Err(err) => err.into_response(),
                }
            }
        }
        ("GET", path) if path.starts_with("/api/builds/") => {
            if !is_authorized(&request, &state.config.auth_token) {
                api_error("UNAUTHORIZED", 401, "missing or invalid bearer token").into_response()
            } else {
                let id = path.trim_start_matches("/api/builds/");
                handle_get_build(id, &state)
            }
        }
        ("GET", path) if path.starts_with("/s/") => {
            let token = path.trim_start_matches("/s/");
            handle_download(token, &state)
        }
        _ => api_error("NOT_FOUND", 404, "not found").into_response(),
    };

    eprintln!(
        "request method={} route={} status={}",
        request.method, route, response.status
    );
    write_response(&mut stream, response)?;
    Ok(())
}

fn body_limit(config: &Config) -> usize {
    config
        .max_source_bytes
        .saturating_mul(4)
        .saturating_add(64 * 1024)
}

fn read_http_headers(stream: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    read_http_headers_from(stream)
}

fn read_http_headers_from<R: Read>(reader: &mut R) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let count = reader
            .read(&mut byte)
            .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to read request"))?;
        if count == 0 {
            return Err(api_error("VALIDATION_FAILED", 400, "empty request"));
        }
        buffer.push(byte[0]);
        if buffer.len() > HEADER_LIMIT {
            return Err(api_error(
                "PAYLOAD_TOO_LARGE",
                413,
                "request headers exceed configured limit",
            ));
        }
        if let Some(end) = find_header_end(&buffer) {
            buffer.truncate(end);
            return Ok((buffer, Vec::new()));
        }
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|pos| pos + 2)
        })
}

struct HttpRequestHead {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
}

impl HttpRequestHead {
    fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| api_error("VALIDATION_FAILED", 400, "request headers must be UTF-8"))?;
        let mut lines = text.lines();
        let request_line = lines
            .next()
            .ok_or_else(|| api_error("VALIDATION_FAILED", 400, "missing request line"))?;
        let mut parts = request_line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| api_error("VALIDATION_FAILED", 400, "missing method"))?
            .to_string();
        let target = parts
            .next()
            .ok_or_else(|| api_error("VALIDATION_FAILED", 400, "missing path"))?
            .to_string();
        let mut headers = BTreeMap::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        Ok(Self {
            method,
            target,
            headers,
        })
    }

    fn path_without_query(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

fn read_request_body(
    stream: &mut TcpStream,
    request: &HttpRequestHead,
    mut leftover: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, ApiError> {
    let length = request
        .header("content-length")
        .ok_or_else(|| api_error("VALIDATION_FAILED", 400, "content-length is required"))?
        .parse::<usize>()
        .map_err(|_| api_error("VALIDATION_FAILED", 400, "content-length must be numeric"))?;
    if length > limit {
        return Err(api_error(
            "PAYLOAD_TOO_LARGE",
            413,
            "request body exceeds configured limit",
        ));
    }
    if leftover.len() > length {
        leftover.truncate(length);
    }
    while leftover.len() < length {
        let mut chunk = vec![0u8; length - leftover.len()];
        let count = stream
            .read(&mut chunk)
            .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to read request body"))?;
        if count == 0 {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "request body ended early",
            ));
        }
        leftover.extend_from_slice(&chunk[..count]);
    }
    Ok(leftover)
}

fn drain_small_body_after_rejection(
    stream: &mut TcpStream,
    request: &HttpRequestHead,
    mut leftover: Vec<u8>,
) -> io::Result<()> {
    const MAX_REJECTED_BODY_DRAIN: usize = 64 * 1024;
    let Some(length) = request
        .header("content-length")
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return Ok(());
    };
    if length > MAX_REJECTED_BODY_DRAIN {
        return Ok(());
    }
    if leftover.len() > length {
        leftover.truncate(length);
    }
    let mut remaining = length.saturating_sub(leftover.len());
    let mut buffer = [0u8; 4096];
    while remaining > 0 {
        let read_len = remaining.min(buffer.len());
        let count = stream.read(&mut buffer[..read_len])?;
        if count == 0 {
            break;
        }
        remaining -= count;
    }
    Ok(())
}

fn is_authorized(request: &HttpRequestHead, expected: &str) -> bool {
    let Some(header) = request.header("authorization") else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max_len {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        diff |= (left ^ right) as usize;
    }
    diff == 0
}

fn route_pattern(method: &str, path: &str) -> &'static str {
    match (method, path) {
        ("GET", "/health") => "/health",
        ("POST", "/api/builds") => "/api/builds",
        _ if path.starts_with("/api/builds/") => "/api/builds/:id",
        _ if path.starts_with("/s/") => "/s/:download_token",
        _ => "unmatched",
    }
}

fn handle_health(request: &HttpRequestHead, state: &AppState) -> HttpResponse {
    if request.header("authorization").is_none() {
        return json_response(
            200,
            format!(
                r#"{{"ok":true,"data":{{"version":"{}","status":"ok","auth_required":true}}}}"#,
                json_escape(VERSION)
            ),
        );
    }
    if !is_authorized(request, &state.config.auth_token) {
        return api_error("UNAUTHORIZED", 401, "missing or invalid bearer token").into_response();
    }
    let toolchain = get_cached_toolchain(state);
    json_response(
        200,
        format!(
            r#"{{"ok":true,"data":{{"version":"{}","status":"ok","auth_required":true,"cherri":"{}","shortcuts_sign":"{}","cache_ttl_seconds":{}}}}}"#,
            json_escape(VERSION),
            json_escape(&toolchain.cherri),
            json_escape(&toolchain.shortcuts_sign),
            state.config.health_cache_ttl.as_secs()
        ),
    )
}

fn get_cached_toolchain(state: &AppState) -> Toolchain {
    let mut cache = state.health_cache.lock().expect("health cache poisoned");
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

fn handle_post_build(body: &[u8], state: &AppState) -> HttpResponse {
    let request = match BuildRequest::parse(body, state.config.max_source_bytes) {
        Ok(request) => request,
        Err(err) => return err.into_response(),
    };
    match build_or_renew(request, state) {
        Ok(response) => json_response(
            200,
            format!(
                r#"{{"id":"{}","download_url":"{}","expires_at":"{}"}}"#,
                json_escape(&response.id),
                json_escape(&response.download_url),
                json_escape(&format_rfc3339(response.expires_at))
            ),
        ),
        Err(err) => err.into_response(),
    }
}

fn handle_get_build(id: &str, state: &AppState) -> HttpResponse {
    if !is_valid_build_id(id) {
        return api_error("NOT_FOUND", 404, "not found").into_response();
    }
    match load_metadata(&state.config.storage, id) {
        Ok(Some(metadata)) => json_response(200, metadata.to_api_json(now_unix())),
        Ok(None) => api_error("NOT_FOUND", 404, "not found").into_response(),
        Err(_) => api_error("INTERNAL_ERROR", 500, "failed to read metadata").into_response(),
    }
}

fn handle_download(token: &str, state: &AppState) -> HttpResponse {
    if !is_valid_download_token(token) {
        return api_error("NOT_FOUND", 404, "not found").into_response();
    }
    let token_hash = sha256_hex(token.as_bytes());
    match resolve_download(&state.config.storage, &token_hash, now_unix()) {
        Ok(Some(download)) => match fs::read(&download.artifact_path) {
            Ok(bytes) => {
                let filename = format!("{}.shortcut", safe_filename(&download.name));
                binary_response(
                    200,
                    bytes,
                    vec![
                        (
                            "Content-Type".to_string(),
                            "application/octet-stream".to_string(),
                        ),
                        (
                            "Content-Disposition".to_string(),
                            format!(r#"attachment; filename="{filename}""#),
                        ),
                    ],
                )
            }
            Err(_) => api_error("NOT_FOUND", 404, "not found").into_response(),
        },
        Ok(None) => api_error("NOT_FOUND", 404, "not found").into_response(),
        Err(_) => api_error("INTERNAL_ERROR", 500, "failed to read metadata").into_response(),
    }
}

#[derive(Debug)]
struct BuildRequest {
    name: String,
    source_format: String,
    source: String,
    sign_mode: String,
    ttl_seconds: u64,
}

impl BuildRequest {
    fn parse(body: &[u8], max_source_bytes: usize) -> Result<Self, ApiError> {
        let value = JsonParser::new(body)
            .parse()
            .map_err(|err| api_error("VALIDATION_FAILED", 400, &err))?;
        let object = value.as_object().ok_or_else(|| {
            api_error(
                "VALIDATION_FAILED",
                400,
                "request body must be a JSON object",
            )
        })?;
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "name" | "source_format" | "source" | "sign_mode" | "ttl_seconds"
            ) {
                return Err(api_error("VALIDATION_FAILED", 400, "unknown request field"));
            }
        }
        let name = required_string(object, "name")?.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "name must be 1-80 characters",
            ));
        }
        let source_format = required_string(object, "source_format")?;
        if source_format != "cherri" {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "source_format must be cherri",
            ));
        }
        let source = required_string(object, "source")?;
        if source.is_empty() {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "source must be non-empty",
            ));
        }
        if source.len() > max_source_bytes {
            return Err(api_error(
                "PAYLOAD_TOO_LARGE",
                413,
                "source exceeds configured limit",
            ));
        }
        let sign_mode =
            optional_string(object, "sign_mode")?.unwrap_or_else(|| "anyone".to_string());
        if sign_mode != "anyone" {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "sign_mode must be anyone",
            ));
        }
        let ttl_seconds = optional_i64(object, "ttl_seconds")?
            .map(|value| if value < 0 { 0 } else { value as u64 })
            .unwrap_or(DEFAULT_TTL_SECONDS);
        if !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&ttl_seconds) {
            return Err(api_error(
                "VALIDATION_FAILED",
                400,
                "ttl_seconds must be between 60 and 2592000",
            ));
        }
        Ok(Self {
            name,
            source_format,
            source,
            sign_mode,
            ttl_seconds,
        })
    }
}

fn required_string(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<String, ApiError> {
    object
        .get(key)
        .and_then(JsonValue::as_string)
        .map(ToString::to_string)
        .ok_or_else(|| api_error("VALIDATION_FAILED", 400, &format!("{key} is required")))
}

fn optional_string(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, ApiError> {
    match object.get(key) {
        Some(value) => value
            .as_string()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| api_error("VALIDATION_FAILED", 400, &format!("{key} must be a string"))),
        None => Ok(None),
    }
}

fn optional_i64(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<Option<i64>, ApiError> {
    match object.get(key) {
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            api_error(
                "VALIDATION_FAILED",
                400,
                &format!("{key} must be an integer"),
            )
        }),
        None => Ok(None),
    }
}

struct BuildResponse {
    id: String,
    download_url: String,
    expires_at: i64,
}

fn build_or_renew(request: BuildRequest, state: &AppState) -> Result<BuildResponse, ApiError> {
    let fingerprint_input = format!(
        "{}\n{}\n{}",
        request.source_format, request.sign_mode, request.source
    );
    let source_hash = sha256_hex(fingerprint_input.as_bytes());
    let id = source_hash[..BUILD_ID_LEN].to_string();

    let per_id_lock = {
        let mut locks = state.build_locks.lock().expect("build lock table poisoned");
        locks
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = per_id_lock.lock().expect("per-build lock poisoned");

    let now = now_unix();
    let expires_at = now.saturating_add(request.ttl_seconds as i64);
    let mut existing = load_metadata(&state.config.storage, &id)
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to read metadata"))?;
    if let Some(metadata) = existing.as_ref()
        && metadata.source_hash != source_hash
    {
        return Err(api_error(
            "INTERNAL_ERROR",
            500,
            "truncated build id collision detected",
        ));
    }

    let toolchain = probe_toolchain(&state.config);
    if !toolchain.is_available() {
        return Err(api_error(
            "TOOL_UNAVAILABLE",
            503,
            "required external tool is unavailable",
        ));
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
        match run_build_pipeline(&request, &id, &state.config) {
            Ok(signed_path) => {
                persist_artifact(&state.config.storage, &id, &signed_path)
                    .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to persist artifact"))?;
                let _ = fs::remove_file(&signed_path);
                let token = generate_download_token()
                    .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to generate token"))?;
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
                    .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to persist metadata"))?;
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
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to generate token"))?;
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
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to persist metadata"))?;
    Ok(BuildResponse {
        id,
        download_url: format!("{}/s/{}", state.config.public_base_url, token),
        expires_at,
    })
}

struct BuildSlot<'a> {
    state: &'a AppState,
}

impl<'a> BuildSlot<'a> {
    fn try_acquire(state: &'a AppState) -> Result<Self, ApiError> {
        let mut count = state.build_slots.lock().expect("build semaphore poisoned");
        if *count >= state.config.max_build_concurrency {
            return Err(api_error(
                "SERVER_BUSY",
                503,
                "build concurrency limit reached",
            ));
        }
        *count += 1;
        Ok(Self { state })
    }
}

impl Drop for BuildSlot<'_> {
    fn drop(&mut self) {
        let mut count = self
            .state
            .build_slots
            .lock()
            .expect("build semaphore poisoned");
        *count = count.saturating_sub(1);
    }
}

fn run_build_pipeline(
    request: &BuildRequest,
    id: &str,
    config: &Config,
) -> Result<PathBuf, ApiError> {
    let temp_dir = create_private_temp_dir(&config.storage, "build")
        .and_then(fs::canonicalize)
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to create build directory"))?;
    let cleanup = TempDirCleanup(temp_dir.clone());
    let source_path = temp_dir.join("source.cherri");
    let mut unsigned_path = temp_dir.join("unsigned.shortcut");
    let signed_path = temp_dir.join("signed.shortcut");
    write_private_file(&source_path, request.source.as_bytes())
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to write source"))?;

    let cherri_output_arg = format!("--output={}", unsigned_path.display());
    let compile = run_command_with_timeout(
        &config.cherri_bin,
        &[
            source_path.to_string_lossy().as_ref(),
            "--skip-sign",
            &cherri_output_arg,
            "--no-ansi",
        ],
        &temp_dir,
        config.build_timeout,
        "cherri",
    )
    .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to run cherri"))?;
    if compile.timed_out {
        return Err(api_error("TIMEOUT", 504, "cherri compile timed out"));
    }
    if !compile.success {
        return Err(api_error("BUILD_FAILED", 422, "Cherri compile failed"));
    }
    if !unsigned_path.exists() {
        if let Some(discovered) = find_cherri_unsigned_output(&temp_dir, &signed_path) {
            unsigned_path = discovered;
        } else {
            return Err(api_error(
                "BUILD_FAILED",
                422,
                "Cherri did not produce shortcut output",
            ));
        }
    }

    let sign = run_command_with_timeout(
        &config.shortcuts_bin,
        &[
            "sign",
            "--mode",
            "anyone",
            "--input",
            unsigned_path.to_string_lossy().as_ref(),
            "--output",
            signed_path.to_string_lossy().as_ref(),
        ],
        &temp_dir,
        config.build_timeout,
        "shortcuts",
    )
    .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to run shortcuts sign"))?;
    if sign.timed_out {
        return Err(api_error("TIMEOUT", 504, "shortcuts sign timed out"));
    }
    if !sign.success {
        return Err(api_error("SIGN_FAILED", 422, "shortcuts sign failed"));
    }
    if !signed_path.exists() {
        return Err(api_error(
            "SIGN_FAILED",
            422,
            "shortcuts sign did not produce output",
        ));
    }

    let retained = temp_dir
        .parent()
        .unwrap_or(&temp_dir)
        .join(format!("signed-{id}-{}.shortcut", now_unix()));
    fs::create_dir_all(retained.parent().expect("retained has parent"))
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to prepare temp output"))?;
    fs::rename(&signed_path, &retained)
        .or_else(|_| fs::copy(&signed_path, &retained).map(|_| ()))
        .map_err(|_| api_error("INTERNAL_ERROR", 500, "failed to retain signed output"))?;
    let _ = fs::remove_file(&source_path);
    let _ = fs::remove_file(&unsigned_path);
    drop(cleanup);
    Ok(retained)
}

fn find_cherri_unsigned_output(temp_dir: &Path, signed_path: &Path) -> Option<PathBuf> {
    let mut matches = fs::read_dir(temp_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path != signed_path
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with("_unsigned.shortcut") || name.ends_with(".shortcut"))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    matches.sort();
    if matches.len() == 1 {
        matches.pop()
    } else {
        matches.into_iter().find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .ends_with("_unsigned.shortcut")
        })
    }
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CommandCapture {
    success: bool,
    timed_out: bool,
}

fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    work_dir: &Path,
    timeout: Duration,
    label: &str,
) -> io::Result<CommandCapture> {
    let stdout_path = work_dir.join(format!("{label}.stdout"));
    let stderr_path = work_dir.join(format!("{label}.stderr"));
    let stdout = File::create(&stdout_path)?;
    let stderr = File::create(&stderr_path)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(CommandCapture {
                success: status.success(),
                timed_out: false,
            });
        }
        if start.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                const SIGKILL: i32 = 9;
                let _ = kill(-(child.id() as i32), SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Ok(CommandCapture {
                success: false,
                timed_out: true,
            });
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[derive(Clone, Debug)]
struct Toolchain {
    cherri: String,
    shortcuts_sign: String,
    fingerprint: String,
}

impl Toolchain {
    fn is_available(&self) -> bool {
        self.cherri != "unavailable" && self.shortcuts_sign == "available"
    }
}

fn probe_toolchain(config: &Config) -> Toolchain {
    let cherri = probe_command_output(&config.cherri_bin, &["--version"])
        .unwrap_or_else(|| "unavailable".to_string());
    let shortcuts_sign = if probe_command_success(&config.shortcuts_bin, &["help", "sign"]) {
        "available".to_string()
    } else {
        "unavailable".to_string()
    };
    let fingerprint =
        sha256_hex(format!("cherri={cherri}\nshortcuts_sign={shortcuts_sign}").as_bytes());
    Toolchain {
        cherri,
        shortcuts_sign,
        fingerprint,
    }
}

fn probe_command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    Some(first_sanitized_line(&text).unwrap_or_else(|| "available".to_string()))
}

fn probe_command_success(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn first_sanitized_line(text: &str) -> Option<String> {
    text.lines()
        .map(strip_ansi_escape)
        .map(|line| line.trim().to_string())
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

fn strip_ansi_escape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BuildStatus {
    Ready,
    Failed,
}

impl BuildStatus {
    fn as_str(&self) -> &'static str {
        match self {
            BuildStatus::Ready => "ready",
            BuildStatus::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(BuildStatus::Ready),
            "failed" => Some(BuildStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct DownloadTokenRecord {
    hash: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Clone, Debug)]
struct BuildMetadata {
    id: String,
    name: String,
    source_format: String,
    source_hash: String,
    sign_mode: String,
    status: BuildStatus,
    download_tokens: Vec<DownloadTokenRecord>,
    toolchain: Toolchain,
    created_at: i64,
    updated_at: i64,
    expires_at: i64,
    error: Option<ErrorBody>,
}

impl BuildMetadata {
    fn status_for_api(&self, now: i64) -> &'static str {
        if self.status == BuildStatus::Ready && self.expires_at <= now {
            "expired"
        } else {
            self.status.as_str()
        }
    }

    fn active_download_token_count(&self, now: i64) -> usize {
        self.download_tokens
            .iter()
            .filter(|token| token.expires_at > now)
            .count()
    }

    fn to_api_json(&self, now: i64) -> String {
        let error = self.error.as_ref().map_or_else(
            || "null".to_string(),
            |error| {
                format!(
                    r#"{{"code":"{}","message":"{}"}}"#,
                    json_escape(&error.code),
                    json_escape(&error.message)
                )
            },
        );
        format!(
            r#"{{"id":"{}","name":"{}","source_format":"{}","source_hash":"{}","sign_mode":"{}","status":"{}","download_url":null,"download_token_count":{},"toolchain":{{"cherri":"{}","shortcuts_sign":"{}","fingerprint":"{}"}},"created_at":"{}","updated_at":"{}","expires_at":"{}","error":{}}}"#,
            json_escape(&self.id),
            json_escape(&self.name),
            json_escape(&self.source_format),
            json_escape(&self.source_hash),
            json_escape(&self.sign_mode),
            self.status_for_api(now),
            self.active_download_token_count(now),
            json_escape(&self.toolchain.cherri),
            json_escape(&self.toolchain.shortcuts_sign),
            json_escape(&self.toolchain.fingerprint),
            json_escape(&format_rfc3339(self.created_at)),
            json_escape(&format_rfc3339(self.updated_at)),
            json_escape(&format_rfc3339(self.expires_at)),
            error
        )
    }

    fn to_storage_json(&self) -> String {
        let tokens = self
            .download_tokens
            .iter()
            .map(|token| {
                format!(
                    r#"{{"hash":"{}","expires_at":"{}"}}"#,
                    json_escape(&token.hash),
                    json_escape(&format_rfc3339(token.expires_at))
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let error = self.error.as_ref().map_or_else(
            || "null".to_string(),
            |error| {
                format!(
                    r#"{{"code":"{}","message":"{}"}}"#,
                    json_escape(&error.code),
                    json_escape(&error.message)
                )
            },
        );
        format!(
            r#"{{
  "id": "{}",
  "name": "{}",
  "source_format": "{}",
  "source_hash": "{}",
  "sign_mode": "{}",
  "status": "{}",
  "download_tokens": [{}],
  "toolchain": {{
    "cherri": "{}",
    "shortcuts_sign": "{}",
    "fingerprint": "{}"
  }},
  "created_at": "{}",
  "updated_at": "{}",
  "expires_at": "{}",
  "error": {}
}}
"#,
            json_escape(&self.id),
            json_escape(&self.name),
            json_escape(&self.source_format),
            json_escape(&self.source_hash),
            json_escape(&self.sign_mode),
            self.status.as_str(),
            tokens,
            json_escape(&self.toolchain.cherri),
            json_escape(&self.toolchain.shortcuts_sign),
            json_escape(&self.toolchain.fingerprint),
            json_escape(&format_rfc3339(self.created_at)),
            json_escape(&format_rfc3339(self.updated_at)),
            json_escape(&format_rfc3339(self.expires_at)),
            error
        )
    }

    fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let value = JsonParser::new(bytes).parse()?;
        let object = value
            .as_object()
            .ok_or_else(|| "metadata must be an object".to_string())?;
        let id = json_required_string(object, "id")?;
        let name = json_required_string(object, "name")?;
        let source_format = json_required_string(object, "source_format")?;
        let source_hash = json_required_string(object, "source_hash")?;
        let sign_mode = json_required_string(object, "sign_mode")?;
        let status = BuildStatus::from_str(&json_required_string(object, "status")?)
            .ok_or_else(|| "metadata status is invalid".to_string())?;
        let token_values = object
            .get("download_tokens")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "download_tokens must be an array".to_string())?;
        let mut download_tokens = Vec::new();
        for token_value in token_values {
            let token = token_value
                .as_object()
                .ok_or_else(|| "download token must be an object".to_string())?;
            download_tokens.push(DownloadTokenRecord {
                hash: json_required_string(token, "hash")?,
                expires_at: parse_rfc3339(&json_required_string(token, "expires_at")?)
                    .ok_or_else(|| "download token expires_at is invalid".to_string())?,
            });
        }
        let toolchain_value = object
            .get("toolchain")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "toolchain must be an object".to_string())?;
        let toolchain = Toolchain {
            cherri: json_required_string(toolchain_value, "cherri")?,
            shortcuts_sign: json_required_string(toolchain_value, "shortcuts_sign")?,
            fingerprint: json_required_string(toolchain_value, "fingerprint")?,
        };
        let created_at = parse_rfc3339(&json_required_string(object, "created_at")?)
            .ok_or_else(|| "created_at is invalid".to_string())?;
        let updated_at = parse_rfc3339(&json_required_string(object, "updated_at")?)
            .ok_or_else(|| "updated_at is invalid".to_string())?;
        let expires_at = parse_rfc3339(&json_required_string(object, "expires_at")?)
            .ok_or_else(|| "expires_at is invalid".to_string())?;
        let error = match object.get("error") {
            Some(JsonValue::Null) | None => None,
            Some(value) => {
                let error = value
                    .as_object()
                    .ok_or_else(|| "error must be null or object".to_string())?;
                Some(ErrorBody {
                    code: json_required_string(error, "code")?,
                    message: json_required_string(error, "message")?,
                })
            }
        };
        Ok(Self {
            id,
            name,
            source_format,
            source_hash,
            sign_mode,
            status,
            download_tokens,
            toolchain,
            created_at,
            updated_at,
            expires_at,
            error,
        })
    }
}

fn json_required_string(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(JsonValue::as_string)
        .map(ToString::to_string)
        .ok_or_else(|| format!("{key} is required"))
}

fn prune_tokens(tokens: &mut Vec<DownloadTokenRecord>, now: i64) {
    tokens.retain(|token| token.expires_at > now);
}

fn save_metadata(storage: &Path, metadata: &BuildMetadata) -> io::Result<()> {
    let dir = build_dir(storage, &metadata.id);
    fs::create_dir_all(&dir)?;
    set_private_dir(&dir)?;
    let path = dir.join("metadata.json");
    atomic_write(&path, metadata.to_storage_json().as_bytes())
}

fn load_metadata(storage: &Path, id: &str) -> io::Result<Option<BuildMetadata>> {
    let path = metadata_path(storage, id);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    BuildMetadata::from_json(&bytes)
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn persist_artifact(storage: &Path, id: &str, source: &Path) -> io::Result<()> {
    let dir = build_dir(storage, id);
    fs::create_dir_all(&dir)?;
    set_private_dir(&dir)?;
    let final_path = artifact_path(storage, id);
    let tmp_path = final_path.with_extension("shortcut.tmp");
    {
        let mut input = File::open(source)?;
        let mut output = File::create(&tmp_path)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    sync_parent_dir(&final_path)?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

fn sync_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            let dir = File::open(parent)?;
            dir.sync_all()?;
        }
    }
    Ok(())
}

fn build_dir(storage: &Path, id: &str) -> PathBuf {
    storage.join("builds").join(&id[..2]).join(id)
}

fn metadata_path(storage: &Path, id: &str) -> PathBuf {
    build_dir(storage, id).join("metadata.json")
}

fn artifact_path(storage: &Path, id: &str) -> PathBuf {
    build_dir(storage, id).join("artifact.shortcut")
}

struct ResolvedDownload {
    name: String,
    artifact_path: PathBuf,
}

fn resolve_download(
    storage: &Path,
    token_hash: &str,
    now: i64,
) -> io::Result<Option<ResolvedDownload>> {
    for metadata in scan_metadata(storage)? {
        if metadata.status != BuildStatus::Ready || metadata.expires_at <= now {
            continue;
        }
        let token_matches = metadata
            .download_tokens
            .iter()
            .any(|token| token.hash == token_hash && token.expires_at > now);
        if token_matches {
            let artifact = artifact_path(storage, &metadata.id);
            if artifact.exists() {
                return Ok(Some(ResolvedDownload {
                    name: metadata.name,
                    artifact_path: artifact,
                }));
            }
            return Ok(None);
        }
    }
    Ok(None)
}

fn scan_metadata(storage: &Path) -> io::Result<Vec<BuildMetadata>> {
    let builds = storage.join("builds");
    if !builds.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for shard in fs::read_dir(builds)? {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }
        for build in fs::read_dir(shard.path())? {
            let build = build?;
            if !build.file_type()?.is_dir() {
                continue;
            }
            let metadata_path = build.path().join("metadata.json");
            if metadata_path.exists() {
                let bytes = fs::read(metadata_path)?;
                if let Ok(metadata) = BuildMetadata::from_json(&bytes) {
                    out.push(metadata);
                }
            }
        }
    }
    Ok(out)
}

fn run_gc(config: &GcConfig) -> io::Result<()> {
    let threshold = now_unix().saturating_sub(config.expired_before_age.as_secs() as i64);
    let metadata = scan_metadata(&config.storage)?;
    let mut removed = 0usize;
    for build in metadata {
        if build.expires_at < threshold {
            let dir = build_dir(&config.storage, &build.id);
            if dir.exists() {
                fs::remove_dir_all(&dir)?;
                removed += 1;
            }
        }
    }
    println!(
        "removed {removed} expired build(s) from {}",
        config.storage.display()
    );
    Ok(())
}

fn is_valid_build_id(id: &str) -> bool {
    id.len() == BUILD_ID_LEN
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_valid_download_token(token: &str) -> bool {
    token.starts_with("dl_")
        && token.len() >= 25
        && token[3..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn generate_download_token() -> io::Result<String> {
    let bytes = random_bytes(32)?;
    Ok(format!("dl_{}", base64url_no_pad(&bytes)))
}

fn random_bytes(len: usize) -> io::Result<Vec<u8>> {
    let mut file = File::open("/dev/urandom")?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    match bytes.len() - i {
        1 => {
            let n = (bytes[i] as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        }
        2 => {
            let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }
    out
}

fn create_private_temp_dir(storage: &Path, prefix: &str) -> io::Result<PathBuf> {
    let root = storage.join("tmp");
    fs::create_dir_all(&root)?;
    set_private_dir(&root)?;
    for _ in 0..16 {
        let suffix = random_bytes(12)
            .map(|bytes| base64url_no_pad(&bytes))
            .unwrap_or_else(|_| format!("{}-{}", std::process::id(), now_unix()));
        let dir = root.join(format!("{prefix}-{suffix}"));
        match fs::create_dir(&dir) {
            Ok(()) => {
                set_private_dir(&dir)?;
                return Ok(dir);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create unique temp directory",
    ))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn set_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn safe_filename(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        let replacement = matches!(ch, '\r' | '\n' | '"' | '\\' | '/' | ':' | ';');
        if replacement || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches(['.', ' ', '_']).to_string();
    if trimmed.is_empty() {
        "shortcut".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

#[derive(Debug)]
struct ApiError {
    code: &'static str,
    status: u16,
    message: String,
}

fn api_error(code: &'static str, status: u16, message: &str) -> ApiError {
    ApiError {
        code,
        status,
        message: message.to_string(),
    }
}

impl ApiError {
    fn into_response(self) -> HttpResponse {
        json_response(
            self.status,
            format!(
                r#"{{"ok":false,"error":{{"code":"{}","message":"{}"}}}}"#,
                self.code,
                json_escape(&self.message)
            ),
        )
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn json_response(status: u16, body: String) -> HttpResponse {
    binary_response(
        status,
        body.into_bytes(),
        vec![("Content-Type".to_string(), "application/json".to_string())],
    )
}

fn binary_response(status: u16, body: Vec<u8>, headers: Vec<(String, String)>) -> HttpResponse {
    HttpResponse {
        status,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\n",
        response.status,
        status_reason(response.status)
    )?;
    for (name, value) in response.headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(
        stream,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "OK",
    }
}

#[derive(Debug, Clone)]
enum JsonValue {
    Null,
    Bool(()),
    Number(i64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            JsonValue::Object(value) => Some(value),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(value) => Some(value),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<&str> {
        match self {
            JsonValue::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(value) => Some(*value),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(mut self) -> Result<JsonValue, String> {
        let value = self.parse_value()?;
        self.skip_ws();
        if self.pos != self.input.len() {
            return Err("trailing JSON content".to_string());
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(JsonValue::Number),
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(JsonValue::Null)
            }
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(JsonValue::Bool(()))
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(JsonValue::Bool(()))
            }
            _ => Err("invalid JSON value".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect_byte(b'{')?;
        let mut object = BTreeMap::new();
        self.skip_ws();
        if self.consume_byte(b'}') {
            return Ok(JsonValue::Object(object));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            let value = self.parse_value()?;
            object.insert(key, value);
            self.skip_ws();
            if self.consume_byte(b'}') {
                break;
            }
            self.expect_byte(b',')?;
        }
        Ok(JsonValue::Object(object))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect_byte(b'[')?;
        let mut values = Vec::new();
        self.skip_ws();
        if self.consume_byte(b']') {
            return Ok(JsonValue::Array(values));
        }
        loop {
            values.push(self.parse_value()?);
            self.skip_ws();
            if self.consume_byte(b']') {
                break;
            }
            self.expect_byte(b',')?;
        }
        Ok(JsonValue::Array(values))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect_byte(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err("unterminated JSON string".to_string());
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let escape = self
                        .next_byte()
                        .ok_or_else(|| "invalid JSON escape".to_string())?;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                self.expect_byte(b'\\')?;
                                self.expect_byte(b'u')?;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("invalid JSON surrogate pair".to_string());
                                }
                                let scalar = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                out.push(
                                    char::from_u32(scalar)
                                        .ok_or_else(|| "invalid JSON unicode scalar".to_string())?,
                                );
                            } else {
                                out.push(
                                    char::from_u32(code)
                                        .ok_or_else(|| "invalid JSON unicode scalar".to_string())?,
                                );
                            }
                        }
                        _ => return Err("invalid JSON escape".to_string()),
                    }
                }
                0x00..=0x1f => return Err("control character in JSON string".to_string()),
                _ => {
                    let rest = std::str::from_utf8(&self.input[self.pos..])
                        .map_err(|_| "JSON string must be UTF-8".to_string())?;
                    let ch = rest.chars().next().expect("peek returned byte");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn parse_number(&mut self) -> Result<i64, String> {
        let start = self.pos;
        if self.consume_byte(b'-') && self.peek().is_none() {
            return Err("invalid JSON number".to_string());
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err("only integer JSON numbers are supported".to_string());
        }
        let text = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| "invalid JSON number".to_string())?;
        text.parse::<i64>()
            .map_err(|_| "invalid JSON number".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let byte = self
                .next_byte()
                .ok_or_else(|| "unterminated JSON unicode escape".to_string())?;
            value = (value << 4)
                | match byte {
                    b'0'..=b'9' => (byte - b'0') as u32,
                    b'a'..=b'f' => (byte - b'a' + 10) as u32,
                    b'A'..=b'F' => (byte - b'A' + 10) as u32,
                    _ => return Err("invalid JSON unicode escape".to_string()),
                };
        }
        Ok(value)
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.input.get(self.pos..self.pos + literal.len()) == Some(literal) {
            self.pos += literal.len();
            Ok(())
        } else {
            Err("invalid JSON literal".to_string())
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), String> {
        if self.consume_byte(expected) {
            Ok(())
        } else {
            Err(format!("expected '{}'", expected as char))
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs() as i64
}

fn format_rfc3339(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn parse_rfc3339(value: &str) -> Option<i64> {
    if value.len() != 20 || !value.ends_with('Z') {
        return None;
    }
    let year = value[0..4].parse::<i32>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<i64>().ok()?;
    let minute = value[14..16].parse::<i64>().ok()?;
    let second = value[17..19].parse::<i64>().ok()?;
    if &value[4..5] != "-"
        || &value[7..8] != "-"
        || &value[10..11] != "T"
        || &value[13..14] != ":"
        || &value[16..17] != ":"
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let mut y = year as i64;
    let m = month as i64;
    let d = day as i64;
    y -= if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn sha256_hex(input: &[u8]) -> String {
    sha256(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h = H0;
    let bit_len = (input.len() as u64) * 8;
    let mut data = input.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn json_parser_handles_escaped_source() {
        let body = br#"{"name":"T","source_format":"cherri","source":"showNotification(\"ok\", \"x\")\n","ttl_seconds":60}"#;
        let request = BuildRequest::parse(body, 1024).unwrap();
        assert_eq!(request.source, "showNotification(\"ok\", \"x\")\n");
        assert_eq!(request.sign_mode, "anyone");
    }

    #[test]
    fn header_reader_does_not_consume_body_bytes() {
        let request = b"POST /api/builds HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\n\r\nsecret-body";
        let mut cursor = io::Cursor::new(request);
        let (headers, leftover) = read_http_headers_from(&mut cursor).unwrap();
        assert!(
            std::str::from_utf8(&headers)
                .unwrap()
                .contains("POST /api/builds")
        );
        assert!(leftover.is_empty());
        assert_eq!(
            cursor.position() as usize,
            request.len() - "secret-body".len()
        );
    }

    #[test]
    fn build_id_is_stable_32_hex() {
        let input = "cherri\nanyone\nshowNotification(\"ok\", \"x\")\n";
        let hash = sha256_hex(input.as_bytes());
        let id = &hash[..BUILD_ID_LEN];
        assert_eq!(id.len(), 32);
        assert!(is_valid_build_id(id));
    }

    #[test]
    fn token_format_is_url_safe_and_not_build_id_like() {
        let token = generate_download_token().unwrap();
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
    fn filename_sanitizer_removes_header_sensitive_chars() {
        assert_eq!(safe_filename(" bad/name\";\n "), "bad_name");
        assert_eq!(safe_filename("///"), "shortcut");
    }

    #[test]
    fn config_file_parses_flat_values_and_comments() {
        let root = test_temp_dir("shortcut-forge-config-file");
        let path = root.join("shortcut-forge.conf");
        fs::write(
            &path,
            r#"
# Flat key/value config.
host = "0.0.0.0"
port = 8787
public_base_url = "http://mac-mini.local:8787"
storage = "/Users/test/Library/Application Support/ShortcutForge/data"
auth_token = "token#with-hash"
cherri_bin = '/opt/cherri'
"#,
        )
        .unwrap();

        let config = parse_config_file(&path).unwrap();
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
            parse_u16_config(&flags, &file_config, "port", "SHORTCUT_FORGE_TEST_PORT").unwrap(),
            Some(9999)
        );
    }

    #[test]
    fn parse_cli_supports_operator_commands() {
        match parse_cli(vec!["status".to_string(), "--json".to_string()]).unwrap() {
            CommandMode::Status(command) => assert!(command.json),
            _ => panic!("expected status command"),
        }
        match parse_cli(vec![
            "config".to_string(),
            "set".to_string(),
            "public_base_url".to_string(),
            "http://mac-mini.local:8787".to_string(),
        ])
        .unwrap()
        {
            CommandMode::ConfigSet(command) => {
                assert_eq!(command.key, "public-base-url");
                assert_eq!(command.value, "http://mac-mini.local:8787");
            }
            _ => panic!("expected config set command"),
        }
        match parse_cli(vec![
            "token".to_string(),
            "rotate".to_string(),
            "--print".to_string(),
        ])
        .unwrap()
        {
            CommandMode::TokenRotate(command) => assert!(command.print),
            _ => panic!("expected token rotate command"),
        }
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

    #[test]
    fn build_renewal_rotates_download_url_without_plaintext_token_storage() {
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
            build_locks: Mutex::new(HashMap::new()),
            build_slots: Mutex::new(0),
            health_cache: Mutex::new(None),
            _storage_lock: StorageLock::acquire(&storage).unwrap(),
        };

        let request = BuildRequest {
            name: "Minimal".to_string(),
            source_format: "cherri".to_string(),
            source: "#define name Minimal\nshowNotification(\"ok\", \"x\")\n".to_string(),
            sign_mode: "anyone".to_string(),
            ttl_seconds: 120,
        };
        let first = build_or_renew(request, &state).unwrap();
        let first_token = first.download_url.rsplit('/').next().unwrap().to_string();
        let first_metadata = load_metadata(&storage, &first.id).unwrap().unwrap();
        assert_eq!(first.id.len(), BUILD_ID_LEN);
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
        let second = build_or_renew(request, &state).unwrap();
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

    #[test]
    fn expired_download_stops_and_repost_refreshes_same_id() {
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

    #[test]
    fn toolchain_fingerprint_change_rebuilds_same_id() {
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

    #[test]
    fn gc_removes_expired_build_directories() {
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

    #[test]
    fn build_slot_saturation_returns_server_busy() {
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

    #[test]
    fn build_pipeline_handles_relative_storage_paths() {
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
        );
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
            build_locks: Mutex::new(HashMap::new()),
            build_slots: Mutex::new(0),
            health_cache: Mutex::new(None),
            _storage_lock: StorageLock::acquire(storage).unwrap(),
        }
    }
}
