use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::config::{Config, ConfirmationChannel, Rule};
use crate::exec;
use crate::notify;
use crate::policy::{self, PolicyDecision};
use crate::sessions;
use crate::state::{AppState, CapabilityProfile, HubMode, Transport};

const MAX_SUMMARY_ENTRIES: usize = 128;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) async fn collect(state: &AppState) -> Value {
    let generated_at = Utc::now();
    let config = state.config.read().await.clone();
    let (surface_tools, surface_revision) =
        crate::stdio_server::standalone_surface(state.runtime.profile);
    let active = sessions::current_sessions(state).await;
    let active_count = active.len();
    let resolved_limit = config.limits.max_active_sessions.resolve();
    let pending_count = state.pending_confirmations.lock().await.len();
    let (hub_sender, reporting_sender) =
        tokio::join!(async { state.hub_sender.lock().await.is_some() }, async {
            state.reporting_sender.lock().await.is_some()
        },);
    let (freedesktop_available, freedesktop_actions) =
        tokio::task::spawn_blocking(notify::detect_freedesktop_notification_support)
            .await
            .unwrap_or((false, false));
    let ntfy_available = match state.runtime.transport {
        Transport::Hub => hub_sender,
        Transport::TunnelStdio => reporting_sender,
    };
    let config_health = config_health(state, &config);
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
        &config.workspace_root,
    );
    let (read_only_roots, read_only_truncated) = path_values(
        config.path_policy.read_only_roots.iter(),
        &config.workspace_root,
    );
    let (deny_roots, deny_truncated) =
        path_values(config.path_policy.deny_roots.iter(), &config.workspace_root);
    let policy_summary = policy_summary(&config, state.runtime.profile);
    let paths_truncated = write_truncated || read_only_truncated || deny_truncated;
    let channels = config
        .confirmation_provider
        .channels
        .iter()
        .map(|channel| channel.as_str())
        .collect::<Vec<_>>();
    let providers = vec![
        json!({
            "id": "freedesktop",
            "configured": config.confirmation_provider.channels.contains(&ConfirmationChannel::Freedesktop),
            "available": freedesktop_available && freedesktop_actions,
            "supportsActions": freedesktop_actions,
            "reason": if !freedesktop_available { json!("notification_service_unavailable") } else if !freedesktop_actions { json!("actions_unavailable") } else { Value::Null },
        }),
        json!({
            "id": "ntfy",
            "configured": config.confirmation_provider.channels.contains(&ConfirmationChannel::Ntfy),
            "available": ntfy_available,
            "supportsActions": ntfy_available,
            "transport": "hub-relay",
            "deliveryHealth": "unknown",
            "reason": if ntfy_available { Value::Null } else { json!("hub_relay_disconnected") },
        }),
    ];
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
        "surface": {
            "toolCount": surface_tools.len(),
            "tools": surface_tools,
            "revision": surface_revision,
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
            "sessions": {
                "configuredMax": config.limits.max_active_sessions.configured_label(),
                "resolvedMax": resolved_limit.resolved,
                "active": active_count,
                "available": resolved_limit.resolved.saturating_sub(active_count),
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
        .to_string_lossy()
        .into_owned()
}

fn path_values<'a>(
    roots: impl Iterator<Item = &'a PathBuf>,
    workspace_root: &Path,
) -> (Vec<String>, bool) {
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
    if values.is_empty() {
        values.push(exact_path(workspace_root));
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
            return ConfigHealth {
                disk_status: "invalid",
                modified_at,
                live_subset_matches_disk: false,
                restart_required_fields: Vec::new(),
                error_code: Some("config_invalid".to_string()),
                issues: vec!["config_invalid"],
            }
        }
    };
    let live_subset_matches_disk = live_subset(effective) == live_subset(&disk);
    let restart_required_fields = restart_fields(effective, &disk);
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

fn live_subset(config: &Config) -> Value {
    json!({
        "policy": config.policy,
        "pathPolicy": config.path_policy,
        "limits": config.limits,
    })
}

fn restart_fields(effective: &Config, disk: &Config) -> Vec<String> {
    let pairs = [
        ("agentId", json!(effective.agent_id) != json!(disk.agent_id)),
        (
            "displayName",
            json!(effective.display_name) != json!(disk.display_name),
        ),
        ("hubUrl", json!(effective.hub_url) != json!(disk.hub_url)),
        (
            "hubTransport",
            json!(effective.hub_transport) != json!(disk.hub_transport),
        ),
        (
            "agentSecret",
            json!(effective.agent_secret) != json!(disk.agent_secret),
        ),
        (
            "workspaceRoot",
            json!(effective.workspace_root) != json!(disk.workspace_root),
        ),
        (
            "backupLimit",
            json!(effective.backup_limit) != json!(disk.backup_limit),
        ),
        (
            "confirmationProvider",
            json!(effective.confirmation_provider) != json!(disk.confirmation_provider),
        ),
        (
            "confirmationLanguage",
            json!(effective.confirmation_language) != json!(disk.confirmation_language),
        ),
        ("sandbox", json!(effective.sandbox) != json!(disk.sandbox)),
        (
            "mcpServers",
            json!(effective.mcp_servers) != json!(disk.mcp_servers),
        ),
        ("skills", json!(effective.skills) != json!(disk.skills)),
        ("room", json!(effective.room) != json!(disk.room)),
        ("tunnel", json!(effective.tunnel) != json!(disk.tunnel)),
    ];
    pairs
        .into_iter()
        .filter_map(|(name, differs)| differs.then_some(name.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::AgentMessage;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex, RwLock};

    fn state(profile: CapabilityProfile) -> AppState {
        let mut config = Config::default_config().unwrap();
        let root =
            std::env::temp_dir().join(format!("agent-info-{}", uuid::Uuid::new_v4().simple()));
        config.workspace_root = root.clone();
        config.path_policy.write_roots = vec![root];
        AppState {
            config_path: std::env::temp_dir().join("agent-info-missing-config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: crate::state::RuntimeModel::tunnel(profile, false),
            started_at: Utc::now(),
            supervised: true,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(sessions::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    #[tokio::test]
    async fn info_is_bounded_redacted_and_profile_correct() {
        let value = collect(&state(CapabilityProfile::Room)).await;
        assert_eq!(value["surface"]["toolCount"], 31);
        assert_eq!(value["identity"]["profile"], "room");
        assert_eq!(
            value["workspace"]["pathPolicy"]["writeRoots"][0].is_string(),
            true
        );
        assert_eq!(value["config"]["diskStatus"], "missing");
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("change-me"));
        assert!(!serialized.contains("agent_secret"));
    }

    #[tokio::test]
    async fn info_reports_restart_differences_and_current_ntfy_relay() {
        let mut app = state(CapabilityProfile::Normal);
        let disk_path = std::env::temp_dir().join(format!(
            "agent-info-config-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let disk = app.config.read().await.clone();
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
        assert_eq!(value["confirmation"]["providers"][1]["available"], true);
        assert_eq!(
            value["confirmation"]["providers"][1]["deliveryHealth"],
            "unknown"
        );
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
        app.config.write().await.limits.max_active_sessions =
            crate::config::MaxActiveSessions::Explicit(0);
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
