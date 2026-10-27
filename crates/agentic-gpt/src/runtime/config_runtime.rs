//! Serial configuration application, with concrete service owners retaining their resources.
use std::{collections::BTreeMap, time::Duration};

use agentic_gpt_protocol::ConfigApplyStatus;
use anyhow::Result;
use serde_json::Value;
use tokio::sync::{watch, Mutex, Notify, RwLock};

use crate::{
    config::{Config, HttpMcpConfig},
    state::AppState,
};

#[derive(Default)]
pub(crate) struct ConfigRuntime {
    pub(crate) apply_lock: Mutex<()>,
    errors: RwLock<BTreeMap<String, String>>,
    pub(crate) http_attempted_config: RwLock<Option<HttpMcpConfig>>,
    pub(crate) hub_changed: Notify,
    pub(crate) hub_connection: RwLock<crate::hub::HubConnectionState>,
    pub(crate) hub_applied: RwLock<Option<Config>>,
    pub(crate) http_updates: RwLock<Option<watch::Sender<HttpMcpConfig>>>,
    pub(crate) http_applied: RwLock<Option<HttpMcpConfig>>,
    pub(crate) http_error: RwLock<Option<String>>,
    pub(crate) http_attempted: Notify,
}

/// Startup selectors remain fixed even when the desired file changes before a worker respawn.
pub(crate) fn preserve_startup(next: &mut Config, startup: &Config) {
    next.mode = startup.mode;
    next.profile = startup.profile;
    next.agent_id = startup.agent_id.clone();
    next.workspace_root = startup.workspace_root.clone();
    let reporting = next.tunnel.as_ref().map(|t| t.hub_reporting.clone());
    next.tunnel = startup.tunnel.clone();
    if let (Some(tunnel), Some(reporting)) = (&mut next.tunnel, reporting) {
        tunnel.hub_reporting = reporting;
    }
}

fn differs<T: serde::Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
}

pub(crate) async fn apply_locked(state: &AppState, desired: Config) -> Result<()> {
    let old = state.config.read().await.clone();
    let mut next = desired;
    preserve_startup(&mut next, &old);
    let mut errors = state.config_runtime.errors.read().await.clone();
    errors.remove("toolsets.enabled");
    if old.email.account == next.email.account {
        errors.remove("email.account");
    }
    if old.room.repository_root == next.room.repository_root {
        errors.remove("room.repositoryRoot");
    }
    let mut old_connection = old.room.secret.clone();
    old_connection.apply_live_settings(&next.room.secret);
    if !differs(&old_connection, &next.room.secret) {
        errors.remove("room.secret");
    }
    if !differs(&old.browser, &next.browser)
        && (!next
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Browser)
            || state.browser_runtime.inner.lock().await.is_some())
    {
        errors.remove("browser");
    }

    // Prepare service owners before publishing the new snapshot. Their guards exclude
    // old in-flight writes and new requests until the corresponding configuration is visible.
    let old_email_enabled = old
        .toolsets
        .is_enabled(crate::config::ToolNamespace::RoomEmail);
    let next_email_enabled = next
        .toolsets
        .is_enabled(crate::config::ToolNamespace::RoomEmail);
    let email_toolset_changed = old_email_enabled != next_email_enabled;
    let email_account_changed = old.email.account != next.email.account;
    let email_guard = if email_account_changed || email_toolset_changed {
        errors.remove("email.account");
        match state.email.prepare_reconfigure().await {
            Ok(guard) => Some(guard),
            Err(error) => {
                let error = crate::startup::error_code(&error.to_string());
                if email_account_changed {
                    next.email.account = old.email.account.clone();
                    errors.insert("email.account".into(), error.clone());
                }
                if email_toolset_changed {
                    if old_email_enabled {
                        next.toolsets
                            .enable(crate::config::ToolNamespace::RoomEmail);
                    } else {
                        next.toolsets
                            .disable(crate::config::ToolNamespace::RoomEmail);
                    }
                    errors.insert("toolsets.enabled".into(), error);
                }
                None
            }
        }
    } else {
        None
    };
    let mut old_secret_connection = old.room.secret.clone();
    old_secret_connection.apply_live_settings(&next.room.secret);
    let old_secret_enabled = old
        .toolsets
        .is_enabled(crate::config::ToolNamespace::RoomSecret);
    let next_secret_enabled = next
        .toolsets
        .is_enabled(crate::config::ToolNamespace::RoomSecret);
    let secret_toolset_changed = old_secret_enabled != next_secret_enabled;
    let secret_config_changed = differs(&old_secret_connection, &next.room.secret);
    let secret_guard = if secret_config_changed || secret_toolset_changed {
        errors.remove("room.secret");
        match state.secret.prepare_reconfigure().await {
            Ok(guard) => Some(guard),
            Err(error) => {
                let error = crate::startup::error_code(&error.to_string());
                if secret_config_changed {
                    let live_settings = next.room.secret.clone();
                    next.room.secret = old.room.secret.clone();
                    next.room.secret.apply_live_settings(&live_settings);
                    errors.insert("room.secret".into(), error.clone());
                }
                if secret_toolset_changed {
                    if old_secret_enabled {
                        next.toolsets
                            .enable(crate::config::ToolNamespace::RoomSecret);
                    } else {
                        next.toolsets
                            .disable(crate::config::ToolNamespace::RoomSecret);
                    }
                    errors.insert("toolsets.enabled".into(), error);
                }
                None
            }
        }
    } else {
        None
    };

    let room_changed = next.toolsets.uses_room_repository()
        && (old.room.repository_root != next.room.repository_root
            || !old.toolsets.uses_room_repository());
    let _room = if room_changed {
        Some(state.room_repository_writes.lock().await)
    } else {
        None
    };
    if room_changed {
        errors.remove("room.repositoryRoot");
        if let Err(error) = crate::room_repository::ensure_repository(&next) {
            next.room.repository_root = old.room.repository_root.clone();
            let mut toolset_changed = false;
            for namespace in crate::config::ToolNamespace::all()
                .iter()
                .copied()
                .filter(|namespace| namespace.uses_room_repository())
            {
                let was_enabled = old.toolsets.is_enabled(namespace);
                let is_enabled = next.toolsets.is_enabled(namespace);
                if was_enabled != is_enabled {
                    toolset_changed = true;
                    if was_enabled {
                        next.toolsets.enable(namespace);
                    } else {
                        next.toolsets.disable(namespace);
                    }
                }
            }
            if toolset_changed {
                errors.insert(
                    "toolsets.enabled".into(),
                    "room_repository_unavailable".into(),
                );
            }
            errors.insert(
                "room.repositoryRoot".into(),
                crate::startup::error_code(&error.to_string()),
            );
        }
    }

    let mut browser_guard = None;
    if differs(&old.browser, &next.browser)
        || old
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Browser)
            != next
                .toolsets
                .is_enabled(crate::config::ToolNamespace::Browser)
    {
        // Serialize lazy initialization with replacement. A failed preparation leaves the
        // old owner alive; shutdown never replays browser actions.
        errors.remove("browser");
        let mut slot = state.browser_runtime.inner.lock().await;
        let enabled = next
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Browser);
        let prepared = if enabled {
            tokio::time::timeout(
                Duration::from_secs(30),
                crate::browser_discovery::resolve_browser_runtime(&next),
            )
            .await
            .ok()
            .flatten()
        } else {
            None
        };
        if enabled && prepared.is_none() {
            next.browser = old.browser.clone();
            errors.insert("browser".into(), "browser_runtime_unavailable".into());
        } else {
            let stopped = if let Some(context) = slot.as_ref() {
                context.manager.shutdown().await
            } else {
                Ok(())
            };
            match stopped {
                Ok(()) => {
                    *slot = prepared;
                }
                Err(error) => {
                    let code = crate::startup::error_code(&error.to_string());
                    if code != "browser_runtime_busy" {
                        *slot = None;
                    }
                    next.browser = old.browser.clone();
                    errors.insert("browser".into(), code);
                }
            }
        }
        if let Some(error) = errors.get("browser").cloned() {
            if old
                .toolsets
                .is_enabled(crate::config::ToolNamespace::Browser)
                != next
                    .toolsets
                    .is_enabled(crate::config::ToolNamespace::Browser)
            {
                if old
                    .toolsets
                    .is_enabled(crate::config::ToolNamespace::Browser)
                {
                    next.toolsets.enable(crate::config::ToolNamespace::Browser);
                } else {
                    next.toolsets.disable(crate::config::ToolNamespace::Browser);
                }
                errors.insert("toolsets.enabled".into(), error);
            }
        }
        browser_guard = Some(slot);
    }
    let hub_changed =
        differs(&old.hub, &next.hub) || old.auxiliary_hub_enabled() != next.auxiliary_hub_enabled();
    let http_changed = differs(&old.http_mcp, &next.http_mcp);
    let tmux_enable = !old.toolsets.is_enabled(crate::config::ToolNamespace::Tmux)
        && next.toolsets.is_enabled(crate::config::ToolNamespace::Tmux);
    let skills_enable = !old
        .toolsets
        .is_enabled(crate::config::ToolNamespace::Skills)
        && next
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Skills);
    {
        let mut live = state.config.write().await;
        *live = next.clone();
        state
            .skill_installs
            .reconfigure(next.skills.max_concurrent_installs);
    }
    // Lazy discovery also clears the Browser error. Keep its slot locked while
    // publishing the error table so an old snapshot cannot resurrect that error.
    let unchanged_browser = if browser_guard.is_none() {
        Some(state.browser_runtime.inner.lock().await)
    } else {
        None
    };
    if unchanged_browser
        .as_ref()
        .is_some_and(|slot| slot.is_some())
    {
        errors.remove("browser");
    }
    *state.config_runtime.errors.write().await = errors;
    drop(unchanged_browser);
    drop(browser_guard);
    drop(email_guard);
    drop(secret_guard);
    if hub_changed {
        let mut connection = state.config_runtime.hub_connection.write().await;
        connection.status = ConfigApplyStatus::Applying;
        connection.error = None;
        drop(connection);
        state.config_runtime.hub_changed.notify_one();
    }
    if http_changed {
        if let Some(sender) = state.config_runtime.http_updates.read().await.as_ref() {
            *state.config_runtime.http_attempted_config.write().await = None;
            let _ = sender.send(next.http_mcp.clone());
            let attempted = async {
                loop {
                    let notified = state.config_runtime.http_attempted.notified();
                    if state
                        .config_runtime
                        .http_attempted_config
                        .read()
                        .await
                        .as_ref()
                        .is_some_and(|value| !differs(value, &next.http_mcp))
                    {
                        break;
                    }
                    notified.await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(3), attempted).await;
        } else if next.http_mcp.enabled {
            *state.config_runtime.http_error.write().await =
                Some("http_mcp_requires_standalone".into());
        } else {
            *state.config_runtime.http_error.write().await = None;
        }
    }
    if tmux_enable {
        let _ = crate::tmux::ensure_default_session(&next.workspace_root).await;
    }
    if skills_enable {
        state.skill_installs.resume_queued(state).await?;
    }
    Ok(())
}

impl ConfigRuntime {
    pub(crate) fn new(config: &Config, browser_available: bool) -> Self {
        let mut runtime = Self::default();
        if config
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Browser)
            && !browser_available
        {
            runtime
                .errors
                .get_mut()
                .insert("browser".into(), "browser_runtime_unavailable".into());
        }
        if config.mode == crate::config::RuntimeMode::Hub || config.auxiliary_hub_enabled() {
            runtime.hub_connection.get_mut().status = ConfigApplyStatus::Applying;
        }
        runtime
    }

    pub(crate) async fn browser_ready(&self) {
        self.errors.write().await.remove("browser");
    }

    pub(crate) async fn status(
        &self,
        path: &[String],
        desired: &Value,
        applied: &Value,
    ) -> (ConfigApplyStatus, Option<String>) {
        let related = |prefix: &str| {
            let prefix = prefix.split('.').collect::<Vec<_>>();
            path.iter()
                .map(String::as_str)
                .zip(prefix.iter().copied())
                .all(|(a, b)| a == b)
        };
        for (prefix, error) in self.errors.read().await.iter() {
            let secret_live = prefix == "room.secret"
                && matches!(
                    path.iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    [
                        "room",
                        "secret",
                        "getPolicy" | "upsertPolicy" | "deletePolicy" | "syncWaitTimeoutMs"
                    ]
                );
            if related(prefix) && !secret_live {
                return (ConfigApplyStatus::Failed, Some(error.clone()));
            }
        }
        if related("httpMcp") {
            if let Some(error) = self.http_error.read().await.clone() {
                return (ConfigApplyStatus::Failed, Some(error));
            }
        }
        if related("hub") || related("tunnel.hubReporting.enabled") {
            let connection = self.hub_connection.read().await;
            if connection.status != ConfigApplyStatus::Applied {
                return (connection.status, connection.error.clone());
            }
        }
        if desired != applied {
            let restart = crate::config_catalog::effect(path)
                == agentic_gpt_protocol::ConfigApplyEffect::RestartRequired
                || (path.is_empty()
                    && (["mode", "profile", "agentId", "workspaceRoot"]
                        .iter()
                        .any(|key| desired.get(*key) != applied.get(*key))
                        || {
                            let mut a = desired.get("tunnel").cloned().unwrap_or(Value::Null);
                            let mut b = applied.get("tunnel").cloned().unwrap_or(Value::Null);
                            if let Some(v) = a.as_object_mut() {
                                v.remove("hubReporting");
                            }
                            if let Some(v) = b.as_object_mut() {
                                v.remove("hubReporting");
                            }
                            a != b
                        }));
            (
                if restart {
                    ConfigApplyStatus::RestartRequired
                } else {
                    ConfigApplyStatus::Applying
                },
                None,
            )
        } else {
            (ConfigApplyStatus::Applied, None)
        }
    }
}
