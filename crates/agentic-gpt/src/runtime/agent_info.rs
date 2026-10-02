use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::config::{Config, ConfirmationChannel, Rule};
use crate::exec;
use crate::notify;
use crate::policy::{self, PolicyDecision};
use crate::process;
use crate::state::{AppState, CapabilityProfile, HubMode, Transport};

const MAX_SUMMARY_ENTRIES: usize = 128;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) async fn collect(state: &AppState) -> Value {
    let generated_at = Utc::now();
    let config = state.config.read().await.clone();
    let active = process::current_processes(state).await;
    let active_count = active.len();
    let resolved_limit = config.limits.max_active_processes.resolve();
    let pending_count = state.pending_confirmations.lock().await.len();
    let (hub_sender, reporting_sender) =
        tokio::join!(async { state.hub_sender.lock().await.is_some() }, async {
            state.reporting_sender.lock().await.is_some()
        },);
    let (freedesktop_available, freedesktop_actions) = freedesktop_notification_support();
    let ntfy_available = match state.runtime.transport {
        Transport::Hub => hub_sender,
        Transport::TunnelStdio => reporting_sender,
        Transport::LocalUnix => false,
    };
    let local_mcp_enabled = matches!(
        state.runtime.transport,
        Transport::TunnelStdio | Transport::LocalUnix
    );
    let local_mcp = crate::local_control::status(&config.agent_id, local_mcp_enabled);
    let config_health = config_health(state, &config);
    let mcp_config_revision =
        crate::config::mcp_servers::server_config_revision(&config.mcp_servers);
    let mcp_enabled_count = config
        .mcp_servers
        .values()
        .filter(|server| server.enabled)
        .count();
    let mut issues = config_health.issues.clone();
    if active_count >= resolved_limit.resolved {
        issues.push("active_session_capacity_exhausted");
    }
    let issues = issues
        .into_iter()
        .map(|issue| Value::String(issue.to_string()))
        .collect::<Vec<_>>();
    let capabilities = state.runtime.capabilities();
    let (write_roots, write_truncated) = path_values(
        config
            .path_policy
            .write_roots
            .iter()
            .chain(std::iter::once(&config.workspace_root)),
    );
    let (read_only_roots, read_only_truncated) =
        path_values(config.path_policy.read_only_roots.iter());
    let (deny_roots, deny_truncated) = path_values(config.path_policy.deny_roots.iter());
    let policy_summary = policy_summary(&config, state.runtime.profile);
    let paths_truncated = write_truncated || read_only_truncated || deny_truncated;
    let channels = config
        .confirmation_provider
        .channels
        .iter()
        .map(|channel| channel.as_str())
        .collect::<Vec<_>>();
    let mut providers = Vec::new();
    if config
        .confirmation_provider
        .channels
        .contains(&ConfirmationChannel::Freedesktop)
    {
        providers.push(json!({
            "id": "freedesktop",
            "available": freedesktop_available && freedesktop_actions,
            "supportsActions": freedesktop_actions,
            "reason": if !freedesktop_available { json!("notification_service_unavailable") } else if !freedesktop_actions { json!("actions_unavailable") } else { Value::Null },
        }));
    }
    if config
        .confirmation_provider
        .channels
        .contains(&ConfirmationChannel::Ntfy)
    {
        providers.push(json!({
            "id": "ntfy",
            "available": ntfy_available,
            "supportsActions": ntfy_available,
            "transport": "hub-relay",
            "deliveryHealth": "unknown",
            "reason": if ntfy_available { Value::Null } else { json!("hub_relay_disconnected") },
        }));
    }
    json!({
        "schemaVersion": 1,
        "generatedAt": generated_at,
        "identity": {
            "agentId": config.agent_id,
            "displayName": config.display_name,
            "version": env!("CARGO_PKG_VERSION"),
            "transport": state.runtime.transport.label(),
            "profile": state.runtime.profile.label(),
            "hubMode": state.runtime.hub_mode.label(),
            "startedAt": state.started_at,
            "supervised": state.supervised,
        },
        "host": {
            "hostname": crate::utils::hostname_fallback(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "availableParallelism": resolved_limit.available_parallelism.unwrap_or(1),
        },
        "workspace": {
            "root": exact_path(&config.workspace_root),
            "sandbox": {
                "enabled": config.sandbox.enabled,
                "mode": if config.sandbox.enabled { "bubblewrap" } else { "disabled" },
            },
            "pathPolicy": {
                "writeRoots": write_roots,
                "readOnlyRoots": read_only_roots,
                "denyRoots": deny_roots,
                "truncated": paths_truncated,
            },
        },
        "execution": {
            "programMatching": "exact",
            "processes": {
                "configuredMax": config.limits.max_active_processes.configured_label(),
                "resolvedMax": resolved_limit.resolved,
                "active": active_count,
                "available": resolved_limit.resolved.saturating_sub(active_count),
            },
            "fileSearch": {
                "maxContextLines": config.limits.max_file_search_context_lines,
            },
            "policy": policy_summary,
        },
        "confirmation": {
            "channels": channels,
            "pendingCount": pending_count,
            "providers": providers,
        },
        "connections": {
            "hubReporting": {
                "enabled": state.runtime.hub_mode != HubMode::Disabled,
                "status": reporting_status(state.runtime, hub_sender, reporting_sender),
            },
            "localMcp": local_mcp,
        },
        "mcp": {
            "configRevision": mcp_config_revision,
            "configuredServerCount": config.mcp_servers.len(),
            "enabledServerCount": mcp_enabled_count,
            "clientLifecycle": "per-call",
            "concurrency": {
                "globalLimit": process::MCP_GLOBAL_CONCURRENCY,
                "perServerLimit": process::MCP_PER_SERVER_CONCURRENCY,
                "active": state.mcp_concurrency.active(),
                "queued": state.mcp_concurrency.queued(),
            },
        },
        "config": {
            "path": exact_path(&state.config_path),
            "diskStatus": config_health.disk_status,
            "diskModifiedAt": config_health.modified_at,
            "liveSubsetMatchesDisk": config_health.live_subset_matches_disk,
            "restartRequiredFields": config_health.restart_required_fields,
            "errorCode": config_health.error_code,
        },
        "capabilities": {
            "skills": capabilities.skills,
            "bootstrap": capabilities.bootstrap,
            "diary": capabilities.diary,
            "notebook": capabilities.notebook,
            "notifications": capabilities.notifications,
        },
        "health": {
            "status": if issues.is_empty() { "ready" } else { "degraded" },
            "issues": issues,
        },
    })
}

fn freedesktop_notification_support() -> (bool, bool) {
    static SUPPORT: LazyLock<(bool, bool)> = LazyLock::new(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(notify::detect_freedesktop_notification_support());
        });
        receiver
            .recv_timeout(std::time::Duration::from_millis(250))
            .unwrap_or((false, false))
    });
    *SUPPORT
}

fn reporting_status(
    runtime: crate::state::RuntimeModel,
    hub_sender: bool,
    reporting_sender: bool,
) -> &'static str {
    if runtime.hub_mode == HubMode::Disabled {
        "disabled"
    } else if matches!(runtime.transport, Transport::Hub) {
        if hub_sender {
            "connected"
        } else {
            "disconnected"
        }
    } else if reporting_sender {
        "connected"
    } else {
        "disconnected"
    }
}

fn exact_path(path: &Path) -> String {
    exec::expand_pathbuf(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

fn path_values<'a>(roots: impl Iterator<Item = &'a PathBuf>) -> (Vec<String>, bool) {
    let mut values = Vec::new();
    let mut truncated = false;
    for root in roots {
        if values.len() >= MAX_SUMMARY_ENTRIES {
            truncated = true;
            break;
        }
        let value = exact_path(root);
        if !values.contains(&value) {
            values.push(value);
        }
    }
    (values, truncated)
}

fn policy_summary(config: &Config, profile: CapabilityProfile) -> Value {
    let (allow, allow_truncated) = rule_values(&config.policy.allow);
    let (confirm, confirm_truncated) = rule_values(&config.policy.confirm);
    let (deny, deny_truncated) = rule_values(&config.policy.deny);
    let (builtin_confirm, builtin_confirm_truncated) =
        rule_values(&policy::builtin_rules(profile, PolicyDecision::Confirm));
    let (builtin_deny, builtin_deny_truncated) =
        rule_values(&policy::builtin_rules(profile, PolicyDecision::Deny));
    json!({
        "counts": {
            "allow": config.policy.allow.len(),
            "confirm": config.policy.confirm.len(),
            "deny": config.policy.deny.len(),
        },
        "allow": allow,
        "confirm": confirm,
        "deny": deny,
        "builtinConfirm": builtin_confirm,
        "builtinDeny": builtin_deny,
        "truncated": allow_truncated || confirm_truncated || deny_truncated || builtin_confirm_truncated || builtin_deny_truncated,
    })
}

fn rule_values(rules: &[Rule]) -> (Vec<Value>, bool) {
    let truncated = rules.len() > MAX_SUMMARY_ENTRIES;
    let values = rules
        .iter()
        .take(MAX_SUMMARY_ENTRIES)
        .map(|rule| json!({"program": rule.program, "argsPrefix": rule.args_prefix}))
        .collect();
    (values, truncated)
}

struct ConfigHealth {
    disk_status: &'static str,
    modified_at: Option<String>,
    live_subset_matches_disk: bool,
    restart_required_fields: Vec<String>,
    error_code: Option<String>,
    issues: Vec<&'static str>,
}

fn config_health(state: &AppState, effective: &Config) -> ConfigHealth {
    let metadata = match fs::metadata(&state.config_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ConfigHealth {
                disk_status: "missing",
                modified_at: None,
                live_subset_matches_disk: false,
                restart_required_fields: Vec::new(),
                error_code: Some("config_missing".to_string()),
                issues: vec!["config_missing"],
            }
        }
        Err(_) => {
            return ConfigHealth {
                disk_status: "unreadable",
                modified_at: None,
                live_subset_matches_disk: false,
                restart_required_fields: Vec::new(),
                error_code: Some("config_unreadable".to_string()),
                issues: vec!["config_unreadable"],
            }
        }
    };
    let modified_at = metadata
        .modified()
        .ok()
        .map(|time| DateTime::<Utc>::from(time).to_rfc3339());
    if metadata.len() > MAX_CONFIG_BYTES {
        return ConfigHealth {
            disk_status: "too-large",
            modified_at,
            live_subset_matches_disk: false,
            restart_required_fields: Vec::new(),
            error_code: Some("config_too_large".to_string()),
            issues: vec!["config_unreadable"],
        };
    }
    let disk = match Config::load(&state.config_path) {
        Ok(config) => config,
        Err(_) => {
            return invalid_config_health(modified_at);
        }
    };
    if disk.validate_mcp_servers().is_err() || disk.validate_http_mcp().is_err() {
        return invalid_config_health(modified_at);
    }
    let live_subset_matches_disk = live_subset_matches_disk_for_workspace(effective, &disk);
    let restart_required_fields = crate::config::restart_required_fields(effective, &disk);
    let mut issues = Vec::new();
    if !live_subset_matches_disk {
        issues.push("config_live_subset_not_applied");
    }
    if !restart_required_fields.is_empty() {
        issues.push("config_restart_required");
    }
    ConfigHealth {
        disk_status: "valid",
        modified_at,
        live_subset_matches_disk,
        restart_required_fields,
        error_code: None,
        issues,
    }
}

fn invalid_config_health(modified_at: Option<String>) -> ConfigHealth {
    ConfigHealth {
        disk_status: "invalid",
        modified_at,
        live_subset_matches_disk: false,
        restart_required_fields: Vec::new(),
        error_code: Some("config_invalid".to_string()),
        issues: vec!["config_invalid"],
    }
}

fn live_subset(config: &Config) -> Value {
    json!({
        "policy": config.policy,
        "pathPolicy": config.path_policy,
        "limits": config.limits,
        "mcpServers": config.mcp_servers,
        "toolsets": config.toolsets,
        "httpMcp": config.http_mcp,
    })
}

fn live_subset_matches_disk_for_workspace(effective: &Config, disk: &Config) -> bool {
    let effective_subset = live_subset(effective);
    let mut disk_subset = live_subset(disk);
    if effective.workspace_root != disk.workspace_root {
        if let (Some(path_policy), Some(disk_object)) = (
            effective_subset.get("pathPolicy").cloned(),
            disk_subset.as_object_mut(),
        ) {
            disk_object.insert("pathPolicy".to_string(), path_policy);
        }
    }
    effective_subset == disk_subset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::mcp_servers::McpServerConfig;
    use agentic_gpt_protocol::AgentMessage;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex, RwLock};

    fn state(profile: CapabilityProfile) -> AppState {
        let mut config = Config::default_config().unwrap();
        let root =
            std::env::temp_dir().join(format!("agent-info-{}", uuid::Uuid::new_v4().simple()));
        config.workspace_root = root.clone();
        config.path_policy.write_roots = vec![root.clone()];
        let private_state = crate::private_state::PrivateStatePaths::for_test_agent(
            root.join(".private-state"),
            config.agent_id.clone(),
        );
        let process_history = crate::process_history::ProcessHistoryStore::open(&private_state);
        AppState {
            config_path: std::env::temp_dir().join("agent-info-missing-config.json"),
            config: Arc::new(RwLock::new(config)),
            event_store: crate::event_store::EventStore::open(&private_state).unwrap(),
            private_state,
            process_history,
            browser_runtime: None,
            runtime: crate::state::RuntimeModel::tunnel(profile, false),
            started_at: Utc::now(),
            boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            supervised: true,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
            room_repository_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    #[tokio::test]
    async fn info_is_bounded_redacted_and_profile_correct() {
        let value = collect(&state(CapabilityProfile::Room)).await;
        assert!(value.get("surface").is_none());
        assert_eq!(value["identity"]["profile"], "room");
        assert!(value["workspace"]["pathPolicy"]["writeRoots"][0].is_string());
        assert_eq!(value["config"]["diskStatus"], "missing");
        assert_eq!(value["mcp"]["concurrency"]["globalLimit"], 8);
        assert_eq!(value["mcp"]["concurrency"]["perServerLimit"], 2);
        assert_eq!(value["mcp"]["concurrency"]["active"], 0);
        assert_eq!(value["mcp"]["concurrency"]["queued"], 0);
        assert_eq!(value["execution"]["fileSearch"]["maxContextLines"], 5);
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("change-me"));
        assert!(!serialized.contains("agent_secret"));
    }

    #[tokio::test]
    async fn info_preserves_empty_policy_lists_and_deduplicates_workspace_root() {
        let app = state(CapabilityProfile::Room);
        let root =
            std::env::temp_dir().join(format!("agent-info-room-{}", uuid::Uuid::new_v4().simple()));
        {
            let mut config = app.config.write().await;
            config.workspace_root = PathBuf::from(format!("{}/", root.display()));
            config.path_policy.write_roots = vec![root.clone(), PathBuf::from("/tmp")];
            config.path_policy.read_only_roots.clear();
            config.path_policy.deny_roots.clear();
        }

        let value = collect(&app).await;
        assert_eq!(value["workspace"]["root"], root.to_string_lossy().as_ref());
        assert_eq!(
            value["workspace"]["pathPolicy"]["writeRoots"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(value["workspace"]["pathPolicy"]["readOnlyRoots"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(value["workspace"]["pathPolicy"]["denyRoots"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn info_reports_restart_differences_and_current_ntfy_relay() {
        let mut app = state(CapabilityProfile::Normal);
        let disk_path = std::env::temp_dir().join(format!(
            "agent-info-config-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let mut disk = app.config.read().await.clone();
        disk.mode = crate::config::RuntimeMode::Hub;
        disk.profile = crate::config::WorkerProfile::Room;
        fs::write(&disk_path, serde_json::to_string_pretty(&disk).unwrap()).unwrap();
        app.config_path = disk_path.clone();
        app.runtime = crate::state::RuntimeModel::hub(CapabilityProfile::Normal);
        app.config.write().await.display_name = "effective-only".to_string();
        let (tx, _rx) = mpsc::unbounded_channel::<AgentMessage>();
        *app.hub_sender.lock().await = Some(tx);
        let value = collect(&app).await;
        assert_eq!(value["config"]["diskStatus"], "valid");
        assert_eq!(value["config"]["liveSubsetMatchesDisk"], true);
        assert!(value["config"]["restartRequiredFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "displayName"));
        assert!(value["config"]["restartRequiredFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "mode"));
        assert!(value["config"]["restartRequiredFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "profile"));
        let ntfy = value["confirmation"]["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["id"] == "ntfy")
            .unwrap();
        assert_eq!(ntfy["available"], true);
        assert_eq!(ntfy["deliveryHealth"], "unknown");
        let _ = fs::remove_file(disk_path);
    }

    #[test]
    fn config_health_ignores_path_policy_drift_with_workspace_restart() {
        let mut app = state(CapabilityProfile::Normal);
        let root = std::env::temp_dir().join(format!(
            "agent-info-path-policy-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let live_workspace = root.join("workspace-live");
        let candidate_workspace = root.join("workspace-candidate");
        let live_write_root = live_workspace.join("write");
        let candidate_write_root = candidate_workspace.join("write");
        fs::create_dir_all(&live_write_root).unwrap();
        fs::create_dir_all(&candidate_write_root).unwrap();

        let mut effective = Config::default_config().unwrap();
        effective.workspace_root = live_workspace;
        effective.path_policy.write_roots = vec![live_write_root];
        let mut disk = effective.clone();
        disk.workspace_root = candidate_workspace;
        disk.path_policy.write_roots = vec![candidate_write_root];
        disk.browser.managed.enabled = false;
        let disk_path = root.join("config.json");
        fs::write(&disk_path, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();
        app.config_path = disk_path.clone();

        let health = config_health(&app, &effective);
        assert_eq!(health.disk_status, "valid");
        assert!(health.live_subset_matches_disk);
        assert!(health
            .restart_required_fields
            .iter()
            .any(|field| field == "workspaceRoot"));
        assert!(health
            .restart_required_fields
            .iter()
            .any(|field| field == "browser"));
        assert!(!health
            .issues
            .iter()
            .any(|issue| issue == &"config_live_subset_not_applied"));

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn info_reports_mcp_live_subset_revision_without_restart_requirement() {
        let mut app = state(CapabilityProfile::Normal);
        let disk_path = std::env::temp_dir().join(format!(
            "agent-info-mcp-config-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let mut effective = app.config.read().await.clone();
        effective.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "streamable-http".to_string(),
                url: Some("https://old.example/mcp".to_string()),
                auth: None,
            },
        );
        *app.config.write().await = effective.clone();
        app.config_path = disk_path.clone();

        let mut disk = effective.clone();
        disk.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "streamable-http".to_string(),
                url: Some("https://new.example/mcp".to_string()),
                auth: None,
            },
        );
        disk.mcp_servers.insert(
            "local".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                url: Some("node ./local.mjs".to_string()),
                auth: None,
            },
        );
        fs::write(&disk_path, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();

        let before_reload = collect(&app).await;
        assert_eq!(before_reload["config"]["liveSubsetMatchesDisk"], false);
        assert!(!before_reload["config"]["restartRequiredFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "mcpServers"));
        assert_eq!(before_reload["mcp"]["configuredServerCount"], 1);
        assert_eq!(before_reload["mcp"]["enabledServerCount"], 1);
        assert_eq!(before_reload["mcp"]["clientLifecycle"], "per-call");
        let before_revision = before_reload["mcp"]["configRevision"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!serde_json::to_string(&before_reload)
            .unwrap()
            .contains("old.example"));

        app.config.write().await.mcp_servers = disk.mcp_servers.clone();
        let after_reload = collect(&app).await;
        assert_eq!(after_reload["config"]["liveSubsetMatchesDisk"], true);
        assert_eq!(after_reload["mcp"]["configuredServerCount"], 2);
        assert_eq!(after_reload["mcp"]["enabledServerCount"], 1);
        assert_ne!(
            after_reload["mcp"]["configRevision"].as_str().unwrap(),
            before_revision
        );
        assert!(!after_reload["config"]["restartRequiredFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "mcpServers"));

        disk.mcp_servers.get_mut("primary").unwrap().transport = "sse".to_string();
        fs::write(&disk_path, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();
        let invalid = collect(&app).await;
        assert_eq!(invalid["config"]["diskStatus"], "invalid");
        assert_eq!(invalid["config"]["errorCode"], "config_invalid");
        let _ = fs::remove_file(disk_path);
    }

    #[test]
    fn info_reports_toolset_live_subset_difference_without_restart_requirement() {
        let mut app = state(CapabilityProfile::Normal);
        let disk_path = std::env::temp_dir().join(format!(
            "agent-info-toolsets-config-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let effective = Config::default_config().unwrap();
        app.config_path = disk_path.clone();

        let mut disk = effective.clone();
        disk.toolsets.enable(crate::config::ToolNamespace::Room);
        fs::write(&disk_path, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();

        let before_reload = config_health(&app, &effective);
        assert_eq!(before_reload.disk_status, "valid");
        assert!(!before_reload.live_subset_matches_disk);
        assert!(!before_reload
            .restart_required_fields
            .iter()
            .any(|field| field == "toolsets"));

        let after_reload = config_health(&app, &disk);
        assert!(after_reload.live_subset_matches_disk);
        assert!(!after_reload
            .restart_required_fields
            .iter()
            .any(|field| field == "toolsets"));

        let _ = fs::remove_file(disk_path);
    }

    #[tokio::test]
    async fn info_reports_invalid_config_and_capacity_exhaustion_without_secrets() {
        let mut app = state(CapabilityProfile::Normal);
        let invalid = std::env::temp_dir().join(format!(
            "agent-info-invalid-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        fs::write(&invalid, b"not-json").unwrap();
        app.config_path = invalid.clone();
        app.config.write().await.limits.max_active_processes =
            crate::config::MaxActiveProcesses::Explicit(0);
        let value = collect(&app).await;
        assert_eq!(value["config"]["diskStatus"], "invalid");
        assert_eq!(value["health"]["status"], "degraded");
        assert!(value["health"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue == "active_session_capacity_exhausted"));
        assert!(!serde_json::to_string(&value).unwrap().contains("change-me"));
        let _ = fs::remove_file(invalid);
    }
}
