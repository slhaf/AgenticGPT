use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{anyhow, Result};

use crate::config::write_config_with_backup;
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

static SECRET_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const SECRET_TEMP_ATTEMPTS: usize = 128;

pub(crate) fn commit_wizard_outcome(
    config_path: &Path,
    outcome: WizardOutcome,
) -> Result<InitSummary> {
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

    atomically_write_secret(&target, plan.value.expose().as_bytes(), 0o600)
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;

    if write_config_with_backup(config_path, &build.config).is_err() {
        let rollback_result = match prior {
            PriorSecretState::Absent => match fs::remove_file(&target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(anyhow!("config_init_secret_rollback_failed")),
            },
            PriorSecretState::Existing { bytes, mode } => {
                atomically_write_secret(&target, &bytes, mode)
            }
        };
        return match rollback_result {
            Ok(()) => Err(anyhow!("config_init_config_write_failed")),
            Err(_) => Err(anyhow!(
                "config_init_config_write_failed: config_init_secret_rollback_failed"
            )),
        };
    }

    Ok(summary)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool> {
    let left = lexical_absolute(&crate::exec::expand_pathbuf(left)?)?;
    let right = lexical_absolute(&crate::exec::expand_pathbuf(right)?)?;
    if left == right {
        return Ok(true);
    }
    Ok(match (fs::canonicalize(&left), fs::canonicalize(&right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    })
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
    let target = crate::exec::expand_pathbuf(path)
        .map_err(|_| anyhow!("config_init_secret_path_invalid"))?;
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
    guard.path = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::{RuntimeMode, SecretValue};
    use crate::WorkerProfile;
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
