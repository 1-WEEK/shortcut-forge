use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

#[cfg(unix)]
use std::os::fd::AsRawFd;

use tokio::sync::Mutex;

use crate::build::probe_toolchain;
use crate::build_lifecycle::BuildLifecycle;
use crate::model::{CachedToolchain, Config, Toolchain};

pub struct AppState {
    pub config: Config,
    pub builds: BuildLifecycle,
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
