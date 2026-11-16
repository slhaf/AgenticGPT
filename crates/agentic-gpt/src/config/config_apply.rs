//! Native-value configuration edits. Transport authorization belongs to the API owner.
use std::{fs, path::Path};

use agentic_gpt_protocol::{
    ConfigChange, ConfigPreviewItem, ConfigUpdateChange, ConfigUpdateResponse,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::{
    config::{self, Config},
    config_catalog,
};

fn parent_mut<'a>(
    root: &'a mut Value,
    path: &[String],
    create: bool,
) -> Result<(&'a mut serde_json::Map<String, Value>, String)> {
    let (key, parents) = path
        .split_last()
        .ok_or_else(|| anyhow!("config_root_update_forbidden"))?;
    let mut node = root;
    for segment in parents {
        let object = node
            .as_object_mut()
            .ok_or_else(|| anyhow!("config_parent_not_object"))?;
        if !object.contains_key(segment) {
            if !create {
                return Err(anyhow!("config_parent_missing"));
            }
            object.insert(segment.clone(), json!({}));
        }
        node = object.get_mut(segment).expect("parent was inserted");
        if node.is_null() && create {
            *node = json!({});
        }
    }
    Ok((
        node.as_object_mut()
            .ok_or_else(|| anyhow!("config_parent_not_object"))?,
        key.clone(),
    ))
}

fn validate_tree(path: &mut Vec<String>, value: &Value) -> Result<()> {
    config_catalog::entry(path)?;
    if let Some(object) = value.as_object() {
        for (key, child) in object {
            path.push(key.clone());
            validate_tree(path, child)?;
            path.pop();
        }
    }
    Ok(())
}

fn apply_one(root: &mut Value, change: &ConfigChange) -> Result<()> {
    let path = change.path();
    let entry = config_catalog::entry(path)?;
    if path.is_empty() {
        return Err(anyhow!("config_root_update_forbidden"));
    }
    match change {
        ConfigChange::Set { value, .. } => {
            validate_tree(&mut path.to_vec(), value)?;
            let (parent, key) = parent_mut(root, path, true)?;
            parent.insert(key, value.clone());
        }
        ConfigChange::Unset { .. } => {
            if matches!(path, [key] if matches!(key.as_str(), "mode" | "profile" | "toolsets"))
                || path == ["toolsets", "enabled"]
            {
                return Err(anyhow!("config_required_field"));
            }
            // Removing an already absent known entry is idempotent.
            if root.pointer(&config_catalog::pointer(path)).is_some() {
                let (parent, key) = parent_mut(root, path, false)?;
                parent.remove(&key);
            }
        }
        ConfigChange::Add { value, .. } | ConfigChange::Remove { value, .. } => {
            if !entry.collection {
                return Err(anyhow!("config_collection_operation_unsupported"));
            }
            if entry.kind == "json-object-array" {
                if !value.is_object() {
                    return Err(anyhow!("config_collection_object_required"));
                }
            } else if !value.is_string() {
                return Err(anyhow!("config_collection_string_required"));
            }
            let pointer = config_catalog::pointer(path);
            if root.pointer(&pointer).is_none_or(Value::is_null) {
                if matches!(change, ConfigChange::Remove { .. }) {
                    return Ok(());
                }
                let initial = entry
                    .default_value
                    .as_ref()
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let (parent, key) = parent_mut(root, path, true)?;
                parent.insert(key, Value::Array(initial));
            }
            let array = root
                .pointer_mut(&pointer)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow!("config_array_required"))?;
            if matches!(change, ConfigChange::Add { .. }) {
                if !array.contains(value) {
                    array.push(value.clone());
                }
            } else {
                array.retain(|item| item != value);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate(config: &Config) -> Result<()> {
    config.validate_local()?;
    config::validate_hub_url_shape(&config.hub.url)?;
    config::validate_hub_transport(&config.hub.transport)?;
    if config.mode == config::RuntimeMode::Hub {
        config.validate_hub_connection()?;
    }
    config.validate_http_mcp()?;
    if config.room.diary_day_boundary_hour > 23 {
        return Err(anyhow!("invalid_diary_day_boundary_hour"));
    }
    config
        .room
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| anyhow!("room_timezone_invalid"))?;
    // Initialization may deliberately have pending mode-specific credentials.
    // A supplied Tunnel still has to have statically valid references and distribution settings.
    if config.tunnel.is_some() {
        config.validate_standalone()?;
    }
    Ok(())
}

pub(crate) fn apply_changes(base: &Config, changes: &[ConfigChange]) -> Result<Config> {
    let mut value = serde_json::to_value(base)?;
    for (index, change) in changes.iter().enumerate() {
        apply_one(&mut value, change).with_context(|| {
            format!(
                "config_change_invalid:{index}:{}",
                config_catalog::pointer(change.path())
            )
        })?;
    }
    let candidate = Config::from_value(value)?;
    validate(&candidate)?;
    Ok(candidate)
}

pub(crate) fn raw_locked(path: &Path) -> Result<Value> {
    if path.exists() {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Ok(json!({}))
    }
}

pub(crate) fn preview(
    base: &Config,
    candidate: &Config,
    changes: &[ConfigChange],
) -> Result<ConfigUpdateResponse> {
    let before = config_catalog::snapshot(base)?;
    let after = config_catalog::snapshot(candidate)?;
    let changes = changes
        .iter()
        .map(|change| {
            let path = change.path();
            let key = config_catalog::pointer(path);
            let (before, a) =
                config_catalog::redact(path, before.pointer(&key).unwrap_or(&Value::Null));
            let (after, b) =
                config_catalog::redact(path, after.pointer(&key).unwrap_or(&Value::Null));
            ConfigUpdateChange::Preview(ConfigPreviewItem {
                path: path.to_vec(),
                before,
                after,
                expected_effect: config_catalog::effect(path),
                redacted: a || b,
            })
        })
        .collect();
    Ok(ConfigUpdateResponse {
        saved: false,
        dry_run: true,
        changes,
        error: None,
    })
}

pub(crate) struct ConfigMutation {
    pub(crate) candidate: Config,
    pub(crate) raw: Value,
    pub(crate) response: ConfigUpdateResponse,
}

/// The caller owns acquire_config_mutation_lock for the entire transaction.
/// Recovery, validation and the single commit are shared by every writer adapter.
pub(crate) fn transaction_locked(
    path: &Path,
    changes: &[ConfigChange],
    dry_run: bool,
) -> Result<ConfigMutation> {
    crate::config_setup::recover_pending_setup_locked(path)?;
    let base = Config::load_or_default_locked(path)?;
    let candidate = apply_changes(&base, changes)?;
    let mut response = preview(&base, &candidate, changes)?;
    let raw = if dry_run {
        raw_locked(path)?
    } else {
        match config::write_config_with_backup(path, &candidate) {
            Ok(()) => {}
            Err(error)
                if error
                    .to_string()
                    .starts_with("config_commit_sync_failed_after_rename:") =>
            {
                response.error = Some(agentic_gpt_protocol::ConfigApiError {
                    code: "config_commit_sync_failed_after_rename".into(),
                    message: "配置已替换，目录持久化确认失败。".into(),
                    change_index: None,
                    path: None,
                });
            }
            Err(error) => return Err(error),
        }
        response.saved = true;
        response.dry_run = false;
        config::sparse_config_value(&candidate, false)?
    };
    Ok(ConfigMutation {
        candidate,
        raw,
        response,
    })
}

pub(crate) fn mutate_locked(
    path: &Path,
    changes: &[ConfigChange],
    dry_run: bool,
) -> Result<(Config, Value)> {
    let outcome = transaction_locked(path, changes, dry_run)?;
    Ok((outcome.candidate, serde_json::to_value(outcome.response)?))
}

/// Field-level intents for editing an existing configuration; used by the TUI transaction.
pub(crate) fn changes_between(before: &Config, after: &Config) -> Result<Vec<ConfigChange>> {
    fn walk(
        path: &mut Vec<String>,
        before: &Value,
        after: &Value,
        changes: &mut Vec<ConfigChange>,
    ) -> Result<()> {
        if before == after {
            return Ok(());
        }
        if let (Some(a), Some(b)) = (before.as_object(), after.as_object()) {
            for key in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                path.push(key.clone());
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => walk(path, x, y, changes)?,
                    (_, Some(y)) => {
                        if config_catalog::entry(path).is_ok() {
                            changes.push(ConfigChange::Set {
                                path: path.clone(),
                                value: y.clone(),
                            });
                        }
                    }
                    (Some(_), None) if config_catalog::entry(path).is_ok() => {
                        changes.push(ConfigChange::Unset { path: path.clone() });
                    }
                    _ => {}
                }
                path.pop();
            }
        } else if config_catalog::entry(path).is_ok() {
            if let (Some(a), Some(b)) = (before.as_array(), after.as_array()) {
                let unique = |items: &[Value]| {
                    items
                        .iter()
                        .enumerate()
                        .all(|(i, v)| !items[..i].contains(v))
                };
                let added = b
                    .iter()
                    .filter(|v| !a.contains(v))
                    .cloned()
                    .collect::<Vec<_>>();
                let appended = a
                    .iter()
                    .filter(|v| b.contains(v))
                    .cloned()
                    .chain(added.iter().cloned())
                    .collect::<Vec<_>>();
                if config_catalog::entry(path)?.collection
                    && unique(a)
                    && unique(b)
                    && appended == *b
                {
                    for value in a.iter().filter(|v| !b.contains(v)) {
                        changes.push(ConfigChange::Remove {
                            path: path.clone(),
                            value: value.clone(),
                        });
                    }
                    for value in added {
                        changes.push(ConfigChange::Add {
                            path: path.clone(),
                            value,
                        });
                    }
                    return Ok(());
                }
            }
            changes.push(ConfigChange::Set {
                path: path.clone(),
                value: after.clone(),
            });
        }
        Ok(())
    }
    let mut changes = Vec::new();
    walk(
        &mut Vec::new(),
        &serde_json::to_value(before)?,
        &serde_json::to_value(after)?,
        &mut changes,
    )?;
    Ok(changes)
}
