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
