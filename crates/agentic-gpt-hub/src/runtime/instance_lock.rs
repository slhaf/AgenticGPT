use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

pub(crate) struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub(crate) fn acquire(resource: &Path, suffix: &str, kind: &str) -> Result<Self> {
        if let Some(parent) = resource
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let resource = normalize_resource_path(resource)?;
        let lock_path = lock_path(&resource, suffix);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {} lock {}", kind, lock_path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(anyhow!(
                    "another {kind} instance is already running for {} (lock: {})",
                    resource.display(),
                    lock_path.display()
                ));
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| {
                    format!("failed to acquire {} lock {}", kind, lock_path.display())
                });
            }
        }
        file.set_len(0)?;
        writeln!(file, "pid={}", std::process::id())?;
        Ok(Self { _file: file })
    }
}

fn normalize_resource_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("resource path has no file name: {}", path.display()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.canonicalize()?.join(file_name))
}

fn lock_path(resource: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(resource.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_second_lock_and_releases_on_drop() {
        let root = std::env::temp_dir().join(format!("hub-lock-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let resource = root.join("hub.db");

        let first = InstanceLock::acquire(&resource, ".serve.lock", "hub").unwrap();
        let error = InstanceLock::acquire(&resource, ".serve.lock", "hub")
            .err()
            .unwrap();
        assert!(error.to_string().contains("already running"));
        drop(first);
        InstanceLock::acquire(&resource, ".serve.lock", "hub").unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }
}
