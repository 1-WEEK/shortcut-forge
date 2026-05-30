use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::model::{
    BuildMetadata, BuildStatus, DownloadTokenRecord, GcConfig, ResolvedDownload,
};

pub fn build_dir(storage: &Path, id: &str) -> PathBuf {
    storage.join("builds").join(&id[..2]).join(id)
}

pub(crate) fn metadata_path(storage: &Path, id: &str) -> PathBuf {
    build_dir(storage, id).join("metadata.json")
}

pub fn artifact_path(storage: &Path, id: &str) -> PathBuf {
    build_dir(storage, id).join("artifact.shortcut")
}

pub fn save_metadata(storage: &Path, metadata: &BuildMetadata) -> io::Result<()> {
    let dir = build_dir(storage, &metadata.id);
    fs::create_dir_all(&dir)?;
    set_private_dir(&dir)?;
    let path = dir.join("metadata.json");
    atomic_write(&path, metadata.to_storage_json().as_bytes())
}

pub fn load_metadata(storage: &Path, id: &str) -> io::Result<Option<BuildMetadata>> {
    let path = metadata_path(storage, id);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    BuildMetadata::from_json(&bytes)
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn persist_artifact(storage: &Path, id: &str, source: &Path) -> io::Result<()> {
    let dir = build_dir(storage, id);
    fs::create_dir_all(&dir)?;
    set_private_dir(&dir)?;
    let final_path = artifact_path(storage, id);
    let tmp_path = final_path.with_extension("shortcut.tmp");
    {
        let mut input = fs::File::open(source)?;
        let mut output = fs::File::create(&tmp_path)?;
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
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

pub(crate) fn sync_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            let dir = fs::File::open(parent)?;
            dir.sync_all()?;
        }
    }
    Ok(())
}

pub fn scan_metadata(storage: &Path) -> io::Result<Vec<BuildMetadata>> {
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

pub fn resolve_download(
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

pub fn prune_tokens(tokens: &mut Vec<DownloadTokenRecord>, now: i64) {
    tokens.retain(|token| token.expires_at > now);
}

pub fn is_valid_build_id(id: &str) -> bool {
    use crate::model::BUILD_ID_LEN;
    id.len() == BUILD_ID_LEN
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn is_valid_download_token(token: &str) -> bool {
    token.starts_with("dl_")
        && token.len() >= 25
        && token[3..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub fn safe_filename(name: &str) -> String {
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

pub(crate) fn set_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(input);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max_len {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        diff |= (left ^ right) as usize;
    }
    diff == 0
}

pub fn random_bytes(len: usize) -> io::Result<Vec<u8>> {
    let mut file = fs::File::open("/dev/urandom")?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn generate_download_token() -> io::Result<String> {
    let bytes = random_bytes(32)?;
    Ok(format!("dl_{}", base64url_no_pad(&bytes)))
}

pub(crate) fn base64url_no_pad(bytes: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn create_private_temp_dir(storage: &Path, prefix: &str) -> io::Result<PathBuf> {
    let root = storage.join("tmp");
    fs::create_dir_all(&root)?;
    set_private_dir(&root)?;
    for _ in 0..16 {
        let suffix = random_bytes(12)
            .map(|bytes| base64url_no_pad(&bytes))
            .unwrap_or_else(|_| format!("{}-{}", std::process::id(), crate::model::now_unix()));
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

pub fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn run_gc(config: &GcConfig) -> io::Result<()> {
    let threshold = crate::model::now_unix().saturating_sub(config.expired_before_age.as_secs() as i64);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn filename_sanitizer_removes_header_sensitive_chars() {
        assert_eq!(safe_filename(" bad/name\";\n "), "bad_name");
        assert_eq!(safe_filename("///"), "shortcut");
    }
}
