//! Binary creation shared by domain downloads. Directory handles prevent symlink races.
use super::*;
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::Component;

pub(crate) async fn create_binary_in_workspace(
    state: &AppState,
    input: &str,
    bytes: &[u8],
) -> Result<String, FileError> {
    let config = state.config.read().await.clone();
    let mut parts = Vec::new();
    for component in Path::new(input).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(FileError::new(
                        "email_path_outside_workspace",
                        "path leaves the workspace",
                    ));
                }
            }
            _ => {
                return Err(FileError::new(
                    "email_path_outside_workspace",
                    "path must be workspace-relative",
                ))
            }
        }
    }
    if parts.is_empty() {
        return Err(FileError::new("file_path_empty", "a file path is required"));
    }
    let relative: PathBuf = parts.iter().collect();
    let root = fs::canonicalize(&config.workspace_root)
        .map_err(|e| path_io_error(e, &config.workspace_root, false))?;
    let target = root.join(&relative);
    check_policy(&config, &target, Access::Write)?;
    if is_reserved_path(&config, &target) {
        return Err(FileError::new(
            "file_reserved_path",
            "Agentic runtime path is reserved",
        ));
    }
    // Pin the workspace before waiting, including every ancestor of its absolute
    // path. A root swapped for a symlink must not become the download directory.
    let mut directory = open_workspace_root(&root)?;
    let _lock = lock_target(state, &target).await;
    if fs::canonicalize(format!("/proc/self/fd/{}", directory.as_raw_fd()))
        .map_err(|e| path_io_error(e, &root, true))?
        != root
    {
        return Err(FileError::new(
            "file_path_changed",
            "workspace changed while waiting for the file lock",
        ));
    }
    // Each parent is opened relative to an already held descriptor. Never follow a symlink.
    let mut logical = root;
    let mut owned = Vec::new();
    let result = (|| {
        for part in &parts[..parts.len() - 1] {
            logical.push(part);
            let name = CString::new(part.as_encoded_bytes())
                .map_err(|_| FileError::new("file_path_invalid", "path contains NUL"))?;
            let mut fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            let mut created_parent = false;
            if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                check_policy(&config, &logical, Access::Write)?;
                if is_reserved_path(&config, &logical) {
                    return Err(FileError::new(
                        "file_reserved_path",
                        "Agentic runtime path is reserved",
                    ));
                }
                let created = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if created < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(FileError::new(
                        "file_write_failed",
                        "could not create parent directory",
                    ));
                }
                fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                created_parent = created == 0;
            }
            if fd < 0 {
                return Err(FileError::new(
                    "email_path_unsafe",
                    "parent must be a directory and must not be a symbolic link",
                ));
            }
            directory = unsafe { fs::File::from_raw_fd(fd) };
            if created_parent {
                let metadata = directory
                    .metadata()
                    .map_err(|e| path_io_error(e, &logical, true))?;
                owned.push(owned_parent_directory(&logical, &metadata));
            }
        }
        let descriptor_target = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
            .join(parts.last().expect("nonempty path"));
        use std::os::unix::fs::PermissionsExt;
        let temp = stage_temp(
            &descriptor_target,
            bytes,
            Some(&fs::Permissions::from_mode(0o600)),
        )?;
        commit_new_file(&temp, &descriptor_target)?;
        directory.sync_all().map_err(|_| {
            FileError::new(
                "file_sync_failed",
                "file was saved but directory durability could not be confirmed",
            )
        })?;
        Ok(relative.to_string_lossy().into_owned())
    })();
    if result.is_err() {
        cleanup_owned_parent_directories(&mut owned);
    }
    result
}

fn open_workspace_root(root: &Path) -> Result<fs::File, FileError> {
    let mut directory = fs::File::open("/").map_err(|error| path_io_error(error, root, true))?;
    for component in root.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let name = CString::new(part.as_encoded_bytes())
            .map_err(|_| FileError::new("file_path_invalid", "path contains NUL"))?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(FileError::new(
                "file_path_changed",
                "workspace path is unavailable or changed",
            ));
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tier ⑤: exercise real binary creation and OS path handling. Existing patch
    // tests do not protect download no-replace or the independent workspace bound.
    #[tokio::test]
    async fn email_download_is_atomic_no_replace_and_stays_inside_workspace() {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = std::env::temp_dir().join(format!("email-download-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let _cleanup = Cleanup(root.clone());
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root.join("workspace");
        config.policy.enabled = false;
        let state = crate::startup::build_app_state_with_private_state(
            root.join("config.json"),
            config,
            crate::state::RuntimeModel::local(crate::state::CapabilityProfile::Room),
            false,
            crate::private_state::PrivateStatePaths::for_test(root.join("private")),
        )
        .unwrap();
        let original = [0, 255, 128, 1];
        let saved = create_binary_in_workspace(&state, "downloads/email/a/file.bin", &original)
            .await
            .unwrap();
        assert_eq!(
            fs::read(root.join("workspace").join(saved)).unwrap(),
            original
        );
        assert!(
            create_binary_in_workspace(&state, "downloads/email/a/file.bin", b"replacement")
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(root.join("workspace/downloads/email/a/file.bin")).unwrap(),
            original
        );
        assert!(
            create_binary_in_workspace(&state, "../outside.bin", b"escape")
                .await
                .is_err()
        );
        assert!(!root.join("outside.bin").exists());
        fs::create_dir(root.join("outside")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("workspace/linked")).unwrap();
        assert!(
            create_binary_in_workspace(&state, "linked/escape.bin", b"escape")
                .await
                .is_err()
        );
        assert!(!root.join("outside/escape.bin").exists());
        let files = fs::read_dir(root.join("workspace/downloads/email/a"))
            .unwrap()
            .count();
        assert_eq!(files, 1); // Failed creation leaves neither replacement nor temporary files.

        // Arrange the actual await window: replacement of the workspace root
        // while the download waits must not redirect the write outside it.
        let target = root.join("workspace/root-race.bin");
        let lock = lock_target(&state, &target).await;
        let download_state = state.clone();
        let download = tokio::spawn(async move {
            create_binary_in_workspace(&download_state, "root-race.bin", b"escape").await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let waiting = state
                    .file_locks
                    .lock()
                    .await
                    .get(&target)
                    .is_some_and(|weak| weak.strong_count() >= 2);
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        fs::rename(root.join("workspace"), root.join("original-workspace")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("workspace")).unwrap();
        drop(lock);
        assert!(download.await.unwrap().is_err());
        assert!(!root.join("outside/root-race.bin").exists());
    }
}
