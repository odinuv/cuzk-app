//! Cross-process lock that keeps two check runs (manual + scheduled) apart.

use crate::error::CoreError;
use fs4::{FileExt, TryLockError};
use std::fs::{File, OpenOptions};
use std::path::Path;

/// Exclusive lock on the lock file; released when dropped (or when the process exits).
#[derive(Debug)]
pub struct RunLock {
    file: File,
}

impl RunLock {
    /// Takes the lock without waiting. Creates the file and its parent directory when
    /// missing. Fails with [`CoreError::AlreadyRunning`] when another holder has it.
    pub fn acquire(path: &Path) -> Result<RunLock, CoreError> {
        let io_err =
            |e: std::io::Error| CoreError::Db(format!("Zámek kontroly nelze vytvořit: {e}"));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_err)?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(RunLock { file }),
            Err(TryLockError::WouldBlock) => Err(CoreError::AlreadyRunning),
            Err(TryLockError::Error(e)) => Err(io_err(e)),
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}
