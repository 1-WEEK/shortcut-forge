use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::model::*;

pub fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME is not set".to_string())
}

pub fn app_support_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join("ShortcutForge"))
}

pub fn default_storage_dir() -> Result<PathBuf, String> {
    Ok(app_support_dir()?.join("data"))
}

pub fn default_config_path() -> Result<PathBuf, String> {
    Ok(app_support_dir()?.join("shortcut-forge.conf"))
}

pub fn default_log_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("Logs")
        .join("ShortcutForge"))
}

pub fn default_launch_agent_path() -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

pub fn load_config_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut map = HashMap::new();
    for (key, value) in table {
        let normalized_key = key.replace('_', "-");
        let str_value = match value {
            toml::Value::String(s) => s,
            toml::Value::Integer(i) => i.to_string(),
            toml::Value::Float(f) => f.to_string(),
            toml::Value::Boolean(b) => b.to_string(),
            _ => continue,
        };
        map.insert(normalized_key, str_value);
    }
    Ok(map)
}

pub fn config_value(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Option<String> {
    config_value_with_env(flags, file_config, flag, Some(env_name))
}

pub fn config_value_with_env(
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

pub fn parse_u16_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<u16>, String> {
    parse_u16_value(config_value(flags, file_config, flag, env_name), flag)
}

pub fn parse_u64_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<u64>, String> {
    parse_u64_value(config_value(flags, file_config, flag, env_name), flag)
}

pub fn parse_usize_config(
    flags: &HashMap<String, String>,
    file_config: &HashMap<String, String>,
    flag: &str,
    env_name: &str,
) -> Result<Option<usize>, String> {
    parse_usize_value(config_value(flags, file_config, flag, env_name), flag)
}

pub fn parse_u16_value(value: Option<String>, flag: &str) -> Result<Option<u16>, String> {
    value
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| format!("{flag} must be a u16"))
        })
        .transpose()
}

pub fn parse_u64_value(value: Option<String>, flag: &str) -> Result<Option<u64>, String> {
    value
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{flag} must be a number"))
        })
        .transpose()
}

pub fn parse_usize_value(value: Option<String>, flag: &str) -> Result<Option<usize>, String> {
    value
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{flag} must be a number"))
        })
        .transpose()
}

pub fn build_runtime_config(
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
    .map(|url| normalize_public_base_url(&url, port))
    .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
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

pub fn build_runtime_config_from_file(
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
        .map(|url| normalize_public_base_url(&url, port))
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
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

pub fn normalize_public_base_url(url: &str, port: u16) -> String {
    let trimmed = url.trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return trimmed.to_string();
    }
    if !authority_has_port(authority) {
        let is_default = if trimmed.starts_with("https://") {
            port == 443
        } else {
            port == 80
        };
        if !is_default {
            return format!("{}:{}", trimmed, port);
        }
    }
    trimmed.to_string()
}

pub(crate) fn authority_has_port(authority: &str) -> bool {
    if authority.is_empty() {
        return false;
    }
    let host_start = authority.rfind('@').map_or(0, |i| i + 1);
    let host_port = &authority[host_start..];
    if let Some(rest) = host_port.strip_prefix('[') {
        rest.contains("]:")
    } else {
        host_port.contains(':')
    }
}

pub fn validate_httpish_url(url: &str) -> Result<(), String> {
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

pub fn extract_url_host(url: &str) -> Result<String, String> {
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

pub fn extract_url_path(url: &str) -> Result<&str, String> {
    let scheme_end = url
        .find("://")
        .ok_or_else(|| "URL must include a scheme".to_string())?;
    let rest = &url[scheme_end + 3..];
    let slash = rest
        .find('/')
        .ok_or_else(|| "URL must include a path".to_string())?;
    Ok(&rest[slash..])
}

pub fn local_service_base_url(config: &Config) -> String {
    let host = match config.host.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    format!("http://{host}:{}", config.port)
}

pub fn suggest_public_base_url(port: u16) -> String {
    detect_local_hostname()
        .map(|hostname| format!("http://{hostname}.local:{port}"))
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
}

pub fn detect_local_hostname() -> Option<String> {
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

pub fn resolve_command_path(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.exists().then(|| candidate.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|path| path.exists())
}

pub fn probe_command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
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

pub fn probe_command_success(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn first_sanitized_line(text: &str) -> Option<String> {
    text.lines()
        .map(strip_ansi_escape)
        .map(|line| line.trim().to_string())
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

pub(crate) fn strip_ansi_escape(text: &str) -> String {
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

pub fn current_uid_string() -> Result<String, String> {
    env::var("UID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| probe_command_output("id", &["-u"]))
        .ok_or_else(|| "failed to determine current user id".to_string())
}

pub fn launchctl_service_target() -> Result<String, String> {
    Ok(format!(
        "gui/{}/{}",
        current_uid_string()?,
        LAUNCH_AGENT_LABEL
    ))
}

pub fn operator_config_path(explicit: Option<String>, default: PathBuf) -> Result<PathBuf, String> {
    Ok(explicit
        .map(PathBuf::from)
        .or_else(|| env::var("SHORTCUT_SERVER_CONFIG").ok().map(PathBuf::from))
        .unwrap_or(default))
}

pub fn redacted_effective_config(config: &Config, expired_before: Option<&str>) -> String {
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

pub fn format_config_value_for_key(key: &str, value: &str) -> String {
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

pub fn validate_config_assignment(key: &str, value: &str) -> Result<(), String> {
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

pub fn is_supported_config_set_key(key: &str) -> bool {
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

pub fn should_restart_for_config_key(key: &str) -> bool {
    key != "expired-before"
}

pub fn render_operator_config(config: &Config, expired_before: &str) -> String {
    format!(
        r#"# Shortcut Forge config file.
# Format: TOML. String values require quotes.

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

pub fn parse_age(value: &str) -> Result<Duration, String> {
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

pub fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn load_runtime_config_from_path(path: &Path, require_auth: bool) -> Result<Config, String> {
    let file_config = load_config_file(path)?;
    build_runtime_config_from_file(&file_config, require_auth)
}

pub fn localize_service_url(url: &str, config: &Config) -> Result<String, String> {
    Ok(format!(
        "{}{}",
        local_service_base_url(config),
        extract_url_path(url)?
    ))
}
