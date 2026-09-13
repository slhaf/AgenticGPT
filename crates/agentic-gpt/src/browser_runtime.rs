use anyhow::{anyhow, Result};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserRuntimeDescriptor {
    pub(crate) app_version: String,
    pub(crate) channel: String,
    pub(crate) node_repl_path: PathBuf,
    pub(crate) node_path: PathBuf,
    pub(crate) browser_client_path: PathBuf,
    pub(crate) browser_service_path: PathBuf,
    pub(crate) codex_home: PathBuf,
    pub(crate) codex_cli_path: PathBuf,
    pub(crate) node_module_dirs: Vec<PathBuf>,
    pub(crate) docs_root: PathBuf,
}

pub(crate) fn default_desktop_registry_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow!("browser_runtime_home_directory_unavailable"))?;
    Ok(home.join(".local/state/openai-codex/chrome-native-hosts-v2.json"))
}

pub(crate) fn discover_desktop_runtime(registry_path: &Path) -> Result<BrowserRuntimeDescriptor> {
    let bytes =
        fs::read(registry_path).map_err(|_| anyhow!("browser_runtime_registry_read_failed"))?;
    let registry: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("browser_runtime_registry_invalid_json"))?;
    let registry = registry
        .as_object()
        .ok_or_else(|| anyhow!("browser_runtime_registry_invalid_structure"))?;
    let entries = registry
        .get("entries")
        .ok_or_else(|| anyhow!("browser_runtime_entries_missing"))?
        .as_array()
        .ok_or_else(|| anyhow!("browser_runtime_entries_invalid"))?;
    if entries.is_empty() {
        return Err(anyhow!("browser_runtime_entries_empty"));
    }

    let selected = entries
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let updated_at = entry.get("updatedAt")?.as_str()?;
            (!updated_at.is_empty()).then_some((updated_at, entry))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, entry)| entry)
        .ok_or_else(|| anyhow!("browser_runtime_updated_at_unusable"))?;

    let app_version = required_string(selected.get("appVersion"), "appVersion")?;
    let channel = required_string(selected.get("channel"), "channel")?;
    required_string(selected.get("updatedAt"), "updatedAt")?;

    let paths = selected
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("browser_runtime_paths_invalid"))?;
    let node_repl_path = required_path(paths.get("nodeReplPath"), "paths.nodeReplPath")?;
    let node_path = required_path(paths.get("nodePath"), "paths.nodePath")?;
    let browser_client_path =
        required_path(paths.get("browserClientPath"), "paths.browserClientPath")?;
    let browser_service_path =
        required_path(paths.get("browserServicePath"), "paths.browserServicePath")?;
    let codex_home = required_path(paths.get("codexHome"), "paths.codexHome")?;
    let codex_cli_path = required_path(paths.get("codexCliPath"), "paths.codexCliPath")?;
    let node_module_dirs = optional_path_list(paths.get("nodeModuleDirs"))?;
    let docs_root = derive_docs_root(&browser_client_path)?;

    Ok(BrowserRuntimeDescriptor {
        app_version,
        channel,
        node_repl_path,
        node_path,
        browser_client_path,
        browser_service_path,
        codex_home,
        codex_cli_path,
        node_module_dirs,
        docs_root,
    })
}

fn required_string(value: Option<&Value>, field: &str) -> Result<String> {
    let value = value.ok_or_else(|| anyhow!("browser_runtime_required_field_missing:{field}"))?;
    let value = value
        .as_str()
        .ok_or_else(|| anyhow!("browser_runtime_required_field_invalid:{field}"))?;
    if value.is_empty() {
        return Err(anyhow!("browser_runtime_required_field_empty:{field}"));
    }
    Ok(value.to_string())
}

fn required_path(value: Option<&Value>, field: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required_string(value, field)?))
}

fn optional_path_list(value: Option<&Value>) -> Result<Vec<PathBuf>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("browser_runtime_node_module_dirs_invalid"))?;
    values
        .iter()
        .map(|value| {
            required_string(Some(value), "paths.nodeModuleDirs")
                .map(PathBuf::from)
                .map_err(|_| anyhow!("browser_runtime_node_module_dirs_invalid"))
        })
        .collect()
}

fn derive_docs_root(browser_client_path: &Path) -> Result<PathBuf> {
    let bundle_root = browser_client_path
        .parent()
        .and_then(|parent| parent.parent())
        .filter(|path| {
            path.components()
                .any(|component| matches!(component, Component::Normal(_)))
        })
        .ok_or_else(|| anyhow!("browser_runtime_docs_root_invalid"))?;
    Ok(bundle_root.join("docs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_path(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "agentic-gpt-browser-runtime-{label}-{}-{timestamp}-{id}.json",
            std::process::id()
        ))
    }

    fn write_fixture(label: &str, value: Value) -> PathBuf {
        let path = fixture_path(label);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        path
    }

    fn valid_entry(updated_at: &str, bundle: &str, include_node_module_dirs: bool) -> Value {
        let mut paths = json!({
            "nodeReplPath": format!("/opt/openai/{bundle}/scripts/node-repl.mjs"),
            "nodePath": format!("/opt/openai/{bundle}/bin/node"),
            "browserClientPath": format!("/opt/openai/{bundle}/scripts/browser-client.mjs"),
            "browserServicePath": format!("/opt/openai/{bundle}/scripts/browser-service.mjs"),
            "codexHome": format!("/opt/openai/{bundle}/codex"),
            "codexCliPath": format!("/opt/openai/{bundle}/bin/codex"),
        });
        if include_node_module_dirs {
            paths["nodeModuleDirs"] = json!([
                format!("/opt/openai/{bundle}/node_modules"),
                format!("/opt/openai/{bundle}/shared/node_modules")
            ]);
        }
        json!({
            "appVersion": format!("{bundle}-version"),
            "channel": "stable",
            "updatedAt": updated_at,
            "paths": paths,
        })
    }

    fn discover_fixture(label: &str, entries: Vec<Value>) -> Result<BrowserRuntimeDescriptor> {
        let path = write_fixture(label, json!({ "schemaVersion": 2, "entries": entries }));
        let result = discover_desktop_runtime(&path);
        let _ = fs::remove_file(path);
        result
    }

    #[test]
    fn selects_greatest_updated_at_and_maps_selected_entry() {
        let descriptor = discover_fixture(
            "selection",
            vec![
                valid_entry("2026-01-01T00:00:00Z", "older", true),
                valid_entry("2026-03-01T00:00:00Z", "latest", true),
            ],
        )
        .unwrap();

        assert_eq!(descriptor.app_version, "latest-version");
        assert_eq!(descriptor.channel, "stable");
        assert_eq!(
            descriptor.node_repl_path,
            PathBuf::from("/opt/openai/latest/scripts/node-repl.mjs")
        );
        assert_eq!(
            descriptor.node_path,
            PathBuf::from("/opt/openai/latest/bin/node")
        );
        assert_eq!(
            descriptor.browser_client_path,
            PathBuf::from("/opt/openai/latest/scripts/browser-client.mjs")
        );
        assert_eq!(
            descriptor.browser_service_path,
            PathBuf::from("/opt/openai/latest/scripts/browser-service.mjs")
        );
        assert_eq!(
            descriptor.codex_home,
            PathBuf::from("/opt/openai/latest/codex")
        );
        assert_eq!(
            descriptor.codex_cli_path,
            PathBuf::from("/opt/openai/latest/bin/codex")
        );
        assert_eq!(
            descriptor.node_module_dirs,
            vec![
                PathBuf::from("/opt/openai/latest/node_modules"),
                PathBuf::from("/opt/openai/latest/shared/node_modules")
            ]
        );

        let reversed = discover_fixture(
            "selection-reversed",
            vec![
                valid_entry("2026-03-01T00:00:00Z", "latest", true),
                valid_entry("2026-01-01T00:00:00Z", "older", true),
            ],
        )
        .unwrap();
        assert_eq!(reversed, descriptor);
    }

    #[test]
    fn derives_docs_root_from_browser_client_path() {
        let descriptor = discover_fixture(
            "docs-root",
            vec![valid_entry("2026-03-01T00:00:00Z", "latest", true)],
        )
        .unwrap();

        assert_eq!(
            descriptor.docs_root,
            PathBuf::from("/opt/openai/latest/docs")
        );
    }

    #[test]
    fn absent_node_module_dirs_defaults_to_empty() {
        let descriptor = discover_fixture(
            "node-modules-absent",
            vec![valid_entry("2026-03-01T00:00:00Z", "latest", false)],
        )
        .unwrap();

        assert!(descriptor.node_module_dirs.is_empty());
    }

    #[test]
    fn empty_entries_are_rejected() {
        let error = discover_fixture("entries-empty", Vec::new()).unwrap_err();
        assert_eq!(error.to_string(), "browser_runtime_entries_empty");
    }

    #[test]
    fn missing_or_empty_required_fields_are_rejected() {
        let mut missing_app_version = valid_entry("2026-03-01T00:00:00Z", "latest", false);
        missing_app_version
            .as_object_mut()
            .unwrap()
            .remove("appVersion");
        let error = discover_fixture("app-version-missing", vec![missing_app_version]).unwrap_err();
        assert!(error
            .to_string()
            .starts_with("browser_runtime_required_field_missing:appVersion"));

        let mut empty_node_path = valid_entry("2026-03-01T00:00:00Z", "latest", false);
        empty_node_path["paths"]["nodePath"] = json!("");
        let error = discover_fixture("node-path-empty", vec![empty_node_path]).unwrap_err();
        assert!(error
            .to_string()
            .starts_with("browser_runtime_required_field_empty:paths.nodePath"));
    }

    #[test]
    fn malformed_latest_entry_does_not_fall_back_to_older_valid_entry() {
        let latest = json!({
            "appVersion": "latest-version",
            "channel": "stable",
            "updatedAt": "2026-03-01T00:00:00Z",
            "paths": "malformed"
        });
        let error = discover_fixture(
            "latest-malformed",
            vec![valid_entry("2026-01-01T00:00:00Z", "older", false), latest],
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "browser_runtime_paths_invalid");
    }

    #[test]
    fn malformed_json_is_rejected_with_browser_runtime_context() {
        let path = fixture_path("malformed-json");
        fs::write(&path, b"{").unwrap();
        let error = discover_desktop_runtime(&path).unwrap_err();
        let _ = fs::remove_file(path);
        assert_eq!(error.to_string(), "browser_runtime_registry_invalid_json");
    }

    #[test]
    fn client_path_without_bundle_parent_is_rejected() {
        let mut entry = valid_entry("2026-03-01T00:00:00Z", "latest", false);
        entry["paths"]["browserClientPath"] = json!("scripts/browser-client.mjs");
        let error = discover_fixture("docs-root-invalid", vec![entry]).unwrap_err();
        assert_eq!(error.to_string(), "browser_runtime_docs_root_invalid");
    }
}
