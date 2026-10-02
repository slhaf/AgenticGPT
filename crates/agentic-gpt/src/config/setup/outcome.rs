use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{acquire_config_mutation_lock, sparse_config_json, write_config_with_backup};
use crate::config_templates::{
    build_config, InitBuild, InitSummary, PendingAction, SecretValue, SecretWritePlan,
};

use super::model::{SetupField, SetupSession};
use super::review::ReviewModel;
use super::validation::{ValidationError, ValidationErrors};

// This type intentionally has no Debug implementation: it owns the in-memory
// write plan and can therefore own secret bytes.
pub(crate) struct WizardOutcome {
    pub(crate) build: InitBuild,
    pub(crate) secret_write: Option<SecretWritePlan>,
    pub(crate) summary: String,
}

impl SetupSession {
    pub(crate) fn into_wizard_outcome(self) -> Result<WizardOutcome, ValidationErrors> {
        self.validate_for_review()?;
        let review = self.review_model()?;
        let input = self.build_active_input()?;
        let mut build = build_config(input).map_err(|_| {
            vec![ValidationError {
                field: SetupField::Mode,
                code: "config_init_build_invalid",
            }]
        })?;

        let secret_write = if self.selected_mode()
            == crate::config_templates::RuntimeMode::Standalone
            && self.standalone().provision_secret_now
        {
            self.standalone()
                .secret_value
                .as_ref()
                .map(|value| SecretWritePlan {
                    path: PathBuf::from(
                        self.standalone()
                            .secret_path
                            .trim()
                            .strip_prefix("file:")
                            .unwrap_or_else(|| self.standalone().secret_path.trim()),
                    ),
                    value: SecretValue::new(value.expose()),
                })
        } else {
            None
        };
        if secret_write.is_some() {
            build
                .pending
                .retain(|action| *action != PendingAction::ProvisionTunnelSecret);
        }

        Ok(WizardOutcome {
            build,
            secret_write,
            summary: outcome_summary(&review),
        })
    }
}

fn outcome_summary(review: &ReviewModel) -> String {
    let mut lines = vec![
        "Configuration ready".to_string(),
        format!("Mode: {:?}", review.mode),
        format!("Profile: {:?}", review.profile),
        format!("Config path: {}", review.config_path.display()),
    ];
    if let Some(secret_write) = &review.secret_write {
        if secret_write.will_write {
            lines.push(format!(
                "Secret file: {} (value hidden)",
                secret_write.path.display()
            ));
        }
    }
    for action in &review.pending_actions {
        lines.push(format!("Pending action: {action:?}"));
    }
    lines.join("\n")
}

enum PriorSecretState {
    Absent,
    Existing { bytes: Vec<u8>, mode: u32 },
}

struct TemporarySecretFile {
    path: Option<PathBuf>,
}

impl Drop for TemporarySecretFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}
#[derive(Deserialize, Serialize)]
struct SetupJournal {
    secret_path: PathBuf,
    backup_path: Option<PathBuf>,
    old_config_hash: Option<String>,
    new_config_hash: String,
    old_secret_hash: Option<String>,
    new_secret_hash: String,
    old_secret_mode: Option<u32>,
}

fn setup_journal_path(config_path: &Path) -> Option<PathBuf> {
    let config_path = lexical_absolute(config_path).ok()?;
    let parent = config_path.parent()?;
    let file_name = config_path.file_name()?.to_string_lossy();
    Some(parent.join(format!(".{file_name}.setup-journal")))
}

fn bytes_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn path_hash(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(anyhow!("config_init_recovery_conflict"));
            }
            Ok(Some(bytes_hash(&fs::read(path)?)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn stage_setup_journal(path: &Path, journal: &SetupJournal) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!("config_init_config_write_failed"));
    }
    let bytes = serde_json::to_vec(journal)?;
    for _ in 0..128 {
        let temporary = parent.join(format!(
            ".setup-journal-tmp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if fs::symlink_metadata(path).is_ok() {
            let _ = fs::remove_file(&temporary);
            return Err(anyhow!("config_init_recovery_conflict"));
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        sync_parent(parent)?;
        return Ok(());
    }
    Err(anyhow!("config_init_config_write_failed"))
}

fn cleanup_setup_journal(path: &Path, journal: &SetupJournal) -> Result<()> {
    if let Some(backup) = journal.backup_path.as_deref() {
        match fs::remove_file(backup) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        sync_parent(parent)?;
    }
    Ok(())
}

pub(crate) fn recover_pending_setup_locked(config_path: &Path) -> Result<()> {
    let Some(journal_path) = setup_journal_path(config_path) else {
        return Ok(());
    };
    match fs::symlink_metadata(&journal_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(anyhow!("config_init_recovery_conflict"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let bytes = fs::read(&journal_path)?;
    let journal: SetupJournal =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("config_init_recovery_invalid"))?;
    let config_hash = path_hash(config_path)?;
    let secret_hash = path_hash(&journal.secret_path)?;
    if config_hash.as_deref() == Some(journal.new_config_hash.as_str()) {
        if secret_hash.as_deref() != Some(journal.new_secret_hash.as_str()) {
            return Err(anyhow!("config_init_recovery_conflict"));
        }
        return cleanup_setup_journal(&journal_path, &journal)
            .map_err(|_| anyhow!("config_init_recovery_cleanup_failed"));
    }
    if config_hash.as_ref() != journal.old_config_hash.as_ref() {
        return Err(anyhow!("config_init_recovery_conflict"));
    }
    if secret_hash.as_deref() == Some(journal.new_secret_hash.as_str()) {
        match (&journal.backup_path, journal.old_secret_hash.as_ref()) {
            (Some(backup), Some(old_hash)) => {
                if path_hash(backup)?.as_deref() != Some(old_hash.as_str()) {
                    return Err(anyhow!("config_init_recovery_conflict"));
                }
                let bytes =
                    fs::read(backup).map_err(|_| anyhow!("config_init_recovery_conflict"))?;
                atomically_write_secret(
                    &journal.secret_path,
                    &bytes,
                    journal.old_secret_mode.unwrap_or(0o600),
                )
                .map_err(|_| anyhow!("config_init_recovery_rollback_failed"))?;
            }
            (None, None) => {
                match fs::remove_file(&journal.secret_path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(anyhow!("config_init_recovery_rollback_failed")),
                }
                if let Some(parent) = journal.secret_path.parent() {
                    sync_parent(parent)?;
                }
            }
            _ => return Err(anyhow!("config_init_recovery_conflict")),
        }
    } else if secret_hash.as_ref() != journal.old_secret_hash.as_ref() {
        return Err(anyhow!("config_init_recovery_conflict"));
    }
    cleanup_setup_journal(&journal_path, &journal)
        .map_err(|_| anyhow!("config_init_recovery_cleanup_failed"))
}

static SECRET_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const SECRET_TEMP_ATTEMPTS: usize = 128;
pub(crate) fn commit_wizard_outcome(
    config_path: &Path,
    outcome: WizardOutcome,
) -> Result<InitSummary> {
    let _lock = acquire_config_mutation_lock(config_path)
        .map_err(|_| anyhow!("config_init_config_write_failed"))?;
    recover_pending_setup_locked(config_path)?;
    if fs::symlink_metadata(config_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(anyhow!("config_path_symlink_rejected"));
    }
    let WizardOutcome {
        build,
        secret_write,
        summary,
    } = outcome;
    let _summary = summary;
    let summary = InitSummary {
        mode: build.mode,
        profile: build.profile,
        config_path: config_path.to_path_buf(),
        pending: build.pending.clone(),
    };

    let Some(plan) = secret_write else {
        write_config_with_backup(config_path, &build.config)?;
        return Ok(summary);
    };

    if paths_refer_to_same_file(config_path, &plan.path)? {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    let (target, parent, prior) = validate_and_capture_secret_target(&plan.path)?;
    if paths_refer_to_same_file(config_path, &target)? {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    fs::create_dir_all(&parent).map_err(|_| anyhow!("config_init_secret_parent_invalid"))?;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| anyhow!("config_init_secret_parent_invalid"))?;

    let config_bytes = sparse_config_json(&build.config, false)?.into_bytes();
    let old_config_hash = match fs::read(config_path) {
        Ok(bytes) => Some(bytes_hash(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let new_secret = plan.value.expose().as_bytes();
    let old_secret_hash = match &prior {
        PriorSecretState::Absent => None,
        PriorSecretState::Existing { bytes, .. } => Some(bytes_hash(bytes)),
    };
    let backup_path = match &prior {
        PriorSecretState::Absent => None,
        PriorSecretState::Existing { bytes, .. } => {
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "secret".to_string());
            let backup = parent.join(format!(
                ".{name}.setup-backup-{}",
                uuid::Uuid::new_v4().simple()
            ));
            atomically_write_secret(&backup, bytes, 0o600)
                .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
            Some(backup)
        }
    };
    let journal_path = setup_journal_path(config_path)
        .ok_or_else(|| anyhow!("config_init_config_write_failed"))?;
    let journal = SetupJournal {
        secret_path: target.clone(),
        backup_path,
        old_config_hash,
        new_config_hash: bytes_hash(&config_bytes),
        old_secret_hash,
        new_secret_hash: bytes_hash(new_secret),
        old_secret_mode: match &prior {
            PriorSecretState::Existing { mode, .. } => Some(*mode),
            PriorSecretState::Absent => None,
        },
    };
    if let Err(_error) = stage_setup_journal(&journal_path, &journal) {
        if let Some(backup) = journal.backup_path.as_deref() {
            let _ = fs::remove_file(backup);
        }
        return Err(anyhow!("config_init_config_write_failed"));
    }

    if path_hash(config_path)?.as_ref() != journal.old_config_hash.as_ref() {
        return Err(anyhow!("config_init_recovery_conflict"));
    }
    atomically_write_secret(&target, new_secret, 0o600)
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;

    if let Err(error) = write_config_with_backup(config_path, &build.config) {
        if error
            .to_string()
            .starts_with("config_commit_sync_failed_after_rename")
        {
            return Err(anyhow!(
                "config_init_config_commit_sync_failed_after_rename"
            ));
        }
        let rollback_result = match &prior {
            PriorSecretState::Absent => match fs::remove_file(&target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(anyhow!("config_init_secret_rollback_failed")),
            },
            PriorSecretState::Existing { bytes, mode } => {
                atomically_write_secret(&target, bytes, *mode)
            }
        };
        return match rollback_result {
            Ok(()) => {
                let cleanup = cleanup_setup_journal(&journal_path, &journal);
                if cleanup.is_ok() {
                    Err(anyhow!("config_init_config_write_failed"))
                } else {
                    Err(anyhow!(
                        "config_init_config_write_failed: config_init_recovery_pending"
                    ))
                }
            }
            Err(_) => Err(anyhow!(
                "config_init_config_write_failed: config_init_secret_rollback_failed"
            )),
        };
    }
    cleanup_setup_journal(&journal_path, &journal)
        .map_err(|_| anyhow!("config_init_commit_recovery_pending"))?;
    Ok(summary)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool> {
    let left = lexical_absolute(&crate::exec::expand_pathbuf(left)?)?;
    let right = lexical_absolute(&crate::exec::expand_pathbuf(right)?)?;
    if left == right {
        return Ok(true);
    }
    Ok(canonicalize_with_existing_ancestor(&left)? == canonicalize_with_existing_ancestor(&right)?)
}

fn canonicalize_with_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut candidate = path.to_path_buf();
    let mut missing_components = Vec::new();
    loop {
        match fs::canonicalize(&candidate) {
            Ok(mut canonical) => {
                for component in missing_components.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(file_name) = candidate.file_name() else {
                    return Ok(path.to_path_buf());
                };
                missing_components.push(file_name.to_os_string());
                let Some(parent) = candidate.parent() else {
                    return Ok(path.to_path_buf());
                };
                candidate = parent.to_path_buf();
            }
            Err(_) => return Ok(path.to_path_buf()),
        }
    }
}

fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn validate_and_capture_secret_target(path: &Path) -> Result<(PathBuf, PathBuf, PriorSecretState)> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    let target = lexical_absolute(
        &crate::exec::expand_pathbuf(path)
            .map_err(|_| anyhow!("config_init_secret_path_invalid"))?,
    )?;
    if target.as_os_str().is_empty()
        || target
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    let file_name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?;
    if file_name == "." || file_name == ".." {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }

    let parent = target
        .parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                parent.to_path_buf()
            }
        })
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?;
    if let Ok(metadata) = fs::symlink_metadata(&parent) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(anyhow!("config_init_secret_path_invalid"));
        }
    }

    let prior = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(anyhow!("config_init_secret_path_invalid"));
            }
            let bytes =
                fs::read(&target).map_err(|_| anyhow!("config_init_secret_path_invalid"))?;
            PriorSecretState::Existing {
                bytes,
                mode: metadata.permissions().mode() & 0o7777,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PriorSecretState::Absent,
        Err(_) => return Err(anyhow!("config_init_secret_path_invalid")),
    };

    Ok((target, parent, prior))
}

fn atomically_write_secret(target: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?
        .to_string_lossy();

    let mut temporary = None;
    for _ in 0..SECRET_TEMP_ATTEMPTS {
        let counter = SECRET_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.agentic-gpt-tmp-{}-{counter}",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(anyhow!("config_init_secret_write_failed")),
        }
    }

    let (temporary_path, mut file) =
        temporary.ok_or_else(|| anyhow!("config_init_secret_temp_unavailable"))?;
    let mut guard = TemporarySecretFile {
        path: Some(temporary_path.clone()),
    };
    use std::io::Write;
    file.write_all(bytes)
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    file.sync_all()
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    drop(file);
    fs::rename(&temporary_path, target).map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    sync_parent(parent).map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    guard.path = None;
    Ok(())
}

fn sync_parent(parent: &Path) -> Result<()> {
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::model::OptionalSectionDraft;
    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::{OptionalSection, RuntimeMode, SecretValue};
    use crate::WorkerProfile;
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;

    fn fresh_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agentic-gpt-config-setup-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn outcome_with_secret(config_path: &Path, secret_path: &Path, value: &str) -> WizardOutcome {
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Normal),
                tunnel_id: Some("tunnel-test".to_string()),
                tunnel_api_key: Some(format!("file:{}", secret_path.display())),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            config_path.to_path_buf(),
        );
        session.standalone_mut().provision_secret_now = true;
        session.standalone_mut().secret_value = Some(SecretValue::new(value));
        session.into_wizard_outcome().unwrap()
    }
    fn staged_recovery_fixture(label: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = fresh_root(label);
        let config_path = root.join("config.json");
        let secret_path = root.join("secret");
        let backup_path = root.join(".secret.setup-backup");
        let mut old_config = crate::config::Config::default_config().unwrap();
        old_config.display_name = "old".to_string();
        crate::config::write_config_with_backup(&config_path, &old_config).unwrap();
        let old_config_hash = bytes_hash(&fs::read(&config_path).unwrap());
        let mut new_config = old_config.clone();
        new_config.display_name = "new".to_string();
        let new_config_hash =
            bytes_hash(sparse_config_json(&new_config, false).unwrap().as_bytes());

        fs::write(&secret_path, b"old-secret").unwrap();
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o640)).unwrap();
        atomically_write_secret(&backup_path, b"old-secret", 0o640).unwrap();
        let journal_path = setup_journal_path(&config_path).unwrap();
        let journal = SetupJournal {
            secret_path: secret_path.clone(),
            backup_path: Some(backup_path.clone()),
            old_config_hash: Some(old_config_hash),
            new_config_hash,
            old_secret_hash: Some(bytes_hash(b"old-secret")),
            new_secret_hash: bytes_hash(b"new-secret"),
            old_secret_mode: Some(0o640),
        };
        stage_setup_journal(&journal_path, &journal).unwrap();
        atomically_write_secret(&secret_path, b"new-secret", 0o600).unwrap();
        (root, config_path, secret_path, journal_path, backup_path)
    }

    #[test]
    fn config_load_does_not_recover_setup_while_mutation_lock_is_held() {
        let (root, config_path, secret_path, journal_path, backup_path) =
            staged_recovery_fixture("locked-load");
        let lock = crate::config::acquire_config_mutation_lock(&config_path).unwrap();
        let path_for_thread = config_path.clone();
        let handle = std::thread::spawn(move || crate::config::Config::load(&path_for_thread));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(fs::read(&secret_path).unwrap(), b"new-secret");
        assert!(journal_path.exists());
        drop(lock);

        handle.join().unwrap().unwrap();
        assert_eq!(fs::read(&secret_path).unwrap(), b"old-secret");
        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(!journal_path.exists());
        assert!(!backup_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_crash_state_and_retains_conflicting_evidence() {
        let (root, config_path, secret_path, journal_path, backup_path) =
            staged_recovery_fixture("restart-recovery");
        crate::config::Config::load(&config_path).unwrap();
        assert_eq!(fs::read(&secret_path).unwrap(), b"old-secret");
        assert!(!journal_path.exists());
        assert!(!backup_path.exists());
        let _ = fs::remove_dir_all(root);

        let (root, config_path, secret_path, journal_path, backup_path) =
            staged_recovery_fixture("recovery-conflict");
        fs::write(&config_path, b"external-edit").unwrap();
        let error = recover_pending_setup_locked(&config_path).unwrap_err();
        assert_eq!(error.to_string(), "config_init_recovery_conflict");
        assert_eq!(fs::read(&secret_path).unwrap(), b"new-secret");
        assert!(journal_path.exists());
        assert!(backup_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_creates_secret_parent_0700_file_0600_and_config() {
        let root = fresh_root("permissions-create");
        let config_path = root.join("config").join("config.json");
        let secret_path = root.join("secrets").join("tunnel-api-key");

        commit_wizard_outcome(
            &config_path,
            outcome_with_secret(&config_path, &secret_path, "permission-secret-marker"),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(secret_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let config = crate::config::Config::load(&config_path).unwrap();
        assert_eq!(
            config.tunnel.as_ref().unwrap().api_key,
            format!("file:{}", secret_path.display())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_replacement_restores_existing_mode_and_bytes_on_config_failure() {
        let root = fresh_root("rollback-existing");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
        fs::write(&secret_path, b"old-secret").unwrap();
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o640)).unwrap();

        let blocker = root.join("config-blocker");
        fs::write(&blocker, b"not-a-directory").unwrap();
        let config_path = blocker.join("config.json");
        let error = match commit_wizard_outcome(
            &config_path,
            outcome_with_secret(&config_path, &secret_path, "replacement-secret"),
        ) {
            Ok(_) => panic!("config write unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_config_write_failed");
        assert_eq!(fs::read(&secret_path).unwrap(), b"old-secret");
        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_failure_removes_new_secret_and_invalid_target_has_no_side_effect() {
        let root = fresh_root("rollback-absent");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        let blocker = root.join("config-blocker");
        fs::write(&blocker, b"not-a-directory").unwrap();
        let config_path = blocker.join("config.json");
        let error = match commit_wizard_outcome(
            &config_path,
            outcome_with_secret(&config_path, &secret_path, "new-secret"),
        ) {
            Ok(_) => panic!("config write unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_config_write_failed");
        assert!(!secret_path.exists());

        let invalid_config = root.join("invalid-config.json");
        let invalid = outcome_with_secret(&invalid_config, &invalid_config, "invalid-target");
        let error = match commit_wizard_outcome(&invalid_config, invalid) {
            Ok(_) => panic!("invalid secret target unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_secret_path_invalid");
        assert!(!invalid_config.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_secret_outcome_writes_config_without_secret_material() {
        let root = fresh_root("no-secret");
        let config_path = root.join("config.json");
        let session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                profile: Some(WorkerProfile::Normal),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            config_path.clone(),
        );
        let outcome = session.into_wizard_outcome().unwrap();
        assert!(outcome.secret_write.is_none());
        commit_wizard_outcome(&config_path, outcome).unwrap();
        assert!(config_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imported_config_review_edit_commits_events_and_keeps_backup() {
        let root = fresh_root("import-backup");
        let config_path = root.join("config.json");
        let old = serde_json::json!({
            "mode": "local",
            "profile": "normal",
            "hubUrl": "https://legacy.example.com",
            "hubTransport": "sse",
            "agentSecret": "legacy-secret",
            "futureField": {"keep": true},
            "events": {
                "lowTtlSeconds": 7200,
                "internalOverrides": {
                    "process.failed": "high"
                }
            },
        });
        let old_bytes = serde_json::to_vec_pretty(&old).unwrap();
        fs::write(&config_path, &old_bytes).unwrap();
        let imported = crate::config::Config::import(&config_path).unwrap();
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                profile: Some(WorkerProfile::Normal),
                imported_base: Some(imported.config),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            config_path.clone(),
        );
        let mut events_draft = session.optional_draft(OptionalSection::Events);
        let OptionalSectionDraft::Events(events) = &mut events_draft else {
            panic!("imported events configuration was not reviewable");
        };
        assert_eq!(events.low_ttl_seconds, "7200");
        assert_eq!(
            events
                .internal_overrides
                .get("process.failed")
                .map(String::as_str),
            Some("high")
        );
        events.low_ttl_seconds = "0".to_string();
        events
            .internal_overrides
            .insert("process.completed".to_string(), "off".to_string());
        session
            .save_optional_section_for_review(events_draft)
            .unwrap();
        let review = session.review_model().unwrap();
        let event_group = review
            .optional_sections
            .iter()
            .find(|group| {
                group
                    .items
                    .iter()
                    .any(|item| item.field == Some(SetupField::EventProcessCompletedLevel))
            })
            .unwrap();
        let completed = event_group
            .items
            .iter()
            .find(|item| item.field == Some(SetupField::EventProcessCompletedLevel))
            .unwrap();
        assert_eq!(completed.value, "off");
        assert_eq!(
            completed.choice_values(),
            &["inherit", "low", "medium", "high", "off"]
        );
        commit_wizard_outcome(&config_path, session.into_wizard_outcome().unwrap()).unwrap();

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        assert_eq!(written["events"]["lowTtlSeconds"], 0);
        assert_eq!(
            written["events"]["internalOverrides"]["process.completed"],
            "off"
        );
        assert_eq!(
            written["events"]["internalOverrides"]["process.failed"],
            "high"
        );
        assert_eq!(written["hub"]["url"], "https://legacy.example.com");
        assert_eq!(written["hub"]["transport"], "sse");
        assert_eq!(written["futureField"]["keep"], true);
        let backups = fs::read_dir(root.join("backups"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].path()).unwrap(), old_bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn aliased_config_and_secret_paths_are_rejected_before_secret_write() {
        let root = fresh_root("alias-collision");
        let config_path = root.join(".").join("config.json");
        let secret_path = root.join("config.json");
        fs::write(&config_path, b"existing-config").unwrap();
        let outcome = outcome_with_secret(&config_path, &secret_path, "alias-secret-marker");

        let error = match commit_wizard_outcome(&config_path, outcome) {
            Ok(_) => panic!("aliased config/secret target unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_secret_path_invalid");
        assert_eq!(fs::read(&secret_path).unwrap(), b"existing-config");
        let backup_dir = root.join("backups");
        if backup_dir.exists() {
            let backups = fs::read_dir(backup_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .collect::<Vec<_>>();
            assert!(backups.iter().all(|entry| {
                fs::read(entry.path())
                    .map(|bytes| {
                        !bytes
                            .windows("alias-secret-marker".len())
                            .any(|window| window == b"alias-secret-marker")
                    })
                    .unwrap_or(true)
            }));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn symlink_parent_alias_to_nonexistent_target_is_rejected_before_secret_write() {
        let root = fresh_root("symlink-parent-alias");
        let real = root.join("real");
        let alias = root.join("alias");
        fs::create_dir_all(&real).unwrap();
        symlink("real", &alias).unwrap();

        let config_path = alias.join("config.json");
        let secret_path = real.join("config.json");
        let marker = "symlink-parent-secret-marker";
        let error = match commit_wizard_outcome(
            &config_path,
            outcome_with_secret(&config_path, &secret_path, marker),
        ) {
            Ok(_) => panic!("symlink-parent alias unexpectedly succeeded"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "config_init_secret_path_invalid");
        assert!(!config_path.exists());
        assert!(!secret_path.exists());
        let backup_dir = real.join("backups");
        if backup_dir.exists() {
            assert!(fs::read_dir(&backup_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .all(|entry| {
                    fs::read(entry.path())
                        .map(|bytes| {
                            !bytes
                                .windows(marker.len())
                                .any(|window| window == marker.as_bytes())
                        })
                        .unwrap_or(true)
                }));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn outcome_handoff_revalidates_canonical_connection_before_any_write_plan() {
        let root = fresh_root("canonical-validation");
        let config_path = root.join("config.json");
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                profile: Some(WorkerProfile::Normal),
                hub_url: Some("ftp://invalid.example.com".to_string()),
                hub_transport: Some("websocket".to_string()),
                agent_id: Some("desk".to_string()),
                agent_secret: Some(SecretValue::new("canonical-secret")),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            config_path,
        );
        session.hub_mut().hub_url = "ftp://invalid.example.com".to_string();
        let errors = match session.into_wizard_outcome() {
            Ok(_) => panic!("invalid Hub URL unexpectedly reached outcome"),
            Err(errors) => errors,
        };
        assert_eq!(errors[0].field, SetupField::HubUrl);
        assert_eq!(errors[0].code, "hub_url_invalid");
        let _ = fs::remove_dir_all(root);
    }
}
