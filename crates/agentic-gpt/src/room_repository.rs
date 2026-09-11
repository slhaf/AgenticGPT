use crate::{config::Config, exec};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
use chrono_tz::Asia::Shanghai;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Maximum data retained from a Git subprocess. Git is never allowed to make a
/// user-facing error or status response unbounded.
pub(crate) const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024;
/// Maximum Markdown payload retained by the reusable Room read helper.
pub(crate) const MAX_MARKDOWN_BYTES: usize = 512 * 1024;
const MAX_ROOM_JSON_BYTES: usize = 16 * 1024;
const GIT_COMMAND_TIMEOUT: StdDuration = StdDuration::from_secs(10);

pub(crate) const ROOM_SCHEMA_VERSION: u64 = 1;
const INITIAL_COMMIT_MESSAGE: &str = "Initialize Room repository scaffold";
const SCAFFOLD_DATE_TOKEN: &str = "{{LOGICAL_DATE}}";

const SCAFFOLD_FILES: &[(&str, &str)] = &[
    (
        "room.json",
        include_str!("../assets/room-scaffold/room.json"),
    ),
    (
        "Diary/Daily/current.md",
        include_str!("../assets/room-scaffold/Diary/Daily/current.md"),
    ),
    (
        "Diary/Weekly/current.md",
        include_str!("../assets/room-scaffold/Diary/Weekly/current.md"),
    ),
    (
        "Diary/Monthly/current.md",
        include_str!("../assets/room-scaffold/Diary/Monthly/current.md"),
    ),
    (
        "Notebook/.gitkeep",
        include_str!("../assets/room-scaffold/Notebook/.gitkeep"),
    ),
    (
        "State/entities/.gitkeep",
        include_str!("../assets/room-scaffold/State/entities/.gitkeep"),
    ),
    (
        "manual/diary.md",
        include_str!("../assets/room-scaffold/manual/diary.md"),
    ),
    (
        "manual/notebook.md",
        include_str!("../assets/room-scaffold/manual/notebook.md"),
    ),
    (
        "manual/entity.md",
        include_str!("../assets/room-scaffold/manual/entity.md"),
    ),
    (
        "maintenance/diary/daily/.gitkeep",
        include_str!("../assets/room-scaffold/maintenance/diary/daily/.gitkeep"),
    ),
    (
        "maintenance/diary/weekly/.gitkeep",
        include_str!("../assets/room-scaffold/maintenance/diary/weekly/.gitkeep"),
    ),
    (
        "maintenance/diary/monthly/.gitkeep",
        include_str!("../assets/room-scaffold/maintenance/diary/monthly/.gitkeep"),
    ),
    (
        "maintenance/notebook/.gitkeep",
        include_str!("../assets/room-scaffold/maintenance/notebook/.gitkeep"),
    ),
    (
        "maintenance/entity/.gitkeep",
        include_str!("../assets/room-scaffold/maintenance/entity/.gitkeep"),
    ),
    (
        "scripts/apply_maintenance.py",
        include_str!("../assets/room-scaffold/scripts/apply_maintenance.py"),
    ),
    (
        ".github/workflows/apply-maintenance.yml",
        include_str!("../assets/room-scaffold/.github/workflows/apply-maintenance.yml"),
    ),
];

const SCAFFOLD_DIRECTORIES: &[&str] = &[
    "Diary",
    "Diary/Daily",
    "Diary/Weekly",
    "Diary/Monthly",
    "Notebook",
    "State",
    "State/entities",
    "manual",
    "maintenance",
    "maintenance/diary",
    "maintenance/diary/daily",
    "maintenance/diary/weekly",
    "maintenance/diary/monthly",
    "maintenance/notebook",
    "maintenance/entity",
    "scripts",
    ".github",
    ".github/workflows",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Readiness {
    /// The corresponding capability is present and usable.
    Ready,
    /// The root or capability has not been created.
    Missing,
    /// The capability exists but does not satisfy the Room contract.
    Incomplete,
    /// The capability could not be inspected safely or locally.
    Unavailable,
    /// A versioned capability exists but is not supported by this Agent.
    Outdated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositoryStatus {
    pub(crate) root: PathBuf,
    /// True only when Git reports this exact root as its top level.
    pub(crate) repository_initialized: bool,
    /// Kept separate from schema/control-plane readiness on purpose.
    pub(crate) top_level_initialized: bool,
    pub(crate) branch: Option<String>,
    pub(crate) head: Option<String>,
    pub(crate) clean: Option<bool>,
    pub(crate) schema_version: Option<u64>,
    pub(crate) schema: Readiness,
    pub(crate) scaffold: Readiness,
    pub(crate) control_plane: Readiness,
    pub(crate) local_executor: Readiness,
    pub(crate) workflow: Readiness,
    pub(crate) remote: Readiness,
    pub(crate) sync: Readiness,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedText {
    pub(crate) content: String,
    pub(crate) truncated: bool,
}

/// The bounded, user-facing portion of a Git subprocess result.
///
/// stderr is deliberately discarded here. Callers must map failures to a
/// stable semantic error instead of exposing repository-controlled output.
#[derive(Debug)]
pub(crate) struct BoundedGitOutput {
    pub(crate) success: bool,
    pub(crate) stdout: Vec<u8>,
}

#[derive(Debug)]
struct GitOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Return the effective Room root without touching the filesystem.
///
/// Configured values are expanded, normalized to an absolute path, and made
/// relative to the configured workspace when they are not absolute. The
/// caller still needs `ensure_repository` or `inspect_repository` to reject
/// symlink components before using the path as a semantic boundary.
pub(crate) fn repository_root(config: &Config) -> PathBuf {
    let configured = config
        .room
        .repository_root
        .as_ref()
        .and_then(|path| exec::expand_pathbuf(path).ok());
    let candidate = configured
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                config.workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| config.workspace_root.join("room"));
    absolute_normalize(&candidate).unwrap_or(candidate)
}

/// All semantic repository-relative paths pass through this validator. In
/// particular, platform separators, absolute paths, and dot components are
/// never accepted at a semantic boundary.
pub(crate) fn validate_repository_relative(value: &str) -> Result<()> {
    if value.is_empty() || value.contains('\0') || value.contains('\\') {
        return Err(anyhow!("room_repository_path_invalid"));
    }
    let mut components = Path::new(value).components();
    let Some(Component::Normal(_)) = components.next() else {
        return Err(anyhow!("room_repository_path_invalid"));
    };
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(anyhow!("room_repository_path_invalid"));
    }
    Ok(())
}

/// Return a path under `root` only when it is a validated, non-symlink path.
pub(crate) fn repository_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_repository_relative(relative)?;
    let root_metadata = fs::symlink_metadata(root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(anyhow!("room_repository_root_not_directory"));
    }
    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().copied().enumerate() {
        let Component::Normal(part) = component else {
            return Err(anyhow!("room_repository_path_invalid"));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(anyhow!("room_repository_symlink_escape"));
            }
            Ok(metadata) if !metadata.is_dir() && index + 1 != components.len() => {
                return Err(anyhow!("room_repository_path_not_directory"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

/// Compute a repository-relative path after rejecting escapes and symlinked
/// path components. This is intentionally independent of `Path::display` so
/// callers receive stable `/` separators on every platform.
#[cfg(test)]
pub(crate) fn repository_relative(root: &Path, path: &Path) -> Result<String> {
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(anyhow!("room_repository_path_invalid"));
    }
    let root = checked_existing_directory(root)?;
    let path = absolute_normalize(path)?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| anyhow!("room_path_outside_repository"))?;
    let value = relative
        .to_str()
        .ok_or_else(|| anyhow!("room_path_invalid"))?;
    validate_repository_relative(value)?;
    let _ = repository_path(&root, value)?;
    Ok(value.replace('\\', "/"))
}

/// Read a Markdown file below a repository root with a hard byte bound.
pub(crate) fn read_bounded_markdown(root: &Path, relative: &str) -> Result<BoundedText> {
    validate_repository_relative(relative)?;
    if Path::new(relative)
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("md")
    {
        return Err(anyhow!("room_markdown_path_required"));
    }
    let path = repository_path(root, relative)?;
    read_bounded_text(&path, MAX_MARKDOWN_BYTES)
}

/// Read a regular, non-symlink file while retaining at most `max_bytes`.
pub(crate) fn read_bounded_text(path: &Path, max_bytes: usize) -> Result<BoundedText> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("room_file_not_regular"));
    }
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(max_bytes.saturating_add(1).min(MAX_GIT_OUTPUT_BYTES));
    Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
        while !bytes.is_empty() && std::str::from_utf8(&bytes).is_err() {
            bytes.pop();
        }
    }
    let content = String::from_utf8(bytes).map_err(|_| anyhow!("room_file_not_utf8"))?;
    Ok(BoundedText { content, truncated })
}

/// Idempotently initialize the effective Room root. New/empty roots receive
/// the complete bundled scaffold and one scaffold-only commit. Existing roots
pub(crate) fn ensure_repository(config: &Config) -> Result<()> {
    if config.room.diary_day_boundary_hour > 23 {
        return Err(anyhow!(
            "invalid_diary_day_boundary_hour: {}",
            config.room.diary_day_boundary_hour
        ));
    }
    reject_unresolved_path_components(&config.workspace_root)?;
    if let Some(path) = config.room.repository_root.as_ref() {
        let expanded = exec::expand_pathbuf(path).unwrap_or_else(|_| path.clone());
        let candidate = if expanded.is_absolute() {
            expanded
        } else {
            config.workspace_root.join(expanded)
        };
        reject_unresolved_path_components(&candidate)?;
    }
    let configured_root = repository_root(config);
    let root_existed = configured_root.exists();
    let had_git_metadata = fs::symlink_metadata(configured_root.join(".git"))
        .ok()
        .is_some();
    let had_content = if root_existed {
        let metadata = fs::symlink_metadata(&configured_root)?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("room_repository_symlink_escape"));
        }
        if !metadata.is_dir() {
            return Err(anyhow!("room_repository_root_not_directory"));
        }
        !is_directory_empty(&configured_root)?
    } else {
        false
    };
    let should_bootstrap = !had_content && !had_git_metadata;

    create_directory_checked(&configured_root)?;
    let root = checked_existing_directory(&configured_root)?;
    let workspace = normalized_existing_or_missing(&config.workspace_root)?;
    if root == workspace {
        return Err(anyhow!("room_repository_root_must_be_separate"));
    }

    reject_git_metadata_symlink(&root)?;
    let top = git_top_level(&root)?;
    if top.as_ref() != Some(&root) {
        let output = run_git(&root, &["init", "-b", "main"])?;
        require_git_success(&output, "room_repository_init_failed")?;
    }
    let top = git_top_level(&root)?.ok_or_else(|| anyhow!("room_repository_init_failed"))?;
    if top != root {
        return Err(anyhow!(
            "room_repository_root_not_top_level: expected={}, actual={}",
            root.display(),
            top.display()
        ));
    }

    if should_bootstrap {
        bootstrap_scaffold(&root, config.room.diary_day_boundary_hour)?;
    }
    Ok(())
}

/// Inspect the root without creating it or upgrading its control plane.
pub(crate) fn inspect_repository(config: &Config) -> Result<RepositoryStatus> {
    reject_unresolved_path_components(&config.workspace_root)?;
    if let Some(path) = config.room.repository_root.as_ref() {
        let expanded = exec::expand_pathbuf(path).unwrap_or_else(|_| path.clone());
        let candidate = if expanded.is_absolute() {
            expanded
        } else {
            config.workspace_root.join(expanded)
        };
        reject_unresolved_path_components(&candidate)?;
    }
    let root = normalized_existing_or_missing(&repository_root(config))?;
    let root_exists = fs::symlink_metadata(&root)
        .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
        .unwrap_or(false);
    if root_exists {
        reject_git_metadata_symlink(&root)?;
    }
    let top = if root_exists {
        git_top_level(&root)?
    } else {
        None
    };
    let top_level_initialized = top.as_ref() == Some(&root);
    let branch = if top_level_initialized {
        git_text(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
    } else {
        None
    };
    let head = if top_level_initialized {
        git_text(&root, &["rev-parse", "--verify", "HEAD"])
    } else {
        None
    };
    let clean = if top_level_initialized {
        git_output(&root, &["status", "--porcelain", "--untracked-files=all"])
            .ok()
            .and_then(|output| output.status.success().then(|| output.stdout.is_empty()))
    } else {
        None
    };

    let schema_version = if root_exists {
        read_schema_version(&root).ok().flatten()
    } else {
        None
    };
    let schema = match schema_version {
        Some(ROOM_SCHEMA_VERSION) => Readiness::Ready,
        Some(_) => Readiness::Outdated,
        None if root_exists => Readiness::Missing,
        None => Readiness::Missing,
    };
    let scaffold = if !root_exists {
        Readiness::Missing
    } else if schema != Readiness::Ready {
        Readiness::Incomplete
    } else if scaffold_files_ready(&root) {
        Readiness::Ready
    } else {
        Readiness::Incomplete
    };
    let control_plane = if scaffold == Readiness::Ready {
        Readiness::Ready
    } else {
        scaffold
    };
    let local_executor = if !root_exists {
        Readiness::Missing
    } else {
        match repository_path(&root, "scripts/apply_maintenance.py") {
            Ok(path) => readiness_for_file(&path),
            Err(_) => Readiness::Unavailable,
        }
    };
    let workflow = if root_exists {
        workflow_readiness(&root)
    } else {
        Readiness::Missing
    };
    let remote = if !top_level_initialized {
        Readiness::Missing
    } else {
        match git_output(&root, &["remote", "get-url", "origin"]) {
            Ok(output)
                if output.status.success() && !trimmed_git_text(&output.stdout).is_empty() =>
            {
                Readiness::Ready
            }
            Ok(_) => Readiness::Missing,
            Err(_) => Readiness::Unavailable,
        }
    };
    let sync = if !top_level_initialized {
        Readiness::Missing
    } else {
        match git_output(
            &root,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        ) {
            Ok(output) if output.status.success() => Readiness::Ready,
            Ok(_) => Readiness::Missing,
            Err(_) => Readiness::Unavailable,
        }
    };

    Ok(RepositoryStatus {
        root,
        repository_initialized: top_level_initialized,
        top_level_initialized,
        branch,
        head,
        clean,
        schema_version,
        schema,
        scaffold,
        control_plane,
        local_executor,
        workflow,
        remote,
        sync,
    })
}

/// The exact relative files Agentic may create during a new-root bootstrap.
pub(crate) fn scaffold_paths() -> &'static [&'static str] {
    // Keep this list separate from the map so tests and future status readers
    // can reason about the staging boundary without parsing asset contents.
    static PATHS: [&str; 16] = [
        "room.json",
        "Diary/Daily/current.md",
        "Diary/Weekly/current.md",
        "Diary/Monthly/current.md",
        "Notebook/.gitkeep",
        "State/entities/.gitkeep",
        "manual/diary.md",
        "manual/notebook.md",
        "manual/entity.md",
        "maintenance/diary/daily/.gitkeep",
        "maintenance/diary/weekly/.gitkeep",
        "maintenance/diary/monthly/.gitkeep",
        "maintenance/notebook/.gitkeep",
        "maintenance/entity/.gitkeep",
        "scripts/apply_maintenance.py",
        ".github/workflows/apply-maintenance.yml",
    ];
    &PATHS
}

/// Return deterministic scaffold paths which are absent or not regular files.
///
/// The public status surface reports files rather than implicit directories;
/// the scaffold's `.gitkeep` entries make every otherwise-empty directory
/// observable in this list.
pub(crate) fn scaffold_missing_paths(root: &Path) -> Vec<String> {
    let root_ready = fs::symlink_metadata(root)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false);
    scaffold_paths()
        .iter()
        .filter_map(|relative| {
            let ready = root_ready
                && repository_path(root, relative)
                    .ok()
                    .and_then(|path| fs::symlink_metadata(path).ok())
                    .is_some_and(|metadata| {
                        metadata.is_file() && !metadata.file_type().is_symlink()
                    });
            (!ready).then(|| (*relative).to_string())
        })
        .collect()
}

fn bootstrap_scaffold(root: &Path, boundary_hour: u32) -> Result<()> {
    let logical_date = room_logical_date(Utc::now().with_timezone(&Shanghai), boundary_hour)?;
    for &(relative, template) in SCAFFOLD_FILES {
        let path = repository_path(root, relative)?;
        if let Some(parent) = path.parent() {
            create_directory_checked(parent)?;
        }
        let content = if template.contains(SCAFFOLD_DATE_TOKEN) {
            template.replace(SCAFFOLD_DATE_TOKEN, &logical_date.to_string())
        } else {
            template.to_string()
        };
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        set_scaffold_permissions(relative, &path)?;
    }

    let mut stage_args = vec!["add", "--"];
    stage_args.extend(scaffold_paths().iter().copied());
    let staged = run_git(root, &stage_args)?;
    require_git_success(&staged, "room_repository_stage_failed")?;

    let staged_names = git_text_lines(root, &["diff", "--cached", "--name-only", "--no-renames"])?;
    let expected = scaffold_paths().iter().copied().collect::<BTreeSet<_>>();
    let actual = staged_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(anyhow!("room_repository_scaffold_stage_mismatch"));
    }

    let date = logical_date.format("%Y-%m-%dT00:00:00+08:00").to_string();
    let commit = run_git_with_env(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "tag.gpgSign=false",
            "-c",
            "user.name=Agentic GPT Room",
            "-c",
            "user.email=agentic-gpt-room@localhost",
            "commit",
            "--no-gpg-sign",
            "--no-verify",
            "-m",
            INITIAL_COMMIT_MESSAGE,
        ],
        &[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_AUTHOR_NAME", "Agentic GPT Room"),
            ("GIT_AUTHOR_EMAIL", "agentic-gpt-room@localhost"),
            ("GIT_COMMITTER_NAME", "Agentic GPT Room"),
            ("GIT_COMMITTER_EMAIL", "agentic-gpt-room@localhost"),
            ("GIT_AUTHOR_DATE", &date),
            ("GIT_COMMITTER_DATE", &date),
        ],
    )?;
    require_git_success(&commit, "room_repository_initial_commit_failed")?;
    Ok(())
}

fn room_logical_date(local: DateTime<chrono_tz::Tz>, boundary_hour: u32) -> Result<NaiveDate> {
    if boundary_hour > 23 {
        return Err(anyhow!("invalid_diary_day_boundary_hour: {boundary_hour}"));
    }
    Ok(if local.hour() < boundary_hour {
        local.date_naive() - Duration::days(1)
    } else {
        local.date_naive()
    })
}

#[cfg(unix)]
fn set_scaffold_permissions(relative: &str, path: &Path) -> Result<()> {
    if relative == "scripts/apply_maintenance.py" {
        let mut permissions = fs::metadata(path)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_scaffold_permissions(_relative: &str, _path: &Path) -> Result<()> {
    Ok(())
}

fn scaffold_files_ready(root: &Path) -> bool {
    SCAFFOLD_DIRECTORIES.iter().all(|relative| {
        let Ok(path) = repository_path(root, relative) else {
            return false;
        };
        fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
    }) && SCAFFOLD_FILES.iter().all(|(relative, _)| {
        let Ok(path) = repository_path(root, relative) else {
            return false;
        };
        fs::symlink_metadata(path)
            .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
    })
}

fn read_schema_version(root: &Path) -> Result<Option<u64>> {
    let value = read_bounded_text(&repository_path(root, "room.json")?, MAX_ROOM_JSON_BYTES)?;
    let json: Value =
        serde_json::from_str(&value.content).map_err(|_| anyhow!("room_schema_invalid"))?;
    Ok(json.get("schemaVersion").and_then(Value::as_u64))
}

fn readiness_for_file(path: &Path) -> Readiness {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Readiness::Unavailable,
        Ok(metadata) if metadata.is_file() => Readiness::Ready,
        Ok(_) => Readiness::Incomplete,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Readiness::Missing,
        Err(_) => Readiness::Unavailable,
    }
}

fn workflow_readiness(root: &Path) -> Readiness {
    let path = match repository_path(root, ".github/workflows/apply-maintenance.yml") {
        Ok(path) => path,
        Err(_) => return Readiness::Unavailable,
    };
    if readiness_for_file(&path) != Readiness::Ready {
        return readiness_for_file(&path);
    }
    match read_bounded_text(&path, MAX_ROOM_JSON_BYTES) {
        Ok(value) if value.content.contains("scripts/apply_maintenance.py") => Readiness::Ready,
        Ok(_) => Readiness::Incomplete,
        Err(_) => Readiness::Unavailable,
    }
}

fn is_directory_empty(path: &Path) -> Result<bool> {
    Ok(fs::read_dir(path)?.next().transpose()?.is_none())
}

fn reject_git_metadata_symlink(root: &Path) -> Result<()> {
    let git = root.join(".git");
    if fs::symlink_metadata(&git)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(anyhow!("room_repository_git_symlink"));
    }
    Ok(())
}

fn create_directory_checked(path: &Path) -> Result<()> {
    let normalized = absolute_normalize(path)?;
    let mut current = PathBuf::new();
    for component in normalized.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::Normal(part) => {
                current.push(part);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(anyhow!("room_repository_symlink_escape"));
                    }
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(anyhow!("room_repository_path_not_directory"));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir(&current)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Component::CurDir | Component::ParentDir => {
                return Err(anyhow!("room_repository_path_invalid"));
            }
        }
    }
    Ok(())
}

fn checked_existing_directory(path: &Path) -> Result<PathBuf> {
    let normalized = normalized_existing_or_missing(path)?;
    if !normalized.exists() {
        return Err(anyhow!("room_repository_root_missing"));
    }
    let metadata = fs::symlink_metadata(&normalized)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!("room_repository_root_not_directory"));
    }
    Ok(normalized.canonicalize()?)
}

fn reject_unresolved_path_components(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let has_dot_component = absolute
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir));
    if has_dot_component && !absolute.exists() {
        return Err(anyhow!("room_repository_path_invalid"));
    }
    Ok(())
}

fn normalized_existing_or_missing(path: &Path) -> Result<PathBuf> {
    let normalized = absolute_normalize(path)?;
    let mut current = PathBuf::new();
    let mut missing = false;
    for component in normalized.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::Normal(part) => {
                current.push(part);
                if missing {
                    continue;
                }
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(anyhow!("room_repository_symlink_escape"));
                    }
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(anyhow!("room_repository_path_not_directory"));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
                    Err(error) => return Err(error.into()),
                }
            }
            Component::CurDir | Component::ParentDir => {
                return Err(anyhow!("room_repository_path_invalid"));
            }
        }
    }
    Ok(if missing {
        current
    } else {
        current.canonicalize()?
    })
}

fn absolute_normalize(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err(anyhow!("room_repository_path_invalid"));
                }
            }
            Component::Normal(part) => result.push(part),
        }
    }
    if result.as_os_str().is_empty() {
        return Err(anyhow!("room_repository_path_invalid"));
    }
    Ok(result)
}

fn git_top_level(root: &Path) -> Result<Option<PathBuf>> {
    let output = git_output(root, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let top = trimmed_git_text(&output.stdout);
    if top.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(top);
    normalized_existing_or_missing(&path)
        .map(Some)
        .map_err(|error| anyhow!("room_repository_git_top_level_invalid: {error}"))
}
/// Run Git while retaining only bounded stdout and the exit status.
///
/// Callers must map unsuccessful commands to a stable semantic error. The
/// bounded stderr captured by the internal runner is intentionally not exposed.
pub(crate) fn run_git_bounded(root: &Path, args: &[&str]) -> Result<BoundedGitOutput> {
    run_git_bounded_with_timeout(root, args, GIT_COMMAND_TIMEOUT)
}

/// Run Git with an explicit wall-clock timeout.
pub(crate) fn run_git_bounded_with_timeout(
    root: &Path,
    args: &[&str],
    timeout: StdDuration,
) -> Result<BoundedGitOutput> {
    let output = run_git_with_env_timeout(root, args, &[], timeout)?;
    Ok(BoundedGitOutput {
        success: output.status.success(),
        stdout: output.stdout,
    })
}

/// Dynamic-argument counterpart to [`run_git_bounded_with_timeout`].
pub(crate) fn run_git_bounded_args_with_timeout(
    root: &Path,
    args: &[String],
    timeout: StdDuration,
) -> Result<BoundedGitOutput> {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_git_bounded_with_timeout(root, &args, timeout)
}

/// Run Git with fixed identity/configuration while retaining bounded stdout.
pub(crate) fn run_git_bounded_with_env(
    root: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<BoundedGitOutput> {
    let output = run_git_with_env_timeout(root, args, env, GIT_COMMAND_TIMEOUT)?;
    Ok(BoundedGitOutput {
        success: output.status.success(),
        stdout: output.stdout,
    })
}

fn git_output(root: &Path, args: &[&str]) -> Result<GitOutput> {
    run_git(root, args)
}

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(root, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let value = trimmed_git_text(&output.stdout);
    (!value.is_empty()).then_some(value)
}

fn git_text_lines(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let output = git_output(root, args)?;
    require_git_success(&output, "room_repository_git_query_failed")?;
    Ok(trimmed_git_text(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn run_git(root: &Path, args: &[&str]) -> Result<GitOutput> {
    run_git_with_env(root, args, &[])
}

fn run_git_with_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<GitOutput> {
    run_git_with_env_timeout(root, args, env, GIT_COMMAND_TIMEOUT)
}

fn run_git_with_env_timeout(
    root: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: StdDuration,
) -> Result<GitOutput> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("git_command_failed: {}", args.join(" ")))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_child(&mut child);
            let _ = child.wait();
            return Err(anyhow!("git_stdout_unavailable"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_child(&mut child);
            let _ = child.wait();
            return Err(anyhow!("git_stderr_unavailable"));
        }
    };
    let stdout_reader = thread::spawn(move || read_bounded_stream(stdout));
    let stderr_reader = thread::spawn(move || read_bounded_stream(stderr));
    let deadline = Instant::now() + timeout;
    let status_result: Result<(ExitStatus, bool)> = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok((status, false)),
            Ok(None) if Instant::now() >= deadline => {
                terminate_child(&mut child);
                break child
                    .wait()
                    .map(|status| (status, true))
                    .map_err(|_| anyhow!("git_command_wait_failed"));
            }
            Ok(None) => thread::sleep(StdDuration::from_millis(10)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = child.wait();
                break Err(error.into());
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("git_stdout_reader_failed"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git_stderr_reader_failed"))??;
    let (status, timed_out) = status_result?;
    if timed_out {
        return Err(anyhow!("git_command_timed_out"));
    }
    Ok(GitOutput {
        status,
        stdout,
        stderr,
    })
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        if pid > 0 {
            // Git can leave an SSH/helper child attached to its output pipes.
            // Kill the process group before joining the bounded readers.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
}

fn read_bounded_stream<R: Read>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::with_capacity(MAX_GIT_OUTPUT_BYTES);
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_GIT_OUTPUT_BYTES.saturating_sub(retained.len());
        if remaining != 0 {
            retained.extend_from_slice(&buffer[..count.min(remaining)]);
        }
    }
    Ok(retained)
}

fn trimmed_git_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn require_git_success(output: &GitOutput, code: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = trimmed_git_text(&output.stderr);
    if stderr.is_empty() {
        Err(anyhow!("{}", code))
    } else {
        Err(anyhow!("{code}: {stderr}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use chrono::TimeZone;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-room-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn config_for(workspace: &Path) -> Config {
        let mut config = Config::default_config().expect("default config");
        config.workspace_root = workspace.to_path_buf();
        config
    }

    #[test]
    fn logical_day_respects_the_configured_shanghai_boundary() {
        let before_boundary = Shanghai
            .with_ymd_and_hms(2026, 9, 12, 4, 59, 59)
            .single()
            .unwrap();
        let at_boundary = Shanghai
            .with_ymd_and_hms(2026, 9, 12, 5, 0, 0)
            .single()
            .unwrap();
        assert_eq!(
            room_logical_date(before_boundary, 5).unwrap().to_string(),
            "2026-09-11"
        );
        assert_eq!(
            room_logical_date(at_boundary, 5).unwrap().to_string(),
            "2026-09-12"
        );
    }

    #[test]
    fn invalid_boundary_does_not_partially_initialize_repository() {
        let workspace = temp_root("invalid-boundary");
        let mut config = config_for(&workspace);
        config.room.diary_day_boundary_hour = 24;
        assert!(ensure_repository(&config).is_err());
        assert!(!repository_root(&config).exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn absent_root_bootstraps_exact_scaffold_and_one_main_commit() {
        let workspace = temp_root("bootstrap");
        let config = config_for(&workspace);
        ensure_repository(&config).expect("bootstrap");
        let root = repository_root(&config);
        assert_eq!(git_top_level(&root).expect("top level"), Some(root.clone()));
        assert_eq!(
            git_text(&root, &["symbolic-ref", "--short", "HEAD"]).as_deref(),
            Some("main")
        );
        assert!(git_text(&root, &["rev-parse", "--verify", "HEAD"]).is_some());
        let actual = git_text_lines(&root, &["ls-tree", "-r", "--name-only", "HEAD"])
            .expect("tree")
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected = scaffold_paths()
            .iter()
            .map(|path| (*path).to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert!(!root.join("Hearth").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn bootstrap_is_idempotent_and_does_not_rewrite_initial_commit() {
        let workspace = temp_root("idempotent");
        let config = config_for(&workspace);
        ensure_repository(&config).expect("first bootstrap");
        let root = repository_root(&config);
        let first = git_text(&root, &["rev-parse", "HEAD"]).expect("head");
        ensure_repository(&config).expect("second bootstrap");
        let second = git_text(&root, &["rev-parse", "HEAD"]).expect("head");
        assert_eq!(first, second);
        assert_eq!(
            git_text_lines(&root, &["rev-list", "--count", "HEAD"]).unwrap(),
            vec!["1".to_string()]
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn existing_git_root_is_inspected_without_overwrite_or_scaffold() {
        let workspace = temp_root("existing-git");
        let root = workspace.join("room");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("keep.md"), "keep\n").unwrap();
        let config = config_for(&workspace);
        run_git(&root, &["init", "-b", "main"]).unwrap();
        assert!(!git_text(&root, &["rev-parse", "--verify", "HEAD"]).is_some());
        ensure_repository(&config).unwrap();
        assert_eq!(fs::read_to_string(root.join("keep.md")).unwrap(), "keep\n");
        assert!(!root.join("room.json").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn non_empty_non_git_root_is_unborn_and_unstaged() {
        let workspace = temp_root("non-git");
        let root = workspace.join("room");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("keep.md"), "keep\n").unwrap();
        let config = config_for(&workspace);
        ensure_repository(&config).unwrap();
        assert!(root.join(".git").is_dir());
        assert!(!git_text(&root, &["rev-parse", "--verify", "HEAD"]).is_some());
        assert_eq!(
            git_text_lines(&root, &["status", "--porcelain", "--untracked-files=all"]).unwrap(),
            vec!["?? keep.md".to_string()]
        );
        assert!(!root.join("room.json").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn path_validation_rejects_escape_and_symlink_root() {
        assert!(validate_repository_relative("Diary/current.md").is_ok());
        assert!(validate_repository_relative("../outside.md").is_err());
        assert!(validate_repository_relative("Diary/../outside.md").is_err());
        assert!(validate_repository_relative("Diary\\outside.md").is_err());
        assert!(
            repository_relative(Path::new("/tmp"), Path::new("/tmp/link/../outside.md")).is_err()
        );

        #[cfg(unix)]
        {
            let workspace = temp_root("symlink");
            let outside = temp_root("symlink-target");
            fs::create_dir_all(&workspace).unwrap();
            fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, workspace.join("room")).unwrap();
            let config = config_for(&workspace);
            assert!(ensure_repository(&config).is_err());
            let _ = fs::remove_dir_all(workspace);
            let _ = fs::remove_dir_all(outside);
        }
    }

    #[cfg(unix)]
    #[test]
    fn status_rejects_symlinked_git_metadata() {
        let workspace = temp_root("git-symlink");
        let outside = temp_root("git-symlink-target");
        let root = workspace.join("room");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".git")).unwrap();
        let error = inspect_repository(&config_for(&workspace)).unwrap_err();
        assert!(error.to_string().contains("room_repository_git_symlink"));
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn status_keeps_repository_schema_executor_workflow_remote_and_sync_distinct() {
        let workspace = temp_root("status");
        let config = config_for(&workspace);
        let before = inspect_repository(&config).unwrap();
        assert!(!before.repository_initialized);
        assert_eq!(before.schema, Readiness::Missing);
        assert_eq!(before.local_executor, Readiness::Missing);
        ensure_repository(&config).unwrap();
        let after = inspect_repository(&config).unwrap();
        assert!(after.repository_initialized);
        assert!(after.top_level_initialized);
        assert_eq!(after.schema, Readiness::Ready);
        assert_eq!(after.scaffold, Readiness::Ready);
        assert_eq!(after.control_plane, Readiness::Ready);
        assert_eq!(after.local_executor, Readiness::Ready);
        assert_eq!(after.workflow, Readiness::Ready);
        assert_eq!(after.remote, Readiness::Missing);
        assert_eq!(after.sync, Readiness::Missing);
        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(unix)]
    #[test]
    fn status_rejects_symlinked_executor_and_workflow_ancestors() {
        let workspace = temp_root("status-symlink");
        let outside = temp_root("status-symlink-target");
        let config = config_for(&workspace);
        ensure_repository(&config).unwrap();
        let root = repository_root(&config);
        fs::create_dir_all(outside.join("scripts")).unwrap();
        fs::create_dir_all(outside.join(".github/workflows")).unwrap();
        fs::write(outside.join("scripts/apply_maintenance.py"), "outside\n").unwrap();
        fs::write(
            outside.join(".github/workflows/apply-maintenance.yml"),
            "run: python3 scripts/apply_maintenance.py\n",
        )
        .unwrap();
        fs::remove_dir_all(root.join("scripts")).unwrap();
        fs::remove_dir_all(root.join(".github")).unwrap();
        std::os::unix::fs::symlink(outside.join("scripts"), root.join("scripts")).unwrap();
        std::os::unix::fs::symlink(outside.join(".github"), root.join(".github")).unwrap();

        let status = inspect_repository(&config).unwrap();
        assert_eq!(status.local_executor, Readiness::Unavailable);
        assert_eq!(status.workflow, Readiness::Unavailable);

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn bounded_markdown_helper_caps_bytes_and_rejects_symlinks() {
        let root = temp_root("bounded");
        fs::create_dir_all(root.join("Notebook")).unwrap();
        fs::write(root.join("Notebook/note.md"), "ééé").unwrap();
        let bounded = read_bounded_text(&root.join("Notebook/note.md"), 3).unwrap();
        assert_eq!(bounded.content, "é");
        assert!(bounded.truncated);
        let bounded = read_bounded_markdown(&root, "Notebook/note.md").unwrap();
        assert!(!bounded.truncated);
        assert!(read_bounded_markdown(&root, "Notebook/note.txt").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
