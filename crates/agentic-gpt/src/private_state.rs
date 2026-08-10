use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;
use crate::utils::agentic_home;

#[derive(Clone, Debug)]
pub(crate) struct PrivateStatePaths {
    // Kept as the authoritative per-agent root for subsequent private-state consumers.
    #[allow(dead_code)]
    pub(crate) root: PathBuf,
    pub(crate) active_skills: PathBuf,
    pub(crate) skill_installs: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct PrepareOutcome {
    pub(crate) paths: PrivateStatePaths,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn prepare(config: &Config) -> Result<PrepareOutcome> {
    prepare_at(config, &agentic_home()?)
}

fn prepare_at(config: &Config, agentic_root: &Path) -> Result<PrepareOutcome> {
    let agent_key = agent_state_key(&config.agent_id)?;
    let root = agentic_root.join("state").join("agent").join(agent_key);
    let legacy_root = config.workspace_root.join("state");
    let legacy_active = legacy_root.join("active-skills.json");
    let legacy_installs = legacy_root.join("skill-installs");
    let target_active = root.join("active-skills.json");
    let target_installs = root.join("skill-installs");
    let mut warnings = Vec::new();

    if let Err(error) = ensure_private_dir(&root) {
        warnings.push(format!("private_state_root_unavailable: {error}"));
        return Ok(PrepareOutcome {
            paths: PrivateStatePaths {
                root,
                active_skills: legacy_active,
                skill_installs: legacy_installs,
            },
            warnings,
        });
    }

    let active_skills = migrate_file(
        "active-skills",
        &legacy_active,
        &target_active,
        &mut warnings,
    );
    let skill_installs = migrate_directory(
        "skill-installs",
        &legacy_installs,
        &target_installs,
        &mut warnings,
    );

    if let Err(error) = fs::remove_dir(&legacy_root) {
        if error.kind() != std::io::ErrorKind::NotFound
            && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
        {
            warnings.push(format!("private_state_legacy_root_cleanup_failed: {error}"));
        }
    }

    Ok(PrepareOutcome {
        paths: PrivateStatePaths {
            root,
            active_skills,
            skill_installs,
        },
        warnings,
    })
}

fn migrate_file(label: &str, source: &Path, target: &Path, warnings: &mut Vec<String>) -> PathBuf {
    if target.exists() {
        if source.exists() {
            if verify_file(source, target).is_ok() {
                if let Err(error) = fs::remove_file(source) {
                    warnings.push(format!(
                        "private_state_legacy_cleanup_failed: {label}: {error}"
                    ));
                }
            } else {
                warnings.push(format!(
                    "private_state_conflict: {label} target already exists; legacy source retained"
                ));
            }
        }
        return target.to_path_buf();
    }
    if !source.exists() {
        return target.to_path_buf();
    }

    let result = (|| -> Result<()> {
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(anyhow!("legacy_source_not_regular_file"));
        }
        let parent = target
            .parent()
            .ok_or_else(|| anyhow!("private_state_parent_missing"))?;
        ensure_private_dir(parent)?;
        let tmp = parent.join(format!(".{label}-migrate-{}.tmp", Uuid::new_v4().simple()));
        let cleanup = tmp.clone();
        let attempt = (|| -> Result<()> {
            fs::copy(source, &tmp)?;
            set_private_file(&tmp)?;
            verify_file(source, &tmp)?;
            fs::rename(&tmp, target)?;
            Ok(())
        })();
        if attempt.is_err() {
            let _ = fs::remove_file(cleanup);
        }
        attempt?;
        if let Err(error) = fs::remove_file(source) {
            warnings.push(format!(
                "private_state_legacy_cleanup_failed: {label}: {error}"
            ));
        }
        Ok(())
    })();

    match result {
        Ok(()) => target.to_path_buf(),
        Err(error) => {
            warnings.push(format!("private_state_migration_failed: {label}: {error}"));
            source.to_path_buf()
        }
    }
}

fn migrate_directory(
    label: &str,
    source: &Path,
    target: &Path,
    warnings: &mut Vec<String>,
) -> PathBuf {
    if target.exists() {
        if source.exists() {
            if verify_tree(source, target).is_ok() {
                if let Err(error) = fs::remove_dir_all(source) {
                    warnings.push(format!(
                        "private_state_legacy_cleanup_failed: {label}: {error}"
                    ));
                }
            } else {
                warnings.push(format!(
                    "private_state_conflict: {label} target already exists; legacy source retained"
                ));
            }
        }
        return target.to_path_buf();
    }
    if !source.exists() {
        return target.to_path_buf();
    }

    let result = (|| -> Result<()> {
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(anyhow!("legacy_source_not_directory"));
        }
        let parent = target
            .parent()
            .ok_or_else(|| anyhow!("private_state_parent_missing"))?;
        ensure_private_dir(parent)?;
        let tmp = parent.join(format!(".{label}-migrate-{}", Uuid::new_v4().simple()));
        let attempt = (|| -> Result<()> {
            copy_tree(source, &tmp)?;
            verify_tree(source, &tmp)?;
            fs::rename(&tmp, target)?;
            Ok(())
        })();
        if attempt.is_err() {
            let _ = fs::remove_dir_all(&tmp);
        }
        attempt?;
        if let Err(error) = fs::remove_dir_all(source) {
            warnings.push(format!(
                "private_state_legacy_cleanup_failed: {label}: {error}"
            ));
        }
        Ok(())
    })();

    match result {
        Ok(()) => target.to_path_buf(),
        Err(error) => {
            warnings.push(format!("private_state_migration_failed: {label}: {error}"));
            source.to_path_buf()
        }
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!("migration_tree_invalid"));
    }
    fs::create_dir(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("migration_symlink_rejected"));
        }
        if metadata.is_dir() {
            copy_tree(&source_path, &target_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &target_path)?;
            fs::set_permissions(&target_path, metadata.permissions())?;
        } else {
            return Err(anyhow!("migration_special_file_rejected"));
        }
    }
    Ok(())
}

fn verify_tree(source: &Path, target: &Path) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    let target_metadata = fs::symlink_metadata(target)?;
    if source_metadata.file_type().is_symlink()
        || target_metadata.file_type().is_symlink()
        || !source_metadata.is_dir()
        || !target_metadata.is_dir()
    {
        return Err(anyhow!("migration_verification_mismatch"));
    }
    let mut source_names = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut target_names = fs::read_dir(target)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    source_names.sort();
    target_names.sort();
    if source_names != target_names {
        return Err(anyhow!("migration_verification_mismatch"));
    }
    for name in source_names {
        let source_path = source.join(&name);
        let target_path = target.join(&name);
        let source_metadata = fs::symlink_metadata(&source_path)?;
        let target_metadata = fs::symlink_metadata(&target_path)?;
        if source_metadata.file_type().is_symlink() || target_metadata.file_type().is_symlink() {
            return Err(anyhow!("migration_symlink_rejected"));
        }
        if source_metadata.is_dir() && target_metadata.is_dir() {
            verify_tree(&source_path, &target_path)?;
        } else if source_metadata.is_file() && target_metadata.is_file() {
            verify_file(&source_path, &target_path)?;
        } else {
            return Err(anyhow!("migration_verification_mismatch"));
        }
    }
    Ok(())
}

fn verify_file(source: &Path, target: &Path) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    let target_metadata = fs::symlink_metadata(target)?;
    if source_metadata.file_type().is_symlink()
        || target_metadata.file_type().is_symlink()
        || !source_metadata.is_file()
        || !target_metadata.is_file()
        || source_metadata.len() != target_metadata.len()
        || fs::read(source)? != fs::read(target)?
    {
        return Err(anyhow!("migration_verification_mismatch"));
    }
    Ok(())
}

fn agent_state_key(agent_id: &str) -> Result<String> {
    if agent_id.trim().is_empty() {
        return Err(anyhow!("private_state_agent_id_required"));
    }
    if agent_id.len() <= 128
        && agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Ok(agent_id.to_string());
    }
    let digest = Sha256::digest(agent_id.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("id-{hex}"))
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|_| anyhow!("private_state_directory_unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow!("private_state_directory_unavailable"))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| anyhow!("private_state_file_unavailable"))?;
    }
    Ok(())
}

#[cfg(test)]
impl PrivateStatePaths {
    pub(crate) fn for_test(root: PathBuf) -> Self {
        Self {
            active_skills: root.join("active-skills.json"),
            skill_installs: root.join("skill-installs"),
            root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-private-state-{name}-{}",
            Uuid::new_v4().simple()
        ))
    }

    fn config_with_workspace(workspace: PathBuf) -> Config {
        let mut config = Config::default_config().unwrap();
        config.agent_id = "test-agent".to_string();
        config.workspace_root = workspace;
        config
    }

    #[test]
    fn prepare_migrates_known_legacy_state_and_cleans_empty_root() {
        let root = temp_root("migrate");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        fs::create_dir_all(workspace.join("state/skill-installs/staging/install-1")).unwrap();
        fs::write(
            workspace.join("state/active-skills.json"),
            br#"{"activeSkills":[]}"#,
        )
        .unwrap();
        fs::write(
            workspace.join("state/skill-installs/install-1.json"),
            b"record",
        )
        .unwrap();
        fs::write(
            workspace.join("state/skill-installs/staging/install-1/file.txt"),
            b"payload",
        )
        .unwrap();
        let config = config_with_workspace(workspace.clone());

        let outcome = prepare_at(&config, &agentic).unwrap();

        assert!(outcome.warnings.is_empty());
        assert!(outcome.paths.active_skills.is_file());
        assert!(outcome
            .paths
            .skill_installs
            .join("install-1.json")
            .is_file());
        assert!(outcome
            .paths
            .skill_installs
            .join("staging/install-1/file.txt")
            .is_file());
        assert!(!workspace.join("state").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_is_idempotent_after_successful_migration() {
        let root = temp_root("idempotent");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        fs::create_dir_all(workspace.join("state")).unwrap();
        fs::write(workspace.join("state/active-skills.json"), b"state").unwrap();
        let config = config_with_workspace(workspace);

        let first = prepare_at(&config, &agentic).unwrap();
        let second = prepare_at(&config, &agentic).unwrap();

        assert!(first.warnings.is_empty());
        assert!(second.warnings.is_empty());
        assert_eq!(first.paths.active_skills, second.paths.active_skills);
        assert_eq!(fs::read(second.paths.active_skills).unwrap(), b"state");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn identical_target_and_legacy_source_cleanup_is_self_healing() {
        let root = temp_root("identical");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        let target = agentic.join("state/agent/test-agent");
        fs::create_dir_all(workspace.join("state")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(workspace.join("state/active-skills.json"), b"same").unwrap();
        fs::write(target.join("active-skills.json"), b"same").unwrap();
        let config = config_with_workspace(workspace.clone());

        let outcome = prepare_at(&config, &agentic).unwrap();

        assert!(outcome.warnings.is_empty());
        assert!(!workspace.join("state/active-skills.json").exists());
        assert_eq!(fs::read(outcome.paths.active_skills).unwrap(), b"same");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn target_conflict_keeps_target_and_retains_legacy_source() {
        let root = temp_root("conflict");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        let target = agentic.join("state/agent/test-agent");
        fs::create_dir_all(workspace.join("state")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(workspace.join("state/active-skills.json"), b"legacy").unwrap();
        fs::write(target.join("active-skills.json"), b"target").unwrap();
        let config = config_with_workspace(workspace.clone());

        let outcome = prepare_at(&config, &agentic).unwrap();

        assert_eq!(fs::read(&outcome.paths.active_skills).unwrap(), b"target");
        assert_eq!(
            fs::read(workspace.join("state/active-skills.json")).unwrap(),
            b"legacy"
        );
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.starts_with("private_state_conflict: active-skills")));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlink_and_falls_back_to_legacy() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        fs::create_dir_all(workspace.join("state/skill-installs")).unwrap();
        fs::write(root.join("outside"), b"secret").unwrap();
        symlink(
            root.join("outside"),
            workspace.join("state/skill-installs/link"),
        )
        .unwrap();
        let config = config_with_workspace(workspace.clone());

        let outcome = prepare_at(&config, &agentic).unwrap();

        assert_eq!(
            outcome.paths.skill_installs,
            workspace.join("state/skill-installs")
        );
        assert!(workspace.join("state/skill-installs/link").exists());
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.starts_with("private_state_migration_failed: skill-installs")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_wide_agent_id_uses_stable_safe_state_key() {
        let root = temp_root("wide-id");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        let mut config = config_with_workspace(workspace);
        config.agent_id = "hub/team A".to_string();

        let first = prepare_at(&config, &agentic).unwrap();
        let second = prepare_at(&config, &agentic).unwrap();

        assert_eq!(first.paths.root, second.paths.root);
        assert_eq!(
            first.paths.root.parent(),
            Some(agentic.join("state/agent").as_path())
        );
        let key = first.paths.root.file_name().unwrap().to_string_lossy();
        assert!(key.starts_with("id-"));
        assert!(key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn private_agent_root_is_mode_0700() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("permissions");
        let workspace = root.join("workspace");
        let agentic = root.join("home");
        let config = config_with_workspace(workspace);

        let outcome = prepare_at(&config, &agentic).unwrap();
        let mode = fs::metadata(&outcome.paths.root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        let _ = fs::remove_dir_all(root);
    }
}
