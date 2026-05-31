use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use crate::config::{probe_command_output, probe_command_success};
use crate::error::ApiError;
use crate::model::{BuildRequest, CommandCapture, Config, Toolchain, now_unix};
use crate::store::{create_private_temp_dir, sha256_hex, write_private_file};

pub async fn run_build_pipeline(
    request: &BuildRequest,
    id: &str,
    config: &Config,
) -> Result<PathBuf, ApiError> {
    let temp_dir = create_private_temp_dir(&config.storage, "build")
        .and_then(fs::canonicalize)
        .map_err(|_| ApiError::internal_error("failed to create build directory"))?;
    let cleanup = TempDirCleanup(temp_dir.clone());
    let source_path = temp_dir.join("source.cherri");
    let mut unsigned_path = temp_dir.join("unsigned.shortcut");
    let signed_path = temp_dir.join("signed.shortcut");
    write_private_file(&source_path, request.source.as_bytes())
        .map_err(|_| ApiError::internal_error("failed to write source"))?;

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
    .await
    .map_err(|_| ApiError::internal_error("failed to run cherri"))?;
    if compile.timed_out {
        return Err(ApiError::timeout("cherri compile timed out"));
    }
    if !compile.success {
        return Err(ApiError::build_failed("Cherri compile failed"));
    }
    if !unsigned_path.exists() {
        if let Some(discovered) = find_cherri_unsigned_output(&temp_dir, &signed_path) {
            unsigned_path = discovered;
        } else {
            return Err(ApiError::build_failed(
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
    .await
    .map_err(|_| ApiError::internal_error("failed to run shortcuts sign"))?;
    if sign.timed_out {
        return Err(ApiError::timeout("shortcuts sign timed out"));
    }
    if !sign.success {
        return Err(ApiError::sign_failed("shortcuts sign failed"));
    }
    if !signed_path.exists() {
        return Err(ApiError::sign_failed(
            "shortcuts sign did not produce output",
        ));
    }

    let retained = temp_dir
        .parent()
        .unwrap_or(&temp_dir)
        .join(format!("signed-{id}-{}.shortcut", now_unix()));
    fs::create_dir_all(retained.parent().expect("retained has parent"))
        .map_err(|_| ApiError::internal_error("failed to prepare temp output"))?;
    fs::rename(&signed_path, &retained)
        .or_else(|_| fs::copy(&signed_path, &retained).map(|_| ()))
        .map_err(|_| ApiError::internal_error("failed to retain signed output"))?;
    let _ = fs::remove_file(&source_path);
    let _ = fs::remove_file(&unsigned_path);
    drop(cleanup);
    Ok(retained)
}

pub fn find_cherri_unsigned_output(temp_dir: &Path, signed_path: &Path) -> Option<PathBuf> {
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

pub struct TempDirCleanup(pub PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub async fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    work_dir: &Path,
    timeout: Duration,
    label: &str,
) -> io::Result<CommandCapture> {
    let stdout_path = work_dir.join(format!("{label}.stdout"));
    let stderr_path = work_dir.join(format!("{label}.stderr"));
    let stdout = fs::File::create(&stdout_path)?;
    let stderr = fs::File::create(&stderr_path)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(work_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(stdout))
        .stderr(std::process::Stdio::from(stderr));
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let mut child = command.spawn()?;

    let result = tokio::time::timeout(timeout, child.wait()).await;
    match result {
        Ok(Ok(status)) => Ok(CommandCapture {
            success: status.success(),
            timed_out: false,
        }),
        Ok(Err(_)) => Ok(CommandCapture {
            success: false,
            timed_out: false,
        }),
        Err(_) => {
            #[cfg(unix)]
            {
                let pid = child.id().unwrap_or(0) as i32;
                if pid > 0 {
                    unsafe {
                        let _ = kill(-pid, 9);
                    }
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            Ok(CommandCapture {
                success: false,
                timed_out: true,
            })
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn probe_toolchain(config: &Config) -> Toolchain {
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
