//! Agent-owned configuration API. All transports share admission, authorization and persistence.
use std::sync::{atomic::AtomicBool, Arc};

use agentic_gpt_protocol::{
    ConfigApiError, ConfigChange, ConfigGetRequest, ConfigGetResponse, ConfigItem,
    ConfigManualRequest, ConfigUpdateChange, ConfigUpdateRequest, ConfigUpdateResponse,
};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::{
    config::{self, Config, ConfigUpdatePolicy},
    config_apply, config_catalog,
    operation::{self, RequestContext, RequestIngress},
    state::AppState,
};

fn admit(state: &AppState, config: &Config, operation: &str) -> Result<()> {
    operation::authorize(
        state.runtime,
        config,
        RequestContext::new(RequestIngress::LocalUnix, operation),
    )
    .map_err(|error| anyhow!(error.code()))
}

pub(crate) async fn manual(state: &AppState, request: ConfigManualRequest) -> Result<Value> {
    admit(state, &*state.config.read().await, "config.manual")?;
    if request.section.as_ref().is_some_and(|s| s.len() > 128)
        || request.query.as_ref().is_some_and(|s| s.len() > 512)
    {
        return Err(anyhow!("config_manual_filter_too_long"));
    }
    Ok(serde_json::to_value(config_catalog::manual(&request)?)?)
}

fn error(code: &str, message: &str) -> ConfigApiError {
    ConfigApiError {
        code: code.into(),
        message: message.into(),
        change_index: None,
        path: None,
    }
}

async fn items(
    state: &AppState,
    desired: &Config,
    raw: &Value,
    paths: &[Vec<String>],
) -> Result<Vec<ConfigItem>> {
    let saved = config_catalog::snapshot(desired)?;
    let mut applied = config_catalog::snapshot(&*state.config.read().await)?;
    // HTTP settings reflect the listener/auth owner, not merely a watch message sent to it.
    if state.config_runtime.http_updates.read().await.is_some() {
        applied["httpMcp"] =
            serde_json::to_value(&*state.config_runtime.http_applied.read().await)?;
    }
    // A queued reconnect has selected desired settings, but the old connection
    // can still be delivering the update response. Project the concrete owner.
    let hub_applied = state.config_runtime.hub_applied.read().await.clone();
    let connection = state.config_runtime.hub_connection.read().await.clone();
    if hub_applied.is_some()
        || connection.target.is_some()
        || connection.status != agentic_gpt_protocol::ConfigApplyStatus::Applied
    {
        applied["hub"] = hub_applied
            .as_ref()
            .map_or(Value::Null, |config| json!(config.hub));
        if applied.get("tunnel").is_some_and(Value::is_object) {
            applied["tunnel"]["hubReporting"]["enabled"] = hub_applied
                .as_ref()
                .and_then(|config| config.tunnel.as_ref())
                .map_or(Value::Null, |tunnel| json!(tunnel.hub_reporting.enabled));
        }
    }
    let mut out = Vec::new();
    for path in paths {
        if !path.is_empty() {
            config_catalog::entry(path)?;
        }
        let key = config_catalog::pointer(path);
        let desired_value = saved.pointer(&key);
        let applied_value = applied.pointer(&key);
        let (value, redacted) = config_catalog::redact(path, desired_value.unwrap_or(&Value::Null));
        let (active, active_redacted) =
            config_catalog::redact(path, applied_value.unwrap_or(&Value::Null));
        let (apply_status, apply_error) = state
            .config_runtime
            .status(
                path,
                desired_value.unwrap_or(&Value::Null),
                applied_value.unwrap_or(&Value::Null),
            )
            .await;
        out.push(ConfigItem {
            path: path.clone(),
            value,
            explicit: raw.pointer(&key).is_some(),
            exists: desired_value.is_some(),
            redacted: redacted || active_redacted,
            applied_value: (desired_value != applied_value).then_some(active),
            apply_status,
            error: apply_error.map(|code| error(&code, "配置已保存，但组件尚未成功应用。")),
        });
    }
    Ok(out)
}

pub(crate) async fn get(state: &AppState, request: ConfigGetRequest) -> Result<Value> {
    admit(state, &*state.config.read().await, "config.get")?;
    if request.paths.is_empty() || request.paths.len() > 64 {
        return Err(anyhow!("config_paths_count_invalid"));
    }
    for path in &request.paths {
        if !path.is_empty() {
            config_catalog::entry(path)?;
        }
    }
    let (desired, raw) = {
        let _lock = config::acquire_config_mutation_lock(&state.config_path)?;
        crate::config_setup::recover_pending_setup_locked(&state.config_path)?;
        (
            Config::load_locked(&state.config_path)?,
            config_apply::raw_locked(&state.config_path)?,
        )
    };
    Ok(serde_json::to_value(ConfigGetResponse {
        items: items(state, &desired, &raw, &request.paths).await?,
    })?)
}

fn rejected(request: &ConfigUpdateRequest, err: ConfigApiError) -> Result<Value> {
    Ok(serde_json::to_value(ConfigUpdateResponse {
        saved: false,
        dry_run: request.dry_run,
        changes: vec![],
        error: Some(err),
    })?)
}

pub(crate) async fn update(
    state: &AppState,
    _context: &RequestContext<'_>,
    request: ConfigUpdateRequest,
) -> Result<Value> {
    let initial = state.config.read().await.clone();
    admit(state, &initial, "config.update")?;
    if request.changes.is_empty() || request.changes.len() > 64 {
        return rejected(
            &request,
            error(
                "config_changes_count_invalid",
                "一批修改须包含 1–64 个操作。",
            ),
        );
    }
    if serde_json::to_vec(&request)?.len() > 1024 * 1024 {
        return rejected(
            &request,
            error("config_request_too_large", "配置修改请求过大。"),
        );
    }
    for (index, change) in request.changes.iter().enumerate() {
        if config_catalog::entry(change.path()).is_err() {
            let mut err = error(
                "config_path_unknown",
                "配置路径不可用；请查询 config.manual。",
            );
            err.change_index = Some(index);
            err.path = Some(change.path().to_vec());
            return rejected(&request, err);
        }
    }
    let mut approved = false;
    loop {
        let effective = state.config.read().await.clone();
        admit(state, &effective, "config.update")?;
        if effective.config_api.update_policy == ConfigUpdatePolicy::Confirm && !approved {
            // This displays only paths/operations and redacted values. No file lock is held
            // while asking the human, and candidate policy cannot authorize itself.
            let preview = request
                .changes
                .iter()
                .map(|change| {
                    let value = change
                        .value()
                        .map(|value| config_catalog::redact(change.path(), value).0);
                    json!({"op":change.op_name(),"path":change.path(),"value":value})
                })
                .collect::<Vec<_>>();
            if let Err(reason) = crate::confirmation::authorize_sensitive_operation(
                state,
                &effective,
                crate::policy::PolicyDecision::Confirm,
                "config.update",
                "确认修改 Agent 配置",
                &serde_json::to_string_pretty(&preview)?,
                Arc::new(AtomicBool::new(false)),
            )
            .await
            {
                let code = if matches!(
                    reason.as_str(),
                    "deny" | "deny_once" | "rejected" | "reject" | "denied"
                ) {
                    "config_confirmation_rejected"
                } else {
                    "config_confirmation_unavailable"
                };
                return rejected(&request, error(code, "配置修改未取得明确批准，未保存。"));
            }
            approved = true;
        }
        let _serial = state.config_runtime.apply_lock.lock().await;
        let current = state.config.read().await.clone();
        admit(state, &current, "config.update")?;
        if current.config_api.update_policy == ConfigUpdatePolicy::Confirm && !approved {
            continue;
        }
        let persisted = {
            let _lock = config::acquire_config_mutation_lock(&state.config_path)?;
            config_apply::transaction_locked(&state.config_path, &request.changes, request.dry_run)
        };
        let outcome = match persisted {
            Ok(value) => value,
            Err(cause) => {
                let mut err = error(
                    "config_update_failed",
                    "修改或保存未成功；请检查配置路径、值及关联约束。",
                );
                if let Some(index) = cause
                    .to_string()
                    .strip_prefix("config_change_invalid:")
                    .and_then(|s| s.split(':').next())
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    err.change_index = Some(index);
                    err.path = request
                        .changes
                        .get(index)
                        .map(|change| change.path().to_vec());
                }
                return rejected(&request, err);
            }
        };
        if request.dry_run {
            return Ok(serde_json::to_value(outcome.response)?);
        }
        let candidate = outcome.candidate;
        let raw = outcome.raw;
        let mut warning = outcome.response.error;
        if crate::config_runtime::apply_locked(state, candidate.clone())
            .await
            .is_err()
        {
            warning = Some(error(
                "config_apply_failed",
                "配置已保存，本次运行态应用未全部完成。",
            ));
        }
        let paths = request
            .changes
            .iter()
            .map(ConfigChange::path)
            .map(<[String]>::to_vec)
            .collect::<Vec<_>>();
        let changes = items(state, &candidate, &raw, &paths)
            .await?
            .into_iter()
            .map(ConfigUpdateChange::Applied)
            .collect();
        return Ok(serde_json::to_value(ConfigUpdateResponse {
            saved: true,
            dry_run: false,
            changes,
            error: warning,
        })?);
    }
}
