use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::utils::agentic_home;

#[derive(Clone, Debug)]
pub(crate) struct PrivateStatePaths {
    pub(crate) root: PathBuf,
    pub(crate) active_skills: PathBuf,
    pub(crate) skill_installs: PathBuf,
}

pub(crate) fn prepare() -> Result<PrivateStatePaths> {
    prepare_at(&agentic_home()?)
}

fn prepare_at(agentic_root: &Path) -> Result<PrivateStatePaths> {
    let root = agentic_root.join("state");
    ensure_private_dir(&root)?;
    sync_dir(&root)?;
    Ok(PrivateStatePaths {
        active_skills: root.join("active-skills.json"),
        skill_installs: root.join("skill-installs"),
        root,
    })
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(anyhow!("private_state_directory_symlink"));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(anyhow!("private_state_directory_unavailable"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::create_dir_all(path).map_err(|_| anyhow!("private_state_directory_unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow!("private_state_directory_unavailable"))?;
    }
    Ok(())
}

#[cfg(test)]
impl PrivateStatePaths {
    pub(crate) fn for_test(root: PathBuf) -> Self {
        ensure_private_dir(&root).expect("test private-state root should be creatable");
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

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "agentic-private-state-{name}-{}",
                uuid::Uuid::new_v4().simple()
            )))
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn prepare_keeps_shared_state_and_leaves_legacy_state_untouched() {
        let root = TestRoot::new("shared");
        let agentic = root.0.join("home/.agentic_gpt");
        let workspace_state = root.0.join("workspace/state");
        let old_agent_state = agentic.join("state/agent/old-agent");
        fs::create_dir_all(&workspace_state).unwrap();
        fs::create_dir_all(&old_agent_state).unwrap();
        fs::write(workspace_state.join("active-skills.json"), b"workspace").unwrap();
        fs::write(old_agent_state.join("active-skills.json"), b"old-agent").unwrap();

        let first = prepare_at(&agentic).unwrap();
        fs::write(&first.active_skills, b"shared").unwrap();
        let second = prepare_at(&agentic).unwrap();

        assert_eq!(first.root, agentic.join("state"));
        assert_eq!(first.active_skills, second.active_skills);
        assert_eq!(fs::read(second.active_skills).unwrap(), b"shared");
        assert_eq!(
            fs::read(workspace_state.join("active-skills.json")).unwrap(),
            b"workspace"
        );
        assert_eq!(
            fs::read(old_agent_state.join("active-skills.json")).unwrap(),
            b"old-agent"
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepare_rejects_symlink_root_without_writing_the_target() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("symlink");
        let agentic = root.0.join("home/.agentic_gpt");
        let outside = root.0.join("outside");
        fs::create_dir_all(&agentic).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("active-skills.json"), b"outside").unwrap();
        symlink(&outside, agentic.join("state")).unwrap();

        let error = prepare_at(&agentic).unwrap_err();

        assert_eq!(error.to_string(), "private_state_directory_symlink");
        assert_eq!(
            fs::read(outside.join("active-skills.json")).unwrap(),
            b"outside"
        );
    }

    #[test]
    fn prepare_fails_when_state_root_is_a_file() {
        let root = TestRoot::new("unavailable");
        let agentic = root.0.join("home/.agentic_gpt");
        fs::create_dir_all(&agentic).unwrap();
        fs::write(agentic.join("state"), b"occupied").unwrap();

        let error = prepare_at(&agentic).unwrap_err();

        assert_eq!(error.to_string(), "private_state_directory_unavailable");
        assert_eq!(fs::read(agentic.join("state")).unwrap(), b"occupied");
    }

    #[cfg(unix)]
    #[test]
    fn prepare_enforces_mode_0700_on_existing_shared_root() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestRoot::new("permissions");
        let agentic = root.0.join("home/.agentic_gpt");
        let state = agentic.join("state");
        fs::create_dir_all(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(state.join("active-skills.json"), b"existing").unwrap();

        let paths = prepare_at(&agentic).unwrap();

        assert_eq!(
            fs::metadata(&paths.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::read(paths.active_skills).unwrap(), b"existing");
    }
}
