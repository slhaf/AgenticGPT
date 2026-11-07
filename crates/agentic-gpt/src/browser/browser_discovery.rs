// Browser runtime discovery and startup resolution.
use anyhow::{anyhow, Result};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

use crate::{
    browser_distribution, browser_runtime,
    config::Config,
    state::{AppState, BrowserRuntimeContext},
    utils::log_info,
};

fn log_browser_runtime_unavailable(source: &str, stage: &str, error: &anyhow::Error) {
    log_info(format!(
        "browser runtime unavailable during startup; source={source}; stage={stage}; errorCode={}",
        crate::startup::error_code(&error.to_string())
    ));
}
pub(crate) type BrowserRuntimeProvisionFuture =
    Pin<Box<dyn Future<Output = Result<browser_runtime::BrowserRuntimeDescriptor>> + Send>>;

pub(crate) type ManagedBrowserRuntimeDiscover =
    dyn Fn(&Path, &str, &Path) -> Result<browser_runtime::BrowserRuntimeDescriptor> + Send + Sync;
pub(crate) type DesktopBrowserRuntimeDiscover =
    dyn Fn(&Path) -> Result<browser_runtime::BrowserRuntimeDescriptor> + Send + Sync;

pub(crate) struct BrowserRuntimeSources {
    pub(crate) managed_cache_root: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) managed_codex_home: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) managed_target: Arc<dyn Fn() -> Result<&'static str> + Send + Sync>,
    pub(crate) managed_discover: Arc<ManagedBrowserRuntimeDiscover>,
    pub(crate) managed_provision:
        Arc<dyn Fn(String) -> BrowserRuntimeProvisionFuture + Send + Sync>,
    pub(crate) desktop_registry_path: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) desktop_discover: Arc<DesktopBrowserRuntimeDiscover>,
}

fn production_browser_runtime_sources(cancellation: CancellationToken) -> BrowserRuntimeSources {
    BrowserRuntimeSources {
        managed_cache_root: Arc::new(browser_distribution::managed_browser_cache_root),
        managed_codex_home: Arc::new(browser_distribution::managed_browser_codex_home),
        managed_target: Arc::new(browser_distribution::current_managed_target),
        managed_discover: Arc::new(browser_distribution::discover_managed_browser_runtime),
        managed_provision: Arc::new(move |target| {
            let cancellation = cancellation.clone();
            Box::pin(async move {
                browser_distribution::provision_managed_browser_runtime(&target, cancellation).await
            })
        }),
        desktop_registry_path: Arc::new(browser_runtime::default_desktop_registry_path),
        desktop_discover: Arc::new(browser_runtime::discover_desktop_runtime),
    }
}

pub(crate) async fn resolve_browser_runtime(
    config: &Config,
    cancellation: CancellationToken,
) -> Option<Arc<BrowserRuntimeContext>> {
    let sources = production_browser_runtime_sources(cancellation.clone());
    let result = resolve_browser_runtime_with_sources(config, &sources, &cancellation).await;
    if let Some(runtime) = &result {
        if config.browser.runtime.is_none() && config.browser.managed.enabled {
            if let (Ok(root), Ok(target)) = (
                browser_distribution::managed_browser_cache_root(),
                browser_distribution::current_managed_target(),
            ) {
                if runtime
                    .descriptor
                    .node_path
                    .starts_with(root.join("artifacts").join(target))
                {
                    if let Err(error) = browser_distribution::clean_managed_browser_runtime_cache(
                        &root,
                        target,
                        &runtime.descriptor.codex_home,
                        &runtime.descriptor,
                        cancellation.clone(),
                    )
                    .await
                    {
                        if !cancellation.is_cancelled() {
                            log_browser_runtime_unavailable("managed-cache", "cleanup", &error);
                        }
                        return None;
                    }
                }
            }
        }
    }
    if cancellation.is_cancelled() {
        None
    } else {
        result
    }
}

pub(crate) async fn run_background(state: AppState, shutdown: CancellationToken) {
    let mut changes = state.browser_runtime.changed.subscribe();
    loop {
        changes.borrow_and_update();
        let config = state.config.read().await.clone();
        if config
            .toolsets
            .is_enabled(crate::config::ToolNamespace::Browser)
        {
            let cancellation = CancellationToken::new();
            let preparation = resolve_browser_runtime(&config, cancellation.clone());
            tokio::pin!(preparation);
            let runtime = tokio::select! {
                _ = shutdown.cancelled() => {
                    cancellation.cancel();
                    let _ = preparation.await;
                    return;
                }
                _ = changes.changed() => {
                    cancellation.cancel();
                    let _ = preparation.await;
                    continue;
                }
                result = &mut preparation => result,
            };
            if let Some(runtime) = runtime {
                let mut slot = state.browser_runtime.inner.lock().await;
                let current = state.config.read().await;
                if current.browser == config.browser
                    && current
                        .toolsets
                        .is_enabled(crate::config::ToolNamespace::Browser)
                    && !shutdown.is_cancelled()
                {
                    *slot = Some(runtime);
                    state.config_runtime.browser_ready().await;
                }
            }
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = changes.changed() => {}
        }
    }
}

pub(crate) async fn resolve_browser_runtime_with_sources(
    config: &Config,
    sources: &BrowserRuntimeSources,
    cancellation: &CancellationToken,
) -> Option<Arc<BrowserRuntimeContext>> {
    if !config
        .toolsets
        .is_enabled(crate::config::ToolNamespace::Browser)
    {
        return None;
    }

    if let Some(explicit) = config.browser.runtime.as_ref() {
        let descriptor = match browser_runtime::explicit_runtime_descriptor(explicit) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                log_browser_runtime_unavailable("explicit-config", "descriptor", &error);
                return None;
            }
        };
        return match browser_runtime_context("explicit-config", descriptor) {
            Ok(context) => Some(context),
            Err(error) => {
                log_browser_runtime_unavailable("explicit-config", "launch-spec", &error);
                None
            }
        };
    }

    if config.browser.managed.enabled {
        let target = match (sources.managed_target)() {
            Ok(target) => Some(target),
            Err(error) => {
                log_browser_runtime_unavailable("managed", "target", &error);
                None
            }
        };
        if let Some(target) = target {
            let codex_home = match (sources.managed_codex_home)() {
                Ok(path) => match prepare_managed_codex_home(&path) {
                    Ok(()) => Some(path),
                    Err(error) => {
                        log_browser_runtime_unavailable("managed", "codex-home", &error);
                        None
                    }
                },
                Err(error) => {
                    log_browser_runtime_unavailable("managed", "codex-home", &error);
                    None
                }
            };

            if let Some(codex_home) = codex_home {
                let cache_root = match (sources.managed_cache_root)() {
                    Ok(path) => Some(path),
                    Err(error) => {
                        log_browser_runtime_unavailable("managed-cache", "cache-root", &error);
                        None
                    }
                };

                if let Some(cache_root) = cache_root {
                    match (sources.managed_discover)(&cache_root, target, &codex_home) {
                        Ok(descriptor) => {
                            if let Err(error) = validate_managed_browser_descriptor(&descriptor) {
                                log_browser_runtime_unavailable(
                                    "managed-cache",
                                    "descriptor",
                                    &error,
                                );
                            } else {
                                match browser_runtime_context("managed-cache", descriptor) {
                                    Ok(context) => return Some(context),
                                    Err(error) => log_browser_runtime_unavailable(
                                        "managed-cache",
                                        "launch-spec",
                                        &error,
                                    ),
                                }
                            }
                        }
                        Err(error) => {
                            log_browser_runtime_unavailable("managed-cache", "discovery", &error)
                        }
                    }
                }

                if config.browser.managed.auto_provision {
                    let provision = (sources.managed_provision)(target.to_owned()).await;
                    if cancellation.is_cancelled() {
                        return None;
                    }
                    match provision {
                        Ok(descriptor) => {
                            if let Err(error) = validate_managed_browser_descriptor(&descriptor) {
                                log_browser_runtime_unavailable(
                                    "managed-provision",
                                    "descriptor",
                                    &error,
                                );
                            } else {
                                match browser_runtime_context("managed-provision", descriptor) {
                                    Ok(context) => return Some(context),
                                    Err(error) => log_browser_runtime_unavailable(
                                        "managed-provision",
                                        "launch-spec",
                                        &error,
                                    ),
                                }
                            }
                        }
                        Err(error) => log_browser_runtime_unavailable(
                            "managed-provision",
                            "provision",
                            &error,
                        ),
                    }
                }
            }
        }
    }

    let registry_path = match (sources.desktop_registry_path)() {
        Ok(path) => path,
        Err(error) => {
            log_browser_runtime_unavailable("desktop-registry", "registry-path", &error);
            return None;
        }
    };
    let descriptor = match (sources.desktop_discover)(&registry_path) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            log_browser_runtime_unavailable("desktop-registry", "discovery", &error);
            return None;
        }
    };
    match browser_runtime_context("desktop-registry", descriptor) {
        Ok(context) => Some(context),
        Err(error) => {
            log_browser_runtime_unavailable("desktop-registry", "launch-spec", &error);
            None
        }
    }
}

fn browser_runtime_context(
    source: &str,
    descriptor: browser_runtime::BrowserRuntimeDescriptor,
) -> Result<Arc<BrowserRuntimeContext>> {
    let mut launch_spec =
        browser_runtime::build_node_repl_launch_spec(&descriptor, &BTreeMap::new())?;
    if source.starts_with("managed-")
        && std::env::var("BROWSER_USE_SECURITY_MODE").as_deref() == Ok("disabled-for-local-testing")
        && std::env::var_os("CODEX_CLI_PATH").is_none()
        && !std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|directory| {
                launch_spec
                    .cwd
                    .join(directory)
                    .join("codex")
                    .metadata()
                    .is_ok_and(|metadata| {
                        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                    })
            })
        })
    {
        let executable = std::env::current_exe()
            .map_err(|error| anyhow!("browser_runtime_executable_unavailable:{error}"))?;
        launch_spec.env_overrides.insert(
            "CODEX_CLI_PATH".to_string(),
            executable
                .to_str()
                .ok_or_else(|| anyhow!("browser_runtime_path_not_utf8:agentic_executable"))?
                .to_string(),
        );
        launch_spec.env_overrides.insert(
            "AGENTIC_BROWSER_CONFIG_ADAPTER".to_string(),
            "1".to_string(),
        );
        launch_spec.env_overrides.insert(
            "BROWSER_USE_DISABLE_AMBIENT_NETWORK".to_string(),
            "1".to_string(),
        );
    }
    Ok(BrowserRuntimeContext::new(descriptor, launch_spec))
}

fn validate_managed_browser_descriptor(
    descriptor: &browser_runtime::BrowserRuntimeDescriptor,
) -> Result<()> {
    if descriptor.channel != "prod" {
        return Err(anyhow!("browser_runtime_managed_channel_invalid"));
    }
    if descriptor.codex_cli_path.is_some() {
        return Err(anyhow!("browser_runtime_managed_codex_cli_path_invalid"));
    }
    Ok(())
}

fn prepare_managed_codex_home(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(anyhow!("browser_runtime_managed_codex_home_invalid"));
    }
    fs::create_dir_all(path)
        .map_err(|_| anyhow!("browser_runtime_managed_codex_home_unavailable"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| anyhow!("browser_runtime_managed_codex_home_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!("browser_runtime_managed_codex_home_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow!("browser_runtime_managed_codex_home_unavailable"))?;
    }
    Ok(())
}
