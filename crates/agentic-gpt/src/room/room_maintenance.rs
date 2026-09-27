use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use agentic_gpt_protocol::{
    RoomMaintenanceExecutionMode, RoomMaintenanceLocalExecutorStatus, RoomMaintenanceRemoteStatus,
    RoomMaintenanceRepositoryStatus, RoomMaintenanceRequestItem, RoomMaintenanceScaffoldStatus,
    RoomMaintenanceSchemaStatus, RoomMaintenanceSlot, RoomMaintenanceSlotStatus,
    RoomMaintenanceStatusRequest, RoomMaintenanceStatusResponse, RoomMaintenanceSubmissionState,
    RoomMaintenanceSubmitRequest, RoomMaintenanceSubmitResponse, RoomMaintenanceSyncOutcome,
    RoomMaintenanceSyncStatus, RoomMaintenanceWorkflowStatus,
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::time::sleep;

use crate::config::{Config, RoomMaintenanceMode};
use crate::room_repository::{self, Readiness, RepositoryStatus};
use crate::state::AppState;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const COMMIT_MESSAGE: &str = "Apply Room maintenance";
const SUBMIT_COMMIT_MESSAGE: &str = "Submit Room maintenance request";
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const GIT_IDENTITY: &[(&str, &str)] = &[
    ("GIT_AUTHOR_NAME", "Agentic GPT"),
    ("GIT_AUTHOR_EMAIL", "agentic-gpt@localhost"),
    ("GIT_COMMITTER_NAME", "Agentic GPT"),
    ("GIT_COMMITTER_EMAIL", "agentic-gpt@localhost"),
];

#[derive(Clone, Debug)]
struct PreparedItem {
    slot: RoomMaintenanceSlot,
    payload: Value,
    request_path: String,
    target_path: String,
}

#[derive(Debug)]
struct ProcessOutput {
    success: bool,
}

pub(crate) async fn status(
    state: &AppState,
    _request: RoomMaintenanceStatusRequest,
) -> Result<RoomMaintenanceStatusResponse> {
    let config = state.config.read().await.clone();
    let repository = room_repository::inspect_repository(&config)
        .map_err(|_| anyhow!("room_maintenance_status_unavailable"))?;
    let local_head = repository.head.clone();
    let upstream_head = upstream_head(&repository);
    let in_sync = match (&local_head, &upstream_head) {
        (Some(local), Some(upstream)) => Some(local == upstream),
        _ => None,
    };

    Ok(RoomMaintenanceStatusResponse {
        repository: RoomMaintenanceRepositoryStatus {
            root: repository.root.to_string_lossy().into_owned(),
            initialized: repository.repository_initialized,
            top_level: repository.top_level_initialized,
            branch: repository.branch.clone(),
            head: repository.head.clone(),
            clean: repository.clean,
        },
        schema: RoomMaintenanceSchemaStatus {
            schema_version: repository
                .schema_version
                .and_then(|version| u32::try_from(version).ok()),
            supported: repository.schema == Readiness::Ready,
            ready: repository.schema == Readiness::Ready,
        },
        scaffold: RoomMaintenanceScaffoldStatus {
            ready: repository.scaffold == Readiness::Ready,
            missing_paths: room_repository::scaffold_missing_paths(&repository.root),
        },
        local_executor: RoomMaintenanceLocalExecutorStatus {
            ready: repository.local_executor == Readiness::Ready,
        },
        configured_mode: protocol_mode(config.room.maintenance.mode),
        auto_push: config.room.maintenance.auto_push,
        workflow: RoomMaintenanceWorkflowStatus {
            available: capability_exists(repository.workflow),
            ready: repository.workflow == Readiness::Ready,
        },
        remote: RoomMaintenanceRemoteStatus {
            configured: capability_exists(repository.remote),
            available: repository.remote == Readiness::Ready,
        },
        sync: RoomMaintenanceSyncStatus {
            upstream_available: repository.sync == Readiness::Ready,
            in_sync,
            local_head,
            upstream_head,
        },
        slots: RoomMaintenanceSlot::ALL
            .into_iter()
            .map(|slot| RoomMaintenanceSlotStatus {
                slot,
                occupied: slot_occupied(&repository, slot),
            })
            .collect(),
    })
}

pub(crate) async fn submit(
    state: &AppState,
    request: RoomMaintenanceSubmitRequest,
) -> Result<RoomMaintenanceSubmitResponse> {
    validate_submit_request(&request)?;
    let _write_guard = state.room_repository_writes.lock().await;
    let config = state.config.read().await.clone();
    let repository = room_repository::inspect_repository(&config)
        .map_err(|_| anyhow!("room_maintenance_repository_unavailable"))?;
    require_usable_repository(&repository)?;
    let items = prepare_items(&request.items)?;
    let mode = request
        .mode
        .map(protocol_mode_to_config)
        .unwrap_or(config.room.maintenance.mode);
    match mode {
        RoomMaintenanceMode::Local => submit_local(&repository, &config, &items).await,
        RoomMaintenanceMode::Workflow => {
            submit_workflow(
                &repository,
                &config,
                &items,
                request.effective_wait_seconds(),
            )
            .await
        }
    }
}

fn validate_submit_request(request: &RoomMaintenanceSubmitRequest) -> Result<()> {
    if !request.has_valid_item_count() {
        return Err(anyhow!("room_maintenance_item_count_invalid"));
    }
    for (index, left) in request.items.iter().enumerate() {
        if request.items[index + 1..]
            .iter()
            .any(|right| right.slot == left.slot)
        {
            return Err(anyhow!("room_maintenance_duplicate_slot"));
        }
        let encoded = serde_json::to_vec(&left.payload)
            .map_err(|_| anyhow!("room_maintenance_payload_invalid"))?;
        if encoded.len() > MAX_REQUEST_BYTES {
            return Err(anyhow!("room_maintenance_payload_too_large"));
        }
    }
    Ok(())
}

fn require_usable_repository(repository: &RepositoryStatus) -> Result<()> {
    if !repository.repository_initialized || repository.head.is_none() {
        return Err(anyhow!("room_maintenance_repository_uninitialized"));
    }
    if repository.clean != Some(true) {
        return Err(anyhow!("room_maintenance_repository_dirty"));
    }
    if repository.schema != Readiness::Ready || repository.schema_version != Some(1) {
        return Err(anyhow!("room_maintenance_schema_unsupported"));
    }
    if repository.scaffold != Readiness::Ready || repository.control_plane != Readiness::Ready {
        return Err(anyhow!("room_maintenance_control_plane_incomplete"));
    }
    if repository.local_executor != Readiness::Ready {
        return Err(anyhow!("room_maintenance_executor_unavailable"));
    }
    Ok(())
}

fn prepare_items(items: &[RoomMaintenanceRequestItem]) -> Result<Vec<PreparedItem>> {
    items
        .iter()
        .map(|item| {
            Ok(PreparedItem {
                slot: item.slot,
                payload: item.payload.clone(),
                request_path: slot_request_path(item.slot).to_string(),
                target_path: semantic_target_path(item.slot, &item.payload)?,
            })
        })
        .collect()
}

fn semantic_target_path(slot: RoomMaintenanceSlot, payload: &Value) -> Result<String> {
    match slot {
        RoomMaintenanceSlot::DiaryDaily => Ok("Diary/Daily/current.md".to_string()),
        RoomMaintenanceSlot::DiaryWeekly => Ok("Diary/Weekly/current.md".to_string()),
        RoomMaintenanceSlot::DiaryMonthly => Ok("Diary/Monthly/current.md".to_string()),
        RoomMaintenanceSlot::Notebook => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("room_maintenance_payload_invalid"))?;
            if path.len() > 240
                || !path.starts_with("Notebook/")
                || Path::new(path).extension().and_then(|value| value.to_str()) != Some("md")
            {
                return Err(anyhow!("room_maintenance_payload_invalid"));
            }
            room_repository::validate_repository_relative(path)
                .map_err(|_| anyhow!("room_maintenance_payload_invalid"))?;
            Ok(path.to_string())
        }
        RoomMaintenanceSlot::Entity => {
            let entity = payload
                .get("entity")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("room_maintenance_payload_invalid"))?;
            let components = Path::new(entity).components().collect::<Vec<_>>();
            if entity.is_empty()
                || entity.len() > 160
                || entity.contains(['/', '\\', '\0'])
                || entity == "."
                || entity == ".."
                || components.len() != 1
                || !matches!(components.first(), Some(std::path::Component::Normal(_)))
            {
                return Err(anyhow!("room_maintenance_payload_invalid"));
            }
            Ok(format!("State/entities/{entity}.md"))
        }
    }
}

fn preflight(root: &Path, head: &str, items: &[PreparedItem]) -> Result<()> {
    let worktree = std::env::temp_dir().join(format!(
        "agentic-room-preflight-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let archive = std::env::temp_dir().join(format!(
        "agentic-room-preflight-{}.tar",
        uuid::Uuid::new_v4().simple()
    ));
    let worktree_value = worktree.to_string_lossy().into_owned();
    let archive_value = archive.to_string_lossy().into_owned();
    let add_args = vec![
        "worktree".to_string(),
        "add".to_string(),
        "--detach".to_string(),
        "--no-checkout".to_string(),
        worktree_value.clone(),
        head.to_string(),
    ];
    let added = run_git_mutation_args(root, &add_args);
    if !matches!(added, Ok(output) if output.success) {
        let _ = remove_temporary_file(&archive);
        let _ = remove_worktree(root, &worktree_value);
        return Err(anyhow!("room_maintenance_preflight_failed"));
    }

    let archive_args = vec![
        "archive".to_string(),
        "--format=tar".to_string(),
        format!("--output={archive_value}"),
        head.to_string(),
    ];
    let archived = run_git_mutation_args(root, &archive_args);
    if !matches!(archived, Ok(output) if output.success) {
        let _ = remove_temporary_file(&archive);
        let _ = remove_worktree(root, &worktree_value);
        return Err(anyhow!("room_maintenance_preflight_failed"));
    }

    let tar_args = ["-xf", archive_value.as_str(), "-C", worktree_value.as_str()];
    let extracted = run_process(root, "tar", &tar_args);
    let archive_removed = remove_temporary_file(&archive);
    if !matches!(extracted, Ok(output) if output.success) {
        let _ = remove_worktree(root, &worktree_value);
        return Err(anyhow!("room_maintenance_preflight_failed"));
    }
    if !archive_removed {
        let _ = remove_worktree(root, &worktree_value);
        return Err(anyhow!("room_maintenance_preflight_cleanup_failed"));
    }

    let result = (|| {
        let request_paths = write_requests(&worktree, items)?;
        let executor = run_executor(&worktree);
        let cleanup_requests = remove_request_files(&worktree, &request_paths);
        executor?;
        cleanup_requests?;
        Ok::<(), anyhow::Error>(())
    })();
    let removed = remove_worktree(root, &worktree_value);
    match (result, removed) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(_)) => Err(anyhow!("room_maintenance_preflight_cleanup_failed")),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn remove_temporary_file(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn write_requests(root: &Path, items: &[PreparedItem]) -> Result<Vec<String>> {
    let mut written = Vec::with_capacity(items.len());
    for item in items {
        let path = match write_request(root, item) {
            Ok(path) => path,
            Err(error) => {
                let _ = remove_request_files(root, &written);
                return Err(error);
            }
        };
        written.push(path);
    }
    Ok(written)
}

fn write_request(root: &Path, item: &PreparedItem) -> Result<String> {
    let path = room_repository::repository_path(root, &item.request_path)
        .map_err(|_| anyhow!("room_maintenance_request_path_invalid"))?;
    let payload = serde_json::to_vec(&item.payload)
        .map_err(|_| anyhow!("room_maintenance_payload_invalid"))?;
    if payload.len() > MAX_REQUEST_BYTES {
        return Err(anyhow!("room_maintenance_payload_too_large"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| anyhow!("room_maintenance_slot_occupied"))?;
    if file.write_all(&payload).is_err() {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(anyhow!("room_maintenance_request_write_failed"));
    }
    if file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(anyhow!("room_maintenance_request_write_failed"));
    }
    Ok(item.request_path.clone())
}

fn remove_request_files(root: &Path, paths: &[String]) -> Result<()> {
    for relative in paths {
        let path = room_repository::repository_path(root, relative)
            .map_err(|_| anyhow!("room_maintenance_request_path_invalid"))?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(anyhow!("room_maintenance_request_path_invalid"));
            }
            Ok(_) => fs::remove_file(path)
                .map_err(|_| anyhow!("room_maintenance_request_cleanup_failed"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(anyhow!("room_maintenance_request_cleanup_failed")),
        }
    }
    Ok(())
}

fn remove_worktree(root: &Path, worktree: &str) -> Result<()> {
    let args = vec![
        "worktree".to_string(),
        "remove".to_string(),
        "--force".to_string(),
        worktree.to_string(),
    ];
    let removed = run_git_mutation_args(root, &args);
    let fs_removed = match fs::remove_dir_all(worktree) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    let git_clean = matches!(removed, Ok(output) if output.success)
        || run_git_mutation(root, &["worktree", "prune"])
            .map(|output| output.success)
            .unwrap_or(false);
    if fs_removed && git_clean {
        Ok(())
    } else {
        Err(anyhow!("room_maintenance_preflight_cleanup_failed"))
    }
}

async fn submit_local(
    repository: &RepositoryStatus,
    config: &Config,
    items: &[PreparedItem],
) -> Result<RoomMaintenanceSubmitResponse> {
    if config.room.maintenance.auto_push {
        prepare_local_remote(repository)?;
    }
    let repository = room_repository::inspect_repository(config)
        .map_err(|_| anyhow!("room_maintenance_repository_unavailable"))?;
    require_usable_repository(&repository)?;
    for item in items {
        if slot_occupied(&repository, item.slot) {
            return Err(anyhow!("room_maintenance_slot_occupied"));
        }
    }
    let head = current_head(&repository.root)?;
    preflight(&repository.root, &head, items)?;

    let request_paths = write_requests(&repository.root, items)?;
    let target_paths = items
        .iter()
        .map(|item| item.target_path.clone())
        .collect::<Vec<_>>();
    let result = (|| {
        run_executor(&repository.root)?;

        let changed = git_status_paths(&repository.root)?;
        let expected = target_paths.iter().cloned().collect::<BTreeSet<_>>();
        if changed.iter().any(|path| !expected.contains(path)) {
            return Err(anyhow!("room_maintenance_unexpected_change"));
        }

        stage_paths(&repository.root, &target_paths)?;
        let staged = git_staged_paths(&repository.root)?;
        if staged.iter().any(|path| !expected.contains(path)) {
            return Err(anyhow!("room_maintenance_unexpected_change"));
        }
        if staged != changed {
            return Err(anyhow!("room_maintenance_stage_mismatch"));
        }

        commit(&repository.root, COMMIT_MESSAGE)?;
        let revision = current_head(&repository.root)?;
        let sync = if config.room.maintenance.auto_push {
            push_outcome(&repository.root)
        } else {
            RoomMaintenanceSyncOutcome::NotRequested
        };
        Ok(RoomMaintenanceSubmitResponse {
            mode: RoomMaintenanceExecutionMode::Local,
            state: RoomMaintenanceSubmissionState::Applied,
            local_applied: true,
            sync,
            revision: Some(revision),
        })
    })();
    if result.is_err() {
        cleanup_failed_local_apply(&repository.root, &request_paths, &target_paths);
    }
    result
}

fn prepare_local_remote(repository: &RepositoryStatus) -> Result<()> {
    if !capability_exists(repository.remote) {
        return Ok(());
    }
    if repository.branch.as_deref() != Some("main") {
        return Err(anyhow!("room_maintenance_main_branch_required"));
    }
    let fetched = run_git_mutation(&repository.root, &["fetch", "--no-tags", "origin", "main"])
        .map_err(|_| anyhow!("room_maintenance_remote_unavailable"))?;
    if !fetched.success {
        return Ok(());
    }
    let merged = run_git_mutation(&repository.root, &["merge", "--ff-only", "origin/main"])
        .map_err(|_| anyhow!("room_maintenance_remote_sync_failed"))?;
    if !merged.success {
        return Err(anyhow!("room_maintenance_remote_diverged"));
    }
    Ok(())
}

async fn submit_workflow(
    repository: &RepositoryStatus,
    config: &Config,
    items: &[PreparedItem],
    wait_seconds: u8,
) -> Result<RoomMaintenanceSubmitResponse> {
    if repository.branch.as_deref() != Some("main") {
        return Err(anyhow!("room_maintenance_main_branch_required"));
    }
    if repository.remote != Readiness::Ready {
        return Err(anyhow!("room_maintenance_origin_required"));
    }
    if repository.workflow != Readiness::Ready {
        return Err(anyhow!("room_maintenance_workflow_unavailable"));
    }

    let fetched = run_git_mutation(&repository.root, &["fetch", "--no-tags", "origin", "main"])
        .map_err(|_| anyhow!("room_maintenance_remote_sync_failed"))?;
    if !fetched.success {
        return Err(anyhow!("room_maintenance_remote_sync_failed"));
    }
    let merged = run_git_mutation(&repository.root, &["merge", "--ff-only", "origin/main"])
        .map_err(|_| anyhow!("room_maintenance_remote_sync_failed"))?;
    if !merged.success {
        return Err(anyhow!("room_maintenance_remote_diverged"));
    }

    let repository = room_repository::inspect_repository(config)
        .map_err(|_| anyhow!("room_maintenance_repository_unavailable"))?;
    require_usable_repository(&repository)?;
    if repository.branch.as_deref() != Some("main") {
        return Err(anyhow!("room_maintenance_main_branch_required"));
    }
    if repository.remote != Readiness::Ready {
        return Err(anyhow!("room_maintenance_origin_required"));
    }
    if repository.workflow != Readiness::Ready {
        return Err(anyhow!("room_maintenance_workflow_unavailable"));
    }
    for item in items {
        if slot_occupied(&repository, item.slot) {
            return Err(anyhow!("room_maintenance_slot_occupied"));
        }
    }
    let head = current_head(&repository.root)?;
    preflight(&repository.root, &head, items)?;

    let request_paths = write_requests(&repository.root, items)?;
    let request_set = request_paths.iter().cloned().collect::<BTreeSet<_>>();
    let changed = match git_status_paths(&repository.root) {
        Ok(paths) => paths,
        Err(error) => {
            cleanup_failed_workflow_submit(&repository.root, &request_paths);
            return Err(error);
        }
    };
    if changed.iter().any(|path| !request_set.contains(path)) {
        cleanup_failed_workflow_submit(&repository.root, &request_paths);
        return Err(anyhow!("room_maintenance_unexpected_change"));
    }
    if let Err(error) = stage_paths(&repository.root, &request_paths) {
        cleanup_failed_workflow_submit(&repository.root, &request_paths);
        return Err(error);
    }
    let staged = match git_staged_paths(&repository.root) {
        Ok(paths) => paths,
        Err(error) => {
            cleanup_failed_workflow_submit(&repository.root, &request_paths);
            return Err(error);
        }
    };
    if staged != changed || staged != request_set {
        cleanup_failed_workflow_submit(&repository.root, &request_paths);
        return Err(anyhow!("room_maintenance_stage_mismatch"));
    }
    if let Err(error) = commit(&repository.root, SUBMIT_COMMIT_MESSAGE) {
        cleanup_failed_workflow_submit(&repository.root, &request_paths);
        return Err(error);
    }
    let pushed = run_git_mutation(&repository.root, &["push", "origin", "main"]);
    if !matches!(pushed, Ok(output) if output.success) {
        return Ok(RoomMaintenanceSubmitResponse {
            mode: RoomMaintenanceExecutionMode::Workflow,
            state: RoomMaintenanceSubmissionState::Failed,
            local_applied: false,
            sync: RoomMaintenanceSyncOutcome::Failed,
            revision: None,
        });
    }

    if wait_seconds == 0 {
        return Ok(RoomMaintenanceSubmitResponse {
            mode: RoomMaintenanceExecutionMode::Workflow,
            state: RoomMaintenanceSubmissionState::Submitted,
            local_applied: false,
            sync: RoomMaintenanceSyncOutcome::Pending,
            revision: None,
        });
    }

    if wait_for_workflow(&repository.root, &request_paths, wait_seconds).await {
        let revision = current_head(&repository.root)?;
        return Ok(RoomMaintenanceSubmitResponse {
            mode: RoomMaintenanceExecutionMode::Workflow,
            state: RoomMaintenanceSubmissionState::Applied,
            local_applied: true,
            sync: RoomMaintenanceSyncOutcome::Succeeded,
            revision: Some(revision),
        });
    }

    Ok(RoomMaintenanceSubmitResponse {
        mode: RoomMaintenanceExecutionMode::Workflow,
        state: RoomMaintenanceSubmissionState::Submitted,
        local_applied: false,
        sync: RoomMaintenanceSyncOutcome::Pending,
        revision: None,
    })
}

async fn wait_for_workflow(root: &Path, request_paths: &[String], wait_seconds: u8) -> bool {
    let deadline = Instant::now() + Duration::from_secs(u64::from(wait_seconds));
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let fetched = match room_repository::run_git_bounded_with_timeout(
            root,
            &["fetch", "--no-tags", "origin", "main"],
            remaining,
        ) {
            Ok(output) => output,
            Err(_) => return false,
        };
        if fetched.success && remote_requests_consumed(root, request_paths, deadline) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let merged = match run_git_mutation_with_timeout(
                root,
                &["merge", "--ff-only", "origin/main"],
                remaining,
            ) {
                Ok(output) => output,
                Err(_) => return false,
            };
            return merged.success;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

fn remote_requests_consumed(root: &Path, request_paths: &[String], deadline: Instant) -> bool {
    request_paths.iter().all(|relative| {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let args = vec![
            "ls-tree".to_string(),
            "-r".to_string(),
            "--name-only".to_string(),
            "origin/main".to_string(),
            "--".to_string(),
            literal_git_path(relative),
        ];
        room_repository::run_git_bounded_args_with_timeout(root, &args, remaining)
            .ok()
            .filter(|output| output.success)
            .map(|output| {
                !String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|path| path == relative)
            })
            .unwrap_or(false)
    })
}

fn cleanup_failed_local_apply(root: &Path, request_paths: &[String], target_paths: &[String]) {
    let _ = remove_request_files(root, request_paths);
    let mut restore_args = vec![
        "restore".to_string(),
        "--worktree".to_string(),
        "--staged".to_string(),
        "--".to_string(),
    ];
    restore_args.extend(target_paths.iter().map(|path| literal_git_path(path)));
    let _ = run_git_mutation_args(root, &restore_args);
    let mut clean_args = vec!["clean".to_string(), "-f".to_string(), "--".to_string()];
    clean_args.extend(target_paths.iter().map(|path| literal_git_path(path)));
    let _ = run_git_mutation_args(root, &clean_args);
}

fn cleanup_failed_workflow_submit(root: &Path, request_paths: &[String]) {
    let mut reset_args = vec!["reset".to_string(), "--".to_string()];
    reset_args.extend(request_paths.iter().map(|path| literal_git_path(path)));
    let _ = run_git_mutation_args(root, &reset_args);
    let _ = remove_request_files(root, request_paths);
}

fn literal_git_path(path: &str) -> String {
    format!(":(literal){path}")
}
fn run_git_mutation(root: &Path, args: &[&str]) -> Result<room_repository::BoundedGitOutput> {
    run_git_mutation_with_timeout(root, args, Duration::from_secs(10))
}

fn run_git_mutation_with_timeout(
    root: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<room_repository::BoundedGitOutput> {
    let mut configured = vec!["-c".to_string(), "core.hooksPath=/dev/null".to_string()];
    configured.extend(args.iter().map(|arg| (*arg).to_string()));
    room_repository::run_git_bounded_args_with_timeout(root, &configured, timeout)
}

fn run_git_mutation_args(
    root: &Path,
    args: &[String],
) -> Result<room_repository::BoundedGitOutput> {
    run_git_mutation_args_with_timeout(root, args, Duration::from_secs(10))
}

fn run_git_mutation_args_with_timeout(
    root: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<room_repository::BoundedGitOutput> {
    let mut configured = vec!["-c".to_string(), "core.hooksPath=/dev/null".to_string()];
    configured.extend(args.iter().cloned());
    room_repository::run_git_bounded_args_with_timeout(root, &configured, timeout)
}

fn stage_paths(root: &Path, paths: &[String]) -> Result<()> {
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(paths.iter().map(|path| literal_git_path(path)));
    let output =
        run_git_mutation_args(root, &args).map_err(|_| anyhow!("room_maintenance_stage_failed"))?;
    if output.success {
        Ok(())
    } else {
        Err(anyhow!("room_maintenance_stage_failed"))
    }
}

fn commit(root: &Path, message: &str) -> Result<()> {
    let args = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "tag.gpgSign=false",
        "-c",
        "user.name=Agentic GPT",
        "-c",
        "user.email=agentic-gpt@localhost",
        "commit",
        "--allow-empty",
        "--no-verify",
        "-m",
        message,
    ];
    let env = [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        GIT_IDENTITY[0],
        GIT_IDENTITY[1],
        GIT_IDENTITY[2],
        GIT_IDENTITY[3],
    ];
    let output = room_repository::run_git_bounded_with_env(root, &args, &env)
        .map_err(|_| anyhow!("room_maintenance_commit_failed"))?;
    if output.success {
        Ok(())
    } else {
        Err(anyhow!("room_maintenance_commit_failed"))
    }
}

fn current_head(root: &Path) -> Result<String> {
    let output = room_repository::run_git_bounded(root, &["rev-parse", "--verify", "HEAD"])
        .map_err(|_| anyhow!("room_maintenance_revision_unavailable"))?;
    if !output.success {
        return Err(anyhow!("room_maintenance_revision_unavailable"));
    }
    bounded_text(&output.stdout).ok_or_else(|| anyhow!("room_maintenance_revision_unavailable"))
}

fn push_outcome(root: &Path) -> RoomMaintenanceSyncOutcome {
    let Ok(remote) = room_repository::run_git_bounded(root, &["remote", "get-url", "origin"])
    else {
        return RoomMaintenanceSyncOutcome::Unavailable;
    };
    if !remote.success || bounded_text(&remote.stdout).is_none() {
        return RoomMaintenanceSyncOutcome::Unavailable;
    }
    match run_git_mutation(root, &["push", "origin", "main"]) {
        Ok(output) if output.success => RoomMaintenanceSyncOutcome::Succeeded,
        Ok(_) | Err(_) => RoomMaintenanceSyncOutcome::Failed,
    }
}

fn git_status_paths(root: &Path) -> Result<BTreeSet<String>> {
    let output = room_repository::run_git_bounded(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )
    .map_err(|_| anyhow!("room_maintenance_status_query_failed"))?;
    if !output.success {
        return Err(anyhow!("room_maintenance_status_query_failed"));
    }
    parse_git_paths(&output.stdout)
}

fn git_staged_paths(root: &Path) -> Result<BTreeSet<String>> {
    let output = room_repository::run_git_bounded(root, &["diff", "--cached", "--name-only", "-z"])
        .map_err(|_| anyhow!("room_maintenance_stage_query_failed"))?;
    if !output.success {
        return Err(anyhow!("room_maintenance_stage_query_failed"));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec()).map_err(|_| anyhow!("room_maintenance_path_invalid"))
        })
        .collect()
}
fn parse_git_paths(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 4
            || record[0] == b'R'
            || record[0] == b'C'
            || record[1] == b'R'
            || record[1] == b'C'
        {
            return Err(anyhow!("room_maintenance_rename_unsupported"));
        }
        let path = String::from_utf8(record[3..].to_vec())
            .map_err(|_| anyhow!("room_maintenance_path_invalid"))?;
        paths.insert(path);
    }
    Ok(paths)
}

fn upstream_head(repository: &RepositoryStatus) -> Option<String> {
    if !repository.repository_initialized {
        return None;
    }
    let output = room_repository::run_git_bounded(
        &repository.root,
        &["rev-parse", "--verify", "@{upstream}"],
    )
    .ok()?;
    output
        .success
        .then(|| bounded_text(&output.stdout))
        .flatten()
}

fn slot_occupied(repository: &RepositoryStatus, slot: RoomMaintenanceSlot) -> bool {
    if !repository.root.is_dir() {
        return false;
    }
    let relative = slot_request_path(slot);
    match room_repository::repository_path(&repository.root, relative) {
        Ok(path) => fs::symlink_metadata(path).is_ok(),
        Err(_) => true,
    }
}

fn slot_request_path(slot: RoomMaintenanceSlot) -> &'static str {
    match slot {
        RoomMaintenanceSlot::DiaryDaily => "maintenance/diary/daily/maintenance.json",
        RoomMaintenanceSlot::DiaryWeekly => "maintenance/diary/weekly/maintenance.json",
        RoomMaintenanceSlot::DiaryMonthly => "maintenance/diary/monthly/maintenance.json",
        RoomMaintenanceSlot::Notebook => "maintenance/notebook/maintenance.json",
        RoomMaintenanceSlot::Entity => "maintenance/entity/maintenance.json",
    }
}

fn protocol_mode(mode: RoomMaintenanceMode) -> RoomMaintenanceExecutionMode {
    match mode {
        RoomMaintenanceMode::Local => RoomMaintenanceExecutionMode::Local,
        RoomMaintenanceMode::Workflow => RoomMaintenanceExecutionMode::Workflow,
    }
}

fn protocol_mode_to_config(mode: RoomMaintenanceExecutionMode) -> RoomMaintenanceMode {
    match mode {
        RoomMaintenanceExecutionMode::Local => RoomMaintenanceMode::Local,
        RoomMaintenanceExecutionMode::Workflow => RoomMaintenanceMode::Workflow,
    }
}

fn capability_exists(readiness: Readiness) -> bool {
    !matches!(readiness, Readiness::Missing | Readiness::Unavailable)
}

fn bounded_text(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn run_executor(root: &Path) -> Result<()> {
    let script = room_repository::repository_path(root, "scripts/apply_maintenance.py")
        .map_err(|_| anyhow!("room_maintenance_executor_unavailable"))?;
    let script = script.to_string_lossy().into_owned();
    let output = run_process(root, "python3", &[&script])
        .map_err(|_| anyhow!("room_maintenance_executor_failed"))?;
    if output.success {
        Ok(())
    } else {
        Err(anyhow!("room_maintenance_executor_rejected"))
    }
}

fn run_process(cwd: &Path, program: &str, args: &[&str]) -> Result<ProcessOutput> {
    let mut command = Command::new(program);
    command
        .current_dir(cwd)
        .args(args)
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("room_maintenance_executor_spawn_failed"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                terminate_executor(&mut child);
                break child
                    .wait()
                    .map_err(|_| anyhow!("room_maintenance_executor_failed"))?;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                terminate_executor(&mut child);
                let _ = child.wait();
                return Err(anyhow!("room_maintenance_executor_failed"));
            }
        }
    };
    Ok(ProcessOutput {
        success: status.success(),
    })
}

fn terminate_executor(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        room_repository,
        state::{CapabilityProfile, RuntimeModel},
    };
    use chrono::Utc;
    use serde_json::json;
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::{Mutex, RwLock};

    fn test_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-room-maintenance-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn test_state(workspace_root: PathBuf) -> AppState {
        let mut config = Config::default_config().expect("default config");
        config.workspace_root = workspace_root.clone();
        let private_state = crate::private_state::PrivateStatePaths::for_test(
            workspace_root.join(".private-state"),
        );
        let job_history = crate::job_history::JobHistoryStore::open(&private_state);
        AppState {
            config_path: PathBuf::from("room-maintenance-test-config.json"),
            config: Arc::new(RwLock::new(config)),
            private_state,
            job_history,
            browser_runtime: None,
            runtime: RuntimeModel::local(CapabilityProfile::Room),
            started_at: Utc::now(),
            boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
            room_repository_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    fn notebook_submit(path: &str) -> RoomMaintenanceSubmitRequest {
        RoomMaintenanceSubmitRequest {
            items: vec![RoomMaintenanceRequestItem {
                slot: RoomMaintenanceSlot::Notebook,
                payload: json!({
                    "path": path,
                    "title": "Topic",
                    "body": "Body",
                }),
            }],
            mode: Some(RoomMaintenanceExecutionMode::Local),
            wait_seconds: None,
        }
    }

    fn commit_count(root: &Path) -> String {
        let output =
            room_repository::run_git_bounded(root, &["rev-list", "--count", "HEAD"]).expect("git");
        assert!(output.success);
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    struct WorkspaceCleanup(PathBuf);

    impl Drop for WorkspaceCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const TEST_GIT_ENV: &[(&str, &str)] = &[
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ];

    fn test_git(root: &Path, args: &[&str]) -> Result<room_repository::BoundedGitOutput> {
        room_repository::run_git_bounded_with_env(root, args, TEST_GIT_ENV)
    }

    fn setup_local_origin(root: &Path, origin: &Path) -> Result<()> {
        fs::create_dir_all(origin)?;
        let initialized = test_git(origin, &["init", "--bare", "-b", "main"])?;
        if !initialized.success {
            return Err(anyhow!("test_origin_init_failed"));
        }

        let origin_url = origin.to_string_lossy().into_owned();
        let added = test_git(root, &["remote", "add", "origin", origin_url.as_str()])?;
        if !added.success {
            return Err(anyhow!("test_origin_configure_failed"));
        }
        let pushed = test_git(root, &["push", "-u", "origin", "main"])?;
        if !pushed.success {
            return Err(anyhow!("test_origin_seed_failed"));
        }
        Ok(())
    }

    fn remote_head(origin: &Path) -> Result<String> {
        let output = test_git(origin, &["rev-parse", "--verify", "refs/heads/main"])?;
        if !output.success {
            return Err(anyhow!("test_origin_head_unavailable"));
        }
        bounded_text(&output.stdout).ok_or_else(|| anyhow!("test_origin_head_unavailable"))
    }

    fn remote_contains_path(origin: &Path, relative: &str) -> Result<bool> {
        let output = test_git(
            origin,
            &[
                "ls-tree",
                "-r",
                "--name-only",
                "refs/heads/main",
                "--",
                relative,
            ],
        )?;
        if !output.success {
            return Ok(false);
        }
        Ok(String::from_utf8(output.stdout)?
            .lines()
            .any(|path| path == relative))
    }

    fn remote_file_content(origin: &Path, relative: &str) -> Result<String> {
        let spec = format!("refs/heads/main:{relative}");
        let output = test_git(origin, &["show", spec.as_str()])?;
        if !output.success {
            return Err(anyhow!("test_origin_file_unavailable"));
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    fn create_worker_clone(origin: &Path, worker: &Path) -> Result<()> {
        let origin_url = origin.to_string_lossy().into_owned();
        let worker_path = worker.to_string_lossy().into_owned();
        let cloned = test_git(
            origin,
            &["clone", origin_url.as_str(), worker_path.as_str()],
        )?;
        if !cloned.success {
            return Err(anyhow!("test_worker_clone_failed"));
        }
        Ok(())
    }

    fn worker_apply_request(worker: &Path, request_path: &str, target_path: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("test_worker_timeout"));
            }

            let fetch = room_repository::run_git_bounded_with_timeout(
                worker,
                &["fetch", "--no-tags", "origin", "main"],
                remaining.min(Duration::from_secs(1)),
            )
            .ok();
            if fetch.is_some_and(|output| output.success) {
                let reset = room_repository::run_git_bounded_with_timeout(
                    worker,
                    &["reset", "--hard", "origin/main"],
                    remaining.min(Duration::from_secs(1)),
                )
                .ok();
                if reset.is_some_and(|output| output.success) {
                    let request = room_repository::repository_path(worker, request_path)?;
                    let request_ready = fs::symlink_metadata(request)
                        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                        .unwrap_or(false);
                    if request_ready {
                        run_executor(worker)?;
                        let stage_args = vec![
                            "add".to_string(),
                            "--".to_string(),
                            target_path.to_string(),
                            request_path.to_string(),
                        ];
                        let staged = room_repository::run_git_bounded_args_with_timeout(
                            worker,
                            &stage_args,
                            remaining.min(Duration::from_secs(1)),
                        )?;
                        if !staged.success {
                            return Err(anyhow!("test_worker_stage_failed"));
                        }
                        commit(worker, COMMIT_MESSAGE)?;
                        let pushed = room_repository::run_git_bounded_with_timeout(
                            worker,
                            &["push", "origin", "main"],
                            remaining.min(Duration::from_secs(2)),
                        )?;
                        if !pushed.success {
                            return Err(anyhow!("test_worker_push_failed"));
                        }
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[tokio::test]
    async fn local_submit_rejects_occupied_clean_slot() -> Result<()> {
        let workspace = test_workspace("occupied-clean");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);

        let request_relative = slot_request_path(RoomMaintenanceSlot::Notebook);
        let request = root.join(request_relative);
        let existing = br#"{"path":"Notebook/existing.md","title":"Existing","body":"Body"}"#;
        fs::write(&request, existing)?;
        let staged = run_git_mutation(root.as_path(), &["add", "--", request_relative])?;
        assert!(staged.success);
        commit(&root, "Seed occupied maintenance slot")?;
        let before = current_head(&root)?;

        let error = submit(&state, notebook_submit("Notebook/new.md"))
            .await
            .expect_err("occupied slot must be rejected");
        assert_eq!(error.to_string(), "room_maintenance_slot_occupied");
        assert_eq!(current_head(&root)?, before);
        assert_eq!(fs::read(&request)?, existing);
        assert!(!root.join("Notebook/new.md").exists());
        assert_eq!(
            room_repository::inspect_repository(&config)?.clean,
            Some(true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_auto_push_controls_origin_sync() -> Result<()> {
        let workspace = test_workspace("auto-push");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);
        let origin = workspace.join("origin.git");
        setup_local_origin(&root, &origin)?;
        let baseline = remote_head(&origin)?;

        let first = submit(&state, notebook_submit("Notebook/local-off.md")).await?;
        assert_eq!(first.state, RoomMaintenanceSubmissionState::Applied);
        assert!(first.local_applied);
        assert_eq!(first.sync, RoomMaintenanceSyncOutcome::NotRequested);
        let first_revision = first.revision.clone().expect("local revision");
        assert_ne!(first_revision, baseline);
        assert_eq!(remote_head(&origin)?, baseline);
        assert!(!remote_contains_path(&origin, "Notebook/local-off.md")?);

        {
            let mut config = state.config.write().await;
            config.room.maintenance.auto_push = true;
        }
        let second = submit(&state, notebook_submit("Notebook/local-on.md")).await?;
        assert_eq!(second.state, RoomMaintenanceSubmissionState::Applied);
        assert!(second.local_applied);
        assert_eq!(second.sync, RoomMaintenanceSyncOutcome::Succeeded);
        let second_revision = second.revision.clone().expect("pushed revision");
        assert_eq!(remote_head(&origin)?, second_revision);
        assert_eq!(
            remote_file_content(&origin, "Notebook/local-on.md")?,
            "# Topic\n\nBody\n"
        );
        assert!(remote_contains_path(&origin, "Notebook/local-off.md")?);
        Ok(())
    }

    #[tokio::test]
    async fn local_auto_push_without_origin_reports_unavailable_after_apply() -> Result<()> {
        let workspace = test_workspace("auto-push-no-origin");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        {
            let mut config = state.config.write().await;
            config.room.maintenance.auto_push = true;
        }
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);

        let response = submit(&state, notebook_submit("Notebook/no-origin.md")).await?;
        assert_eq!(response.state, RoomMaintenanceSubmissionState::Applied);
        assert!(response.local_applied);
        assert_eq!(response.sync, RoomMaintenanceSyncOutcome::Unavailable);
        assert!(response.revision.is_some());
        assert_eq!(
            fs::read_to_string(root.join("Notebook/no-origin.md"))?,
            "# Topic\n\nBody\n"
        );
        assert_eq!(commit_count(&root), "2");
        Ok(())
    }

    #[tokio::test]
    async fn local_auto_push_reports_failed_sync_after_origin_disappears() -> Result<()> {
        let workspace = test_workspace("auto-push-unreachable");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        {
            let mut config = state.config.write().await;
            config.room.maintenance.auto_push = true;
        }
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);
        let origin = workspace.join("origin.git");
        setup_local_origin(&root, &origin)?;
        let before = current_head(&root)?;
        fs::remove_dir_all(&origin)?;

        let response = submit(&state, notebook_submit("Notebook/unreachable.md")).await?;
        assert_eq!(response.state, RoomMaintenanceSubmissionState::Applied);
        assert!(response.local_applied);
        assert_eq!(response.sync, RoomMaintenanceSyncOutcome::Failed);
        let revision = response.revision.as_deref().expect("local revision");
        assert_ne!(revision, before);
        assert_eq!(current_head(&root)?, revision);
        assert_eq!(
            fs::read_to_string(root.join("Notebook/unreachable.md"))?,
            "# Topic\n\nBody\n"
        );
        assert_eq!(commit_count(&root), "2");
        Ok(())
    }

    #[tokio::test]
    async fn workflow_wait_timeout_preserves_submitted_request() -> Result<()> {
        let workspace = test_workspace("workflow-timeout");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        {
            let mut config = state.config.write().await;
            config.room.maintenance.mode = RoomMaintenanceMode::Workflow;
        }
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);
        let origin = workspace.join("origin.git");
        setup_local_origin(&root, &origin)?;

        let request_relative = slot_request_path(RoomMaintenanceSlot::Notebook);
        let response = submit(
            &state,
            RoomMaintenanceSubmitRequest {
                mode: Some(RoomMaintenanceExecutionMode::Workflow),
                wait_seconds: Some(1),
                ..notebook_submit("Notebook/workflow-timeout.md")
            },
        )
        .await?;
        assert_eq!(response.mode, RoomMaintenanceExecutionMode::Workflow);
        assert_eq!(response.state, RoomMaintenanceSubmissionState::Submitted);
        assert!(!response.local_applied);
        assert_eq!(response.sync, RoomMaintenanceSyncOutcome::Pending);
        assert!(response.revision.is_none());
        assert!(root.join(request_relative).is_file());
        assert!(remote_contains_path(&origin, request_relative)?);
        let request_payload = remote_file_content(&origin, request_relative)?;
        assert_eq!(
            serde_json::from_str::<Value>(&request_payload)?,
            json!({
                "path": "Notebook/workflow-timeout.md",
                "title": "Topic",
                "body": "Body",
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn workflow_wait_applies_through_repository_worker() -> Result<()> {
        let workspace = test_workspace("workflow-applied");
        fs::create_dir_all(&workspace)?;
        let _cleanup = WorkspaceCleanup(workspace.clone());
        let state = test_state(workspace.clone());
        {
            let mut config = state.config.write().await;
            config.room.maintenance.mode = RoomMaintenanceMode::Workflow;
        }
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);
        let origin = workspace.join("origin.git");
        setup_local_origin(&root, &origin)?;
        let worker = workspace.join("worker");
        create_worker_clone(&origin, &worker)?;

        let request_relative = slot_request_path(RoomMaintenanceSlot::Notebook).to_string();
        let worker_path = worker.clone();
        let worker_request = request_relative.clone();
        let worker_thread = std::thread::spawn(move || {
            worker_apply_request(
                &worker_path,
                &worker_request,
                "Notebook/workflow-applied.md",
            )
        });
        let response_result = submit(
            &state,
            RoomMaintenanceSubmitRequest {
                mode: Some(RoomMaintenanceExecutionMode::Workflow),
                wait_seconds: Some(5),
                ..notebook_submit("Notebook/workflow-applied.md")
            },
        )
        .await;
        let worker_result = worker_thread
            .join()
            .map_err(|_| anyhow!("test_worker_panicked"))?;
        let response = response_result?;
        worker_result?;

        assert_eq!(response.mode, RoomMaintenanceExecutionMode::Workflow);
        assert_eq!(response.state, RoomMaintenanceSubmissionState::Applied);
        assert!(response.local_applied);
        assert_eq!(response.sync, RoomMaintenanceSyncOutcome::Succeeded);
        let revision = response.revision.as_deref().expect("applied revision");
        assert_eq!(current_head(&root)?, revision);
        assert_eq!(remote_head(&origin)?, revision);
        assert_eq!(
            fs::read_to_string(root.join("Notebook/workflow-applied.md"))?,
            "# Topic\n\nBody\n"
        );
        assert_eq!(
            remote_file_content(&origin, "Notebook/workflow-applied.md")?,
            "# Topic\n\nBody\n"
        );
        assert!(!root.join(&request_relative).exists());
        assert!(!remote_contains_path(&origin, &request_relative)?);
        Ok(())
    }

    #[tokio::test]
    async fn status_reports_capabilities_and_empty_slots() -> Result<()> {
        let workspace = test_workspace("status");
        fs::create_dir_all(&workspace)?;
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;

        let response = status(&state, RoomMaintenanceStatusRequest::default()).await?;
        assert!(response.repository.initialized);
        assert_eq!(response.repository.branch.as_deref(), Some("main"));
        assert_eq!(response.schema.schema_version, Some(1));
        assert!(response.schema.supported);
        assert!(response.schema.ready);
        assert!(response.scaffold.ready);
        assert!(response.local_executor.ready);
        assert!(response.workflow.available);
        assert!(response.workflow.ready);
        assert!(!response.remote.configured);
        assert!(!response.remote.available);
        assert!(!response.sync.upstream_available);
        assert!(response.sync.in_sync.is_none());
        assert_eq!(response.slots.len(), RoomMaintenanceSlot::ALL.len());
        assert!(response.slots.iter().all(|slot| !slot.occupied));

        fs::remove_dir_all(workspace)?;
        Ok(())
    }

    #[tokio::test]
    async fn local_submit_preflights_and_commits_one_semantic_change() -> Result<()> {
        let workspace = test_workspace("local");
        fs::create_dir_all(&workspace)?;
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);

        let response = submit(&state, notebook_submit("Notebook/topic.md")).await?;
        assert_eq!(response.mode, RoomMaintenanceExecutionMode::Local);
        assert_eq!(response.state, RoomMaintenanceSubmissionState::Applied);
        assert!(response.local_applied);
        assert_eq!(response.sync, RoomMaintenanceSyncOutcome::NotRequested);
        let first_revision = response.revision.clone().expect("revision");
        assert_eq!(
            fs::read_to_string(root.join("Notebook/topic.md"))?,
            "# Topic\n\nBody\n"
        );
        assert!(!root.join("maintenance/notebook/maintenance.json").exists());
        assert_eq!(commit_count(&root), "2");

        let repeat = submit(&state, notebook_submit("Notebook/topic.md")).await?;
        assert_eq!(repeat.state, RoomMaintenanceSubmissionState::Applied);
        assert!(repeat.local_applied);
        assert_ne!(repeat.revision.as_deref(), Some(first_revision.as_str()));
        assert_eq!(commit_count(&root), "3");
        let worktrees =
            room_repository::run_git_bounded(&root, &["worktree", "list", "--porcelain"])?;
        assert!(worktrees.success);
        assert_eq!(
            String::from_utf8(worktrees.stdout)?
                .lines()
                .filter(|line| line.starts_with("worktree "))
                .count(),
            1
        );

        fs::remove_dir_all(workspace)?;
        Ok(())
    }

    #[tokio::test]
    async fn preflight_rejects_invalid_payload_without_mutating_room() -> Result<()> {
        let workspace = test_workspace("invalid");
        fs::create_dir_all(&workspace)?;
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);
        let before = fs::read_to_string(root.join("Diary/Daily/current.md"))?;

        let error = submit(
            &state,
            RoomMaintenanceSubmitRequest {
                items: vec![RoomMaintenanceRequestItem {
                    slot: RoomMaintenanceSlot::DiaryDaily,
                    payload: json!({"summary": "ok", "unknown": true}),
                }],
                mode: Some(RoomMaintenanceExecutionMode::Local),
                wait_seconds: None,
            },
        )
        .await
        .expect_err("invalid payload must fail preflight");
        assert_eq!(error.to_string(), "room_maintenance_executor_rejected");
        assert_eq!(
            fs::read_to_string(root.join("Diary/Daily/current.md"))?,
            before
        );
        assert!(!root
            .join("maintenance/diary/daily/maintenance.json")
            .exists());
        assert_eq!(commit_count(&root), "1");

        fs::remove_dir_all(workspace)?;
        Ok(())
    }

    #[tokio::test]
    async fn submit_rejects_duplicate_slots_and_dirty_repository() -> Result<()> {
        let workspace = test_workspace("preconditions");
        fs::create_dir_all(&workspace)?;
        let state = test_state(workspace.clone());
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;
        let root = room_repository::repository_root(&config);

        let mut duplicate = notebook_submit("Notebook/duplicate.md");
        duplicate.items.push(RoomMaintenanceRequestItem {
            slot: RoomMaintenanceSlot::Notebook,
            payload: json!({
                "path": "Notebook/other.md",
                "body": "Other",
            }),
        });
        let duplicate_error = submit(&state, duplicate).await.expect_err("duplicate slot");
        assert_eq!(
            duplicate_error.to_string(),
            "room_maintenance_duplicate_slot"
        );

        fs::write(root.join("unrelated.md"), "dirty\n")?;
        let dirty_error = submit(&state, notebook_submit("Notebook/dirty.md"))
            .await
            .expect_err("dirty repository");
        assert_eq!(dirty_error.to_string(), "room_maintenance_repository_dirty");

        fs::remove_dir_all(workspace)?;
        Ok(())
    }

    #[tokio::test]
    async fn workflow_submit_requires_origin_and_workflow_transport() -> Result<()> {
        let workspace = test_workspace("workflow-precondition");
        fs::create_dir_all(&workspace)?;
        let state = test_state(workspace.clone());
        {
            let mut config = state.config.write().await;
            config.room.maintenance.mode = RoomMaintenanceMode::Workflow;
        }
        let config = state.config.read().await.clone();
        room_repository::ensure_repository(&config)?;

        let error = submit(
            &state,
            RoomMaintenanceSubmitRequest {
                mode: None,
                ..notebook_submit("Notebook/workflow.md")
            },
        )
        .await
        .expect_err("workflow needs origin");
        assert_eq!(error.to_string(), "room_maintenance_origin_required");

        fs::remove_dir_all(workspace)?;
        Ok(())
    }
}
