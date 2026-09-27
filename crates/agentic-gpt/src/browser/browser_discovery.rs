// Browser runtime discovery and startup resolution.
use anyhow::{anyhow, Result};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use crate::{
    browser_distribution, browser_runtime, config::Config, state::BrowserRuntimeContext,
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

pub(crate) struct BrowserRuntimeSources {
    pub(crate) managed_cache_root: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) managed_codex_home: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) managed_target: Arc<dyn Fn() -> Result<&'static str> + Send + Sync>,
    pub(crate) managed_discover: Arc<
        dyn Fn(&Path, &str, &Path) -> Result<browser_runtime::BrowserRuntimeDescriptor>
            + Send
            + Sync,
    >,
    pub(crate) managed_provision:
        Arc<dyn Fn(String) -> BrowserRuntimeProvisionFuture + Send + Sync>,
    pub(crate) desktop_registry_path: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    pub(crate) desktop_discover:
        Arc<dyn Fn(&Path) -> Result<browser_runtime::BrowserRuntimeDescriptor> + Send + Sync>,
}

fn production_browser_runtime_sources() -> BrowserRuntimeSources {
    BrowserRuntimeSources {
        managed_cache_root: Arc::new(browser_distribution::managed_browser_cache_root),
        managed_codex_home: Arc::new(browser_distribution::managed_browser_codex_home),
        managed_target: Arc::new(browser_distribution::current_managed_target),
        managed_discover: Arc::new(browser_distribution::discover_managed_browser_runtime),
        managed_provision: Arc::new(|target| {
            Box::pin(async move {
                browser_distribution::provision_managed_browser_runtime(&target).await
            })
        }),
        desktop_registry_path: Arc::new(browser_runtime::default_desktop_registry_path),
        desktop_discover: Arc::new(browser_runtime::discover_desktop_runtime),
    }
}

pub(crate) async fn resolve_browser_runtime(config: &Config) -> Option<Arc<BrowserRuntimeContext>> {
    let sources = production_browser_runtime_sources();
    resolve_browser_runtime_with_sources(config, &sources).await
}

pub(crate) async fn resolve_browser_runtime_with_sources(
    config: &Config,
    sources: &BrowserRuntimeSources,
) -> Option<Arc<BrowserRuntimeContext>> {
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
                    match (sources.managed_provision)(target.to_owned()).await {
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
    _source: &str,
    descriptor: browser_runtime::BrowserRuntimeDescriptor,
) -> Result<Arc<BrowserRuntimeContext>> {
    let launch_spec = browser_runtime::build_node_repl_launch_spec(&descriptor, &BTreeMap::new())?;
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
