use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_ID_LEN: usize = 32;
pub const DEFAULT_PORT: u16 = 8787;
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 524_288;
pub const DEFAULT_BUILD_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_HEALTH_CACHE_SECONDS: u64 = 60;
pub const DEFAULT_TTL_SECONDS: u64 = 2_592_000;
pub const MIN_TTL_SECONDS: u64 = 60;
pub const MAX_TTL_SECONDS: u64 = 2_592_000;
pub const LAUNCH_AGENT_LABEL: &str = "com.shortcut-forge";
#[allow(dead_code)]
pub const DEFAULT_INIT_HOST: &str = "0.0.0.0";
#[allow(dead_code)]
pub const DEFAULT_LOG_LINES: usize = 80;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub public_base_url: String,
    pub storage: std::path::PathBuf,
    pub max_source_bytes: usize,
    pub build_timeout: std::time::Duration,
    pub max_build_concurrency: usize,
    pub auth_token: String,
    pub health_cache_ttl: std::time::Duration,
    pub cherri_bin: String,
    pub shortcuts_bin: String,
}

#[derive(Clone, Debug)]
pub struct GcConfig {
    pub storage: std::path::PathBuf,
    pub expired_before_age: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildStatus {
    Ready,
    Failed,
}

impl BuildStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildStatus::Ready => "ready",
            BuildStatus::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(BuildStatus::Ready),
            "failed" => Some(BuildStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DownloadTokenRecord {
    pub hash: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct BuildMetadata {
    pub id: String,
    pub name: String,
    pub source_format: String,
    pub source_hash: String,
    pub sign_mode: String,
    pub status: BuildStatus,
    pub download_tokens: Vec<DownloadTokenRecord>,
    pub toolchain: Toolchain,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: i64,
    pub error: Option<ErrorBody>,
}

impl BuildMetadata {
    pub fn status_for_api(&self, now: i64) -> &'static str {
        if self.status == BuildStatus::Ready && self.expires_at <= now {
            "expired"
        } else {
            self.status.as_str()
        }
    }

    pub fn active_download_token_count(&self, now: i64) -> usize {
        self.download_tokens
            .iter()
            .filter(|token| token.expires_at > now)
            .count()
    }

    pub fn to_api_json(&self, now: i64) -> String {
        let error = self.error.as_ref().map_or_else(
            || "null".to_string(),
            |error| serde_json::to_string(error).unwrap_or_else(|_| "null".to_string()),
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

    pub fn to_storage_json(&self) -> String {
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
            |error| serde_json::to_string(error).unwrap_or_else(|_| "null".to_string()),
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

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
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
            .and_then(|v| v.as_array())
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
            .and_then(|v| v.as_object())
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
            Some(serde_json::Value::Null) | None => None,
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

#[derive(Clone, Debug)]
pub struct Toolchain {
    pub cherri: String,
    pub shortcuts_sign: String,
    pub fingerprint: String,
}

impl Toolchain {
    pub fn is_available(&self) -> bool {
        self.cherri != "unavailable" && self.shortcuts_sign == "available"
    }
}

#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub name: String,
    pub source_format: String,
    pub source: String,
    pub sign_mode: String,
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug)]
pub struct BuildResponse {
    pub id: String,
    pub download_url: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct LaunchAgentStatus {
    pub loaded: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub state: String,
    #[allow(dead_code)]
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct HealthProbe {
    pub ok: bool,
    pub status: String,
    pub cherri: Option<String>,
    pub shortcuts_sign: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedToolchain {
    pub probed_at: std::time::Instant,
    pub toolchain: Toolchain,
}

#[derive(Debug)]
pub struct CommandCapture {
    pub success: bool,
    pub timed_out: bool,
}

#[derive(Debug)]
pub struct CurlResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug)]
pub struct BuildApiResult {
    pub id: String,
    pub download_url: String,
    pub expires_at: String,
}

#[derive(Debug)]
pub struct ResolvedDownload {
    pub name: String,
    pub artifact_path: std::path::PathBuf,
}

pub fn format_rfc3339(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub fn parse_rfc3339(value: &str) -> Option<i64> {
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

pub fn json_escape(value: &str) -> String {
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

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs() as i64
}
