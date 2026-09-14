use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::ExplicitBrowserRuntimeConfig;

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
    pub(crate) trusted_code_paths: Vec<PathBuf>,
    pub(crate) docs_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NodeReplLaunchSpec {
    pub(crate) program: PathBuf,
    pub(crate) cwd: PathBuf,
    pub(crate) env_overrides: BTreeMap<String, String>,
}

pub(crate) fn build_node_repl_launch_spec(
    runtime: &BrowserRuntimeDescriptor,
    base_env: &BTreeMap<String, String>,
) -> Result<NodeReplLaunchSpec> {
    let cwd = runtime
        .browser_client_path
        .parent()
        .and_then(Path::parent)
        .filter(|path| {
            path.components()
                .any(|component| matches!(component, Component::Normal(_)))
        })
        .ok_or_else(|| anyhow!("browser_runtime_cwd_invalid"))?
        .to_path_buf();

    let node_path = path_to_env_string(&runtime.node_path, "node_path")?;
    let codex_home = path_to_env_string(&runtime.codex_home, "codex_home")?;
    let codex_cli_path = path_to_env_string(&runtime.codex_cli_path, "codex_cli_path")?;
    let browser_service_path =
        path_to_env_string(&runtime.browser_service_path, "browser_service_path")?;

    let mut trusted_code_paths = Vec::new();
    if let Some(value) = base_env.get("NODE_REPL_TRUSTED_CODE_PATHS") {
        trusted_code_paths.extend(std::env::split_paths(value));
    }
    for path in &runtime.trusted_code_paths {
        if !trusted_code_paths.contains(path) {
            trusted_code_paths.push(path.clone());
        }
    }
    let trusted_code_paths = std::env::join_paths(&trusted_code_paths)
        .map_err(|_| anyhow!("browser_runtime_trusted_code_paths_invalid"))?
        .to_str()
        .ok_or_else(|| anyhow!("browser_runtime_trusted_code_paths_invalid"))?
        .to_string();

    let mut env_overrides = base_env.clone();
    env_overrides.insert(
        "NODE_REPL_TRUSTED_CODE_PATHS".to_string(),
        trusted_code_paths,
    );
    env_overrides.insert("NODE_REPL_NODE_PATH".to_string(), node_path);
    env_overrides.insert("CODEX_HOME".to_string(), codex_home);
    env_overrides.insert("CODEX_CLI_PATH".to_string(), codex_cli_path);
    env_overrides.insert(
        "BROWSER_USE_CODEX_APP_VERSION".to_string(),
        runtime.app_version.clone(),
    );
    env_overrides.insert(
        "BROWSER_USE_CODEX_APP_BUILD_FLAVOR".to_string(),
        runtime.channel.clone(),
    );

    let mut trusted_services = match base_env.get("NODE_REPL_TRUSTED_SERVICES") {
        Some(value) => serde_json::from_str(value)
            .map_err(|_| anyhow!("browser_runtime_trusted_services_invalid"))?,
        None => Value::Object(serde_json::Map::new()),
    };
    trusted_services
        .as_object_mut()
        .ok_or_else(|| anyhow!("browser_runtime_trusted_services_invalid"))?
        .insert("browser".to_string(), Value::String(browser_service_path));
    let trusted_services = serde_json::to_string(&trusted_services)
        .map_err(|_| anyhow!("browser_runtime_trusted_services_invalid"))?;
    env_overrides.insert("NODE_REPL_TRUSTED_SERVICES".to_string(), trusted_services);

    if !runtime.node_module_dirs.is_empty() {
        let joined = std::env::join_paths(&runtime.node_module_dirs)
            .map_err(|_| anyhow!("browser_runtime_node_module_dirs_invalid"))?;
        let joined = joined
            .to_str()
            .ok_or_else(|| anyhow!("browser_runtime_path_not_utf8:node_module_dirs"))?;
        env_overrides.insert("NODE_REPL_NODE_MODULE_DIRS".to_string(), joined.to_string());
    }

    Ok(NodeReplLaunchSpec {
        program: runtime.node_repl_path.clone(),
        cwd,
        env_overrides,
    })
}

fn path_to_env_string(path: &Path, field: &str) -> Result<String> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("browser_runtime_path_not_utf8:{field}"))
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
    let trusted_code_paths = derive_trusted_code_paths(&codex_home, &node_module_dirs);
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
        trusted_code_paths,
        docs_root,
    })
}

pub(crate) fn explicit_runtime_descriptor(
    config: &ExplicitBrowserRuntimeConfig,
) -> Result<BrowserRuntimeDescriptor> {
    let app_version = explicit_string(config.app_version.as_deref(), "appVersion")?;
    let channel = explicit_string(config.channel.as_deref(), "channel")?;
    let node_repl_path = explicit_path(config.node_repl_path.as_deref(), "nodeReplPath")?;
    let node_path = explicit_path(config.node_path.as_deref(), "nodePath")?;
    let browser_client_path =
        explicit_path(config.browser_client_path.as_deref(), "browserClientPath")?;
    let browser_service_path =
        explicit_path(config.browser_service_path.as_deref(), "browserServicePath")?;
    let codex_home = explicit_path(config.codex_home.as_deref(), "codexHome")?;
    let codex_cli_path = explicit_path(config.codex_cli_path.as_deref(), "codexCliPath")?;
    let node_module_dirs = config
        .node_module_dirs
        .iter()
        .enumerate()
        .map(|(index, path)| explicit_path(Some(path), &format!("nodeModuleDirs[{index}]")))
        .collect::<Result<Vec<_>>>()?;
    let trusted_code_paths = derive_trusted_code_paths(&codex_home, &node_module_dirs);
    let docs_root = derive_docs_root(&browser_client_path)
        .map_err(|_| anyhow!("browser_runtime_explicit_docs_root_invalid"))?;

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
        trusted_code_paths,
        docs_root,
    })
}

fn explicit_string(value: Option<&str>, field: &str) -> Result<String> {
    let value = value.ok_or_else(|| anyhow!("browser_runtime_explicit_required:{field}"))?;
    if value.is_empty() {
        return Err(anyhow!("browser_runtime_explicit_empty:{field}"));
    }
    Ok(value.to_string())
}

fn explicit_path(value: Option<&str>, field: &str) -> Result<PathBuf> {
    let value = explicit_string(value, field)?;
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return Err(anyhow!("browser_runtime_explicit_relative:{field}"));
    }
    Ok(path)
}

fn derive_trusted_code_paths(codex_home: &Path, node_module_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(1 + node_module_dirs.len());
    for path in std::iter::once(codex_home).chain(node_module_dirs.iter().map(PathBuf::as_path)) {
        if !paths.contains(&path.to_path_buf()) {
            paths.push(path.to_path_buf());
        }
    }
    paths
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

    fn launch_descriptor(include_node_module_dirs: bool) -> BrowserRuntimeDescriptor {
        BrowserRuntimeDescriptor {
            app_version: "2.3.4".to_string(),
            channel: "stable".to_string(),
            node_repl_path: PathBuf::from("/bundle/scripts/node-repl.mjs"),
            node_path: PathBuf::from("/bundle/bin/node"),
            browser_client_path: PathBuf::from("/bundle/scripts/browser-client.mjs"),
            browser_service_path: PathBuf::from("/bundle/scripts/browser-service.mjs"),
            codex_home: PathBuf::from("/bundle/codex"),
            codex_cli_path: PathBuf::from("/bundle/bin/codex"),
            node_module_dirs: if include_node_module_dirs {
                vec![
                    PathBuf::from("/bundle/node_modules"),
                    PathBuf::from("/bundle/shared/node_modules"),
                ]
            } else {
                Vec::new()
            },
            trusted_code_paths: vec![PathBuf::from("/bundle/codex")],
            docs_root: PathBuf::from("/bundle/docs"),
        }
    }

    #[test]
    fn launch_spec_derives_program_and_cwd_from_descriptor() {
        let runtime = launch_descriptor(true);
        let spec = build_node_repl_launch_spec(&runtime, &BTreeMap::new()).unwrap();

        assert_eq!(spec.program, runtime.node_repl_path);
        assert_eq!(spec.cwd, PathBuf::from("/bundle"));
    }

    #[test]
    fn launch_spec_overwrites_stale_runtime_coupled_values() {
        let runtime = launch_descriptor(true);
        let mut base_env = BTreeMap::new();
        for key in [
            "NODE_REPL_NODE_PATH",
            "CODEX_HOME",
            "CODEX_CLI_PATH",
            "BROWSER_USE_CODEX_APP_VERSION",
            "BROWSER_USE_CODEX_APP_BUILD_FLAVOR",
            "NODE_REPL_NODE_MODULE_DIRS",
        ] {
            base_env.insert(key.to_string(), "stale".to_string());
        }

        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();

        assert_eq!(
            spec.env_overrides["NODE_REPL_NODE_PATH"],
            "/bundle/bin/node"
        );
        assert_eq!(spec.env_overrides["CODEX_HOME"], "/bundle/codex");
        assert_eq!(spec.env_overrides["CODEX_CLI_PATH"], "/bundle/bin/codex");
        assert_eq!(spec.env_overrides["BROWSER_USE_CODEX_APP_VERSION"], "2.3.4");
        assert_eq!(
            spec.env_overrides["BROWSER_USE_CODEX_APP_BUILD_FLAVOR"],
            "stable"
        );
        let expected = std::env::join_paths([
            PathBuf::from("/bundle/node_modules"),
            PathBuf::from("/bundle/shared/node_modules"),
        ])
        .unwrap()
        .into_string()
        .unwrap();
        assert_eq!(spec.env_overrides["NODE_REPL_NODE_MODULE_DIRS"], expected);
    }

    #[test]
    fn launch_spec_preserves_unrelated_base_environment() {
        let runtime = launch_descriptor(false);
        let base_env = BTreeMap::from([
            ("CUSTOM_SETTING".to_string(), "preserved".to_string()),
            (
                "NODE_REPL_TRUSTED_CODE_PATHS".to_string(),
                "/caller/path".to_string(),
            ),
        ]);

        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();

        assert_eq!(spec.env_overrides["CUSTOM_SETTING"], "preserved");
        let expected = std::env::join_paths([
            PathBuf::from("/caller/path"),
            PathBuf::from("/bundle/codex"),
        ])
        .unwrap()
        .into_string()
        .unwrap();
        assert_eq!(spec.env_overrides["NODE_REPL_TRUSTED_CODE_PATHS"], expected);
    }

    #[test]
    fn launch_spec_creates_trusted_code_paths_when_base_omits_them() {
        let runtime = launch_descriptor(false);
        let spec = build_node_repl_launch_spec(&runtime, &BTreeMap::new()).unwrap();

        assert_eq!(
            spec.env_overrides["NODE_REPL_TRUSTED_CODE_PATHS"],
            "/bundle/codex"
        );
    }

    #[test]
    fn launch_spec_merges_and_deduplicates_trusted_code_paths() {
        let mut runtime = launch_descriptor(true);
        runtime.trusted_code_paths = vec![
            PathBuf::from("/bundle/codex"),
            PathBuf::from("/bundle/node_modules"),
            PathBuf::from("/bundle/required"),
        ];
        let base_env = BTreeMap::from([(
            "NODE_REPL_TRUSTED_CODE_PATHS".to_string(),
            std::env::join_paths([
                PathBuf::from("/caller/first"),
                PathBuf::from("/bundle/codex"),
                PathBuf::from("/caller/last"),
            ])
            .unwrap()
            .into_string()
            .unwrap(),
        )]);

        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();
        let paths = std::env::split_paths(&spec.env_overrides["NODE_REPL_TRUSTED_CODE_PATHS"])
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/caller/first"),
                PathBuf::from("/bundle/codex"),
                PathBuf::from("/caller/last"),
                PathBuf::from("/bundle/node_modules"),
                PathBuf::from("/bundle/required"),
            ]
        );
    }

    #[test]
    fn launch_spec_replaces_browser_trusted_service_and_preserves_others() {
        let runtime = launch_descriptor(false);
        let mut base_env = BTreeMap::new();
        base_env.insert(
            "NODE_REPL_TRUSTED_SERVICES".to_string(),
            json!({ "browser": "stale", "other": "/other/service" }).to_string(),
        );

        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();
        let trusted_services: Value =
            serde_json::from_str(&spec.env_overrides["NODE_REPL_TRUSTED_SERVICES"]).unwrap();

        assert_eq!(
            trusted_services["browser"],
            "/bundle/scripts/browser-service.mjs"
        );
        assert_eq!(trusted_services["other"], "/other/service");
    }

    #[test]
    fn launch_spec_creates_browser_only_trusted_services_when_absent() {
        let runtime = launch_descriptor(false);
        let spec = build_node_repl_launch_spec(&runtime, &BTreeMap::new()).unwrap();
        let trusted_services: Value =
            serde_json::from_str(&spec.env_overrides["NODE_REPL_TRUSTED_SERVICES"]).unwrap();

        assert_eq!(
            trusted_services,
            json!({
                "browser": "/bundle/scripts/browser-service.mjs"
            })
        );
    }

    #[test]
    fn launch_spec_rejects_invalid_trusted_services() {
        let runtime = launch_descriptor(false);
        for value in ["{", "[]", "null", "\"service\""] {
            let base_env =
                BTreeMap::from([("NODE_REPL_TRUSTED_SERVICES".to_string(), value.to_string())]);
            let error = build_node_repl_launch_spec(&runtime, &base_env).unwrap_err();
            assert_eq!(
                error.to_string(),
                "browser_runtime_trusted_services_invalid"
            );
        }
    }

    #[test]
    fn launch_spec_empty_node_module_dirs_preserves_base_value() {
        let runtime = launch_descriptor(false);
        let base_env = BTreeMap::from([(
            "NODE_REPL_NODE_MODULE_DIRS".to_string(),
            "caller/modules".to_string(),
        )]);

        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();

        assert_eq!(
            spec.env_overrides["NODE_REPL_NODE_MODULE_DIRS"],
            "caller/modules"
        );
    }

    #[test]
    fn launch_spec_does_not_create_empty_node_module_dirs() {
        let runtime = launch_descriptor(false);
        let spec = build_node_repl_launch_spec(&runtime, &BTreeMap::new()).unwrap();

        assert!(!spec
            .env_overrides
            .contains_key("NODE_REPL_NODE_MODULE_DIRS"));
    }

    #[test]
    fn launch_spec_preserves_security_mode_without_inventing_it() {
        let runtime = launch_descriptor(false);
        let spec = build_node_repl_launch_spec(&runtime, &BTreeMap::new()).unwrap();
        assert!(!spec.env_overrides.contains_key("BROWSER_USE_SECURITY_MODE"));

        let base_env = BTreeMap::from([(
            "BROWSER_USE_SECURITY_MODE".to_string(),
            "caller-defined".to_string(),
        )]);
        let spec = build_node_repl_launch_spec(&runtime, &base_env).unwrap();
        assert_eq!(
            spec.env_overrides["BROWSER_USE_SECURITY_MODE"],
            "caller-defined"
        );
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
        assert_eq!(
            descriptor.trusted_code_paths,
            vec![
                PathBuf::from("/opt/openai/latest/codex"),
                PathBuf::from("/opt/openai/latest/node_modules"),
                PathBuf::from("/opt/openai/latest/shared/node_modules"),
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
    fn derives_ordered_deduplicated_trusted_code_paths() {
        let mut entry = valid_entry("2026-03-01T00:00:00Z", "latest", false);
        entry["paths"]["nodeModuleDirs"] = json!([
            "/opt/openai/latest/codex",
            "/opt/openai/latest/node_modules",
            "/opt/openai/latest/codex",
        ]);

        let descriptor = discover_fixture("trusted-code-paths", vec![entry]).unwrap();

        assert_eq!(
            descriptor.trusted_code_paths,
            vec![
                PathBuf::from("/opt/openai/latest/codex"),
                PathBuf::from("/opt/openai/latest/node_modules"),
            ]
        );
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

    fn explicit_config() -> ExplicitBrowserRuntimeConfig {
        ExplicitBrowserRuntimeConfig {
            app_version: Some("26.1.2".to_string()),
            channel: Some("prod".to_string()),
            node_repl_path: Some("/opt/runtime/bin/node_repl".to_string()),
            node_path: Some("/opt/runtime/bin/node".to_string()),
            browser_client_path: Some("/opt/runtime/chrome/scripts/browser-client.mjs".to_string()),
            browser_service_path: Some(
                "/opt/runtime/chrome/scripts/browser-service.mjs".to_string(),
            ),
            codex_home: Some("/opt/runtime/home".to_string()),
            codex_cli_path: Some("/opt/runtime/bin/codex".to_string()),
            node_module_dirs: vec![
                "/opt/runtime/modules".to_string(),
                "/opt/runtime/home".to_string(),
                "/opt/runtime/modules".to_string(),
            ],
        }
    }

    #[test]
    fn explicit_descriptor_derives_shared_docs_and_trusted_paths() {
        let descriptor = explicit_runtime_descriptor(&explicit_config()).unwrap();
        assert_eq!(
            descriptor.docs_root,
            PathBuf::from("/opt/runtime/chrome/docs")
        );
        assert_eq!(
            descriptor.trusted_code_paths,
            vec![
                PathBuf::from("/opt/runtime/home"),
                PathBuf::from("/opt/runtime/modules")
            ]
        );
    }

    #[test]
    fn explicit_descriptor_rejects_empty_and_relative_fields() {
        let mut config = explicit_config();
        config.node_path = Some(String::new());
        assert_eq!(
            explicit_runtime_descriptor(&config)
                .unwrap_err()
                .to_string(),
            "browser_runtime_explicit_empty:nodePath"
        );
        config.node_path = Some("relative/node".to_string());
        assert_eq!(
            explicit_runtime_descriptor(&config)
                .unwrap_err()
                .to_string(),
            "browser_runtime_explicit_relative:nodePath"
        );
        config.node_path = None;
        assert_eq!(
            explicit_runtime_descriptor(&config)
                .unwrap_err()
                .to_string(),
            "browser_runtime_explicit_required:nodePath"
        );
    }
}
