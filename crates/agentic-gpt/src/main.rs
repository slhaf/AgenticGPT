mod agent_info;
mod audit;
mod bootstrap;
mod browser_distribution;
mod browser_kernel;
mod browser_manager;
mod browser_manual;
mod browser_runtime;
mod cli_i18n;
mod config;
mod config_cli;
mod config_setup;
mod config_templates;
mod config_tui;
mod confirmation;
mod exec;
mod file_ops;
mod http_oauth;
mod http_server;
mod hub;
mod instance_lock;
mod job_history;
mod jobs;
mod local_control;
mod local_service;
mod mcp;
mod notify;
mod operation;
mod operation_result;
mod policy;
mod private_state;
mod room_maintenance;
mod room_reads;
mod room_repository;

mod skill_installs;
mod skills;
mod state;
mod stdio_server;
mod supervisor;
mod tmux;
mod transport_ledger;
mod tui;
mod tunnel_distribution;
mod utils;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use cli_i18n::LanguageChoice;
use config::Config;
use config_cli::ConfigCommand;
#[cfg(test)]
use policy::PolicyDecision;
use serde_json::{Map, Value};
use state::{AppState, BrowserRuntimeContext, CapabilityProfile, RuntimeModel};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::future::Future;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{watch, Mutex, RwLock};
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use utils::{config_path, ensure_parent, log_info, log_warn};

pub(crate) use config::{RuntimeMode, WorkerProfile};

#[derive(Parser)]
#[command(name = "agentic-gpt")]
#[command(version)]
#[command(about = "Linux local agent for Agentic GPT")]
struct Cli {
    #[arg(long, global = true, value_enum, default_value_t = LanguageChoice::Auto)]
    language: LanguageChoice,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Local {
        #[arg(long, global = true)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: LocalCommand,
    },
    #[command(name = "stdio-worker", hide = true)]
    StdioWorker {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = WorkerProfile::Normal)]
        profile: WorkerProfile,
        #[arg(long, hide = true)]
        supervisor_token: Option<String>,
    },
    Config {
        #[arg(long, global = true)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Tui {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Tmux {
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: TmuxCommand,
    },
}

#[derive(Subcommand)]
enum LocalCommand {
    ListTools,
    Call {
        tool: String,
        #[arg(long, conflicts_with = "arguments_file")]
        arguments: Option<String>,
        #[arg(long, value_name = "PATH|-", conflicts_with = "arguments")]
        arguments_file: Option<String>,
    },
}

#[derive(Subcommand)]
enum TmuxCommand {
    List,
    Attach {
        session: String,
    },
    Create {
        name: String,
        #[arg(long)]
        cwd: String,
    },
    Close {
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = std::env::args_os().collect::<Vec<_>>();
    let choice = cli_i18n::prescan_language(&args).unwrap_or(LanguageChoice::Auto);
    let language = cli_i18n::resolve_language(choice, &cli_i18n::ProcessLocale);
    let cli = match cli_i18n::parse_cli(args, &cli_i18n::ProcessLocale) {
        Ok((cli, _)) => cli,
        Err(error) => cli_i18n::exit_with_cli_error(error, language),
    };
    match cli.command {
        Commands::Run { config } => run(config_path(config)).await,
        Commands::Local { config, command } => handle_local(config_path(config), command).await,
        Commands::StdioWorker {
            config,
            profile,
            supervisor_token,
        } => run_stdio_worker(config, profile.capability_profile(), supervisor_token).await,
        Commands::Config { config, command } => {
            config_cli::handle_config(config_path(config), command, language).await
        }
        Commands::Tui { config } => handle_tui(config_path(config), language).await,
        Commands::Tmux { config, command } => handle_tmux(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf) -> Result<()> {
    if !config_path.exists() {
        return Err(anyhow!("config_missing: run config init first"));
    }
    let config = Config::load(&config_path)?;
    match config.mode {
        RuntimeMode::Hub => run_hub(config_path).await,
        RuntimeMode::Standalone => supervisor::run(config_path).await,
        RuntimeMode::Local => run_local(config_path).await,
    }
}

async fn run_hub(config_path: PathBuf) -> Result<()> {
    ensure_parent(&config_path)?;
    let _instance_lock = instance_lock::InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    let initial = Config::load(&config_path)?;
    if initial.mode != RuntimeMode::Hub {
        return Err(anyhow!("runtime_mode_changed_before_start"));
    }
    let runtime = RuntimeModel::hub(initial.profile.capability_profile());
    log_info(format!(
        "agentic-gpt starting; runtime={}; hubMode={}; config={}",
        runtime.label(),
        runtime.hub_mode.label(),
        config_path.display(),
    ));
    initial.validate_hub()?;
    initial.ensure_workspace()?;
    if let Err(error) = tmux::ensure_default_session(&initial.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }
    log_info(format!(
        "config loaded; agentId={}; hub.url={}; workspaceRoot={}; sandbox={}; {}",
        initial.agent_id,
        initial.hub.url,
        initial.workspace_root.display(),
        if initial.sandbox.enabled {
            "enabled"
        } else {
            "disabled"
        },
        initial.limits.max_active_jobs.resolve().diagnostic()
    ));
    let browser_runtime = resolve_browser_runtime(&initial).await;
    let state = build_app_state(
        config_path.clone(),
        initial,
        runtime,
        false,
        browser_runtime,
    )?;
    state.skill_installs.recover(state.clone()).await?;
    tokio::spawn(watch_live_config(state.clone(), false, None));
    hub::connect_loop(state).await
}

async fn run_stdio_worker(
    config_path: PathBuf,
    profile: CapabilityProfile,
    supervisor_token: Option<String>,
) -> Result<()> {
    supervisor::authorize_worker(supervisor_token.as_deref())?;
    let supervised = supervisor_token.is_some();
    let config = Config::load(&config_path)?;
    if config.mode != RuntimeMode::Standalone || config.profile.capability_profile() != profile {
        return Err(anyhow!("stdio_worker_config_mismatch"));
    }
    config.validate_standalone()?;
    config.ensure_workspace()?;
    log_info(format!(
        "standalone worker config loaded; {}; policyAllow={}; policyConfirm={}; policyDeny={}; pathWriteRoots={}; pathReadOnlyRoots={}; pathDenyRoots={}",
        config.limits.max_active_jobs.resolve().diagnostic(),
        config.policy.allow.len(),
        config.policy.confirm.len(),
        config.policy.deny.len(),
        config.path_policy.write_roots.len(),
        config.path_policy.read_only_roots.len(),
        config.path_policy.deny_roots.len(),
    ));
    let reporting_enabled = config
        .tunnel
        .as_ref()
        .map(|tunnel| tunnel.hub_reporting.enabled)
        .unwrap_or(false);
    let agent_id = config.agent_id.clone();
    let browser_runtime = resolve_browser_runtime(&config).await;
    let state = build_app_state(
        config_path,
        config,
        RuntimeModel::tunnel(profile, reporting_enabled),
        supervised,
        browser_runtime,
    )?;
    state.skill_installs.recover(state.clone()).await?;
    let listener = local_control::bind(&agent_id).await?;
    log_info(format!(
        "local MCP ingress ready; transport=unix; path={}",
        listener.path().display()
    ));
    let initial_http_mcp = state.config.read().await.http_mcp.clone();
    let (http_updates, http_config) = watch::channel(initial_http_mcp);
    tokio::spawn(watch_live_config(
        state.clone(),
        supervised,
        Some(http_updates),
    ));
    if reporting_enabled {
        tokio::spawn(hub::connect_loop(state.clone()));
    }
    let mut local_task = tokio::spawn(listener.serve(state.clone()));
    let http_shutdown = CancellationToken::new();
    let mut http_task = tokio::spawn(http_server::run(
        state.clone(),
        http_config,
        http_shutdown.clone(),
    ));
    let stdio = stdio_server::serve_stdio(state);
    tokio::pin!(stdio);
    tokio::select! {
        result = &mut stdio => {
            local_task.abort();
            let _ = local_task.await;
            http_shutdown.cancel();
            let _ = http_task.await;
            result
        }
        result = &mut local_task => {
            http_shutdown.cancel();
            let _ = http_task.await;
            match result {
                Ok(result) => result,
                Err(_) => Err(anyhow!("local_mcp_listener_task_failed")),
            }
        }
        result = &mut http_task => {
            local_task.abort();
            let _ = local_task.await;
            match result {
                Ok(Ok(())) | Ok(Err(_)) | Err(_) => Err(anyhow!("http_mcp_server_task_failed")),
            }
        }
    }
}

const MAX_LOCAL_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;

fn log_browser_runtime_unavailable(source: &str, stage: &str, error: &anyhow::Error) {
    log_info(format!(
        "browser runtime unavailable during startup; source={source}; stage={stage}; errorCode={}",
        error_code(&error.to_string())
    ));
}
type BrowserRuntimeProvisionFuture =
    Pin<Box<dyn Future<Output = Result<browser_runtime::BrowserRuntimeDescriptor>> + Send>>;

struct BrowserRuntimeSources {
    managed_cache_root: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    managed_codex_home: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    managed_target: Arc<dyn Fn() -> Result<&'static str> + Send + Sync>,
    managed_discover: Arc<
        dyn Fn(&Path, &str, &Path) -> Result<browser_runtime::BrowserRuntimeDescriptor>
            + Send
            + Sync,
    >,
    managed_provision: Arc<dyn Fn(String) -> BrowserRuntimeProvisionFuture + Send + Sync>,
    desktop_registry_path: Arc<dyn Fn() -> Result<PathBuf> + Send + Sync>,
    desktop_discover:
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

async fn resolve_browser_runtime(config: &Config) -> Option<Arc<BrowserRuntimeContext>> {
    let sources = production_browser_runtime_sources();
    resolve_browser_runtime_with_sources(config, &sources).await
}

async fn resolve_browser_runtime_with_sources(
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

fn build_app_state(
    config_path: PathBuf,
    config: Config,
    runtime: RuntimeModel,
    supervised: bool,
    browser_runtime: Option<Arc<BrowserRuntimeContext>>,
) -> Result<AppState> {
    if config.toolsets.is_enabled(config::ToolNamespace::Room) {
        room_repository::ensure_repository(&config)?;
    }
    let max_concurrent_skill_installs = config.skills.max_concurrent_installs;
    let prepared = private_state::prepare(&config)?;
    for warning in &prepared.warnings {
        log_warn(warning.clone());
    }
    let private_state = prepared.paths;
    let job_history = job_history::JobHistoryStore::open(&private_state);
    let skill_installs_root = private_state.skill_installs.clone();
    Ok(AppState {
        config_path,
        config: Arc::new(RwLock::new(config)),
        private_state,
        job_history,
        browser_runtime,
        runtime,
        started_at: chrono::Utc::now(),
        boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        supervised,
        file_locks: Arc::new(Mutex::new(HashMap::new())),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        reporting_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        mcp_concurrency: Arc::new(jobs::McpConcurrency::new()),
        room_repository_writes: Arc::new(Mutex::new(())),
        skills_writes: Arc::new(Mutex::new(())),
        skill_leases: Arc::new(jobs::SkillLeaseManager::new()),
        skill_installs: Arc::new(skill_installs::InstallManager::with_concurrency(
            skill_installs_root,
            max_concurrent_skill_installs,
        )),
    })
}

async fn run_local(config_path: PathBuf) -> Result<()> {
    ensure_parent(&config_path)?;
    let _instance_lock = instance_lock::InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    let config = Config::load(&config_path)?;
    if config.mode != RuntimeMode::Local {
        return Err(anyhow!("runtime_mode_changed_before_start"));
    }
    let profile = config.profile.capability_profile();
    log_info(format!(
        "local agent starting; profile={}; config={}",
        profile.label(),
        config_path.display()
    ));
    config.validate_local()?;
    config.ensure_workspace()?;
    if let Err(error) = tmux::ensure_default_session(&config.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }
    let agent_id = config.agent_id.clone();
    let browser_runtime = resolve_browser_runtime(&config).await;
    let state = build_app_state(
        config_path,
        config,
        RuntimeModel::local(profile),
        false,
        browser_runtime,
    )?;
    state.skill_installs.recover(state.clone()).await?;
    tokio::spawn(watch_live_config(state.clone(), false, None));
    let listener = local_control::bind(&agent_id).await?;
    log_info(format!(
        "local MCP ingress ready; transport=unix; path={}",
        listener.path().display()
    ));
    let mut local_task = tokio::spawn(listener.serve(state));
    tokio::select! {
        result = &mut local_task => match result {
            Ok(result) => result,
            Err(_) => Err(anyhow!("local_mcp_listener_task_failed")),
        },
        signal = wait_for_local_shutdown_signal() => {
            signal?;
            local_task.abort();
            let _ = local_task.await;
            Ok(())
        }
    }
}

async fn wait_for_local_shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| anyhow!("local_shutdown_signal_failed"))?;
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|_| anyhow!("local_shutdown_signal_failed"))
        }
        _ = terminate.recv() => Ok(()),
    }
}

async fn handle_local(config_path: PathBuf, command: LocalCommand) -> Result<()> {
    let value = match command {
        LocalCommand::ListTools => local_control::list_tools(&config_path).await?,
        LocalCommand::Call {
            tool,
            arguments,
            arguments_file,
        } => {
            let arguments = read_local_arguments(arguments, arguments_file)?;
            local_control::call_tool(&config_path, tool, arguments).await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn read_local_arguments(
    inline: Option<String>,
    arguments_file: Option<String>,
) -> Result<Map<String, Value>> {
    let bytes = if let Some(inline) = inline {
        let bytes = inline.into_bytes();
        if bytes.len() > MAX_LOCAL_ARGUMENT_BYTES {
            return Err(anyhow!("local_arguments_too_large"));
        }
        bytes
    } else if let Some(source) = arguments_file {
        let reader: Box<dyn Read> = if source == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(fs::File::open(source).map_err(|_| anyhow!("local_arguments_unavailable"))?)
        };
        let mut bytes = Vec::new();
        reader
            .take((MAX_LOCAL_ARGUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("local_arguments_unavailable"))?;
        if bytes.len() > MAX_LOCAL_ARGUMENT_BYTES {
            return Err(anyhow!("local_arguments_too_large"));
        }
        bytes
    } else {
        b"{}".to_vec()
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("local_arguments_invalid_json"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("local_arguments_must_be_object"))
}

async fn handle_tui(config_path: PathBuf, language: cli_i18n::UiLanguage) -> Result<()> {
    if !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
        || !std::io::stderr().is_terminal()
    {
        return Err(anyhow!("tui_requires_tty"));
    }
    Config::load(&config_path)?;

    let (sender, receiver) = std::sync::mpsc::channel();
    let poll_path = config_path.clone();
    let poller = tokio::spawn(async move {
        loop {
            let client = match local_control::LocalJobClient::connect(&poll_path).await {
                Ok(client) => client,
                Err(error) => {
                    if sender
                        .send(tui::ProcessUpdate::Error(error.to_string()))
                        .is_err()
                    {
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            loop {
                match client.list_jobs(100).await {
                    Ok(page) => {
                        if sender.send(tui::ProcessUpdate::Jobs(page)).is_err() {
                            client.close().await;
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(tui::ProcessUpdate::Error(error.to_string()));
                        break;
                    }
                }
                sleep(Duration::from_millis(500)).await;
            }
            client.close().await;
            sleep(Duration::from_millis(500)).await;
        }
    });

    let screen = tui::ProcessScreen::new(receiver, language);
    let outcome = tui::TuiApp::process(screen, language).run();
    poller.abort();
    match outcome? {
        tui::TuiOutcome::Exited | tui::TuiOutcome::Cancelled => Ok(()),
        tui::TuiOutcome::ConfigCommitted(_) => Err(anyhow!("unexpected_tui_outcome")),
    }
}

async fn handle_tmux(config_path: PathBuf, command: TmuxCommand) -> Result<()> {
    use operation::{RequestContext, RequestIngress};

    let config = Config::load_or_default(&config_path)?;
    let operation = match &command {
        TmuxCommand::List => "tmux.listSessions",
        TmuxCommand::Attach { .. } => "tmux.attach",
        TmuxCommand::Create { .. } => "tmux.createSession",
        TmuxCommand::Close { .. } => "tmux.closeSession",
    };
    let context = RequestContext::new(RequestIngress::Cli, operation);
    operation::authorize(
        RuntimeModel::local(config.profile.capability_profile()),
        &config,
        context,
    )
    .map_err(|error| anyhow!(error.to_string()))?;
    match command {
        TmuxCommand::List => println!(
            "{}",
            serde_json::to_string_pretty(&tmux::list_sessions().await)?
        ),
        TmuxCommand::Attach { session } => tmux::attach(&session)
            .await
            .map_err(|error| anyhow!(error))?,
        TmuxCommand::Create { name, cwd } => {
            let result = tmux::create_session_for_cli(
                &config,
                agentic_gpt_protocol::TmuxCreateSessionRequest { name, cwd },
                context,
            )
            .await;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        TmuxCommand::Close { name } => {
            let result = tmux::close_session_for_cli(
                &config,
                agentic_gpt_protocol::TmuxCloseSessionRequest {
                    name,
                    need_confirm: false,
                },
                context,
            )
            .await;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

fn config_matches_runtime(config: &Config, runtime: RuntimeModel) -> bool {
    let mode = match runtime.transport {
        crate::state::Transport::Hub => RuntimeMode::Hub,
        crate::state::Transport::TunnelStdio => RuntimeMode::Standalone,
        crate::state::Transport::LocalUnix => RuntimeMode::Local,
    };
    config.mode == mode && config.profile.capability_profile() == runtime.profile
}

async fn watch_live_config(
    state: AppState,
    supervised: bool,
    http_updates: Option<watch::Sender<config::HttpMcpConfig>>,
) {
    let mut last_modified = fs::metadata(&state.config_path)
        .and_then(|meta| meta.modified())
        .ok();
    loop {
        sleep(Duration::from_secs(2)).await;
        let modified = fs::metadata(&state.config_path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_none() || modified == last_modified {
            continue;
        }
        last_modified = modified;

        let resolved = match reload_live_config_once(&state).await {
            Ok(resolved) => resolved,
            Err(error) => {
                if !supervised {
                    log_warn(format!(
                        "live config reload rejected; keeping previous subset; errorCode={}",
                        error_code(&error.to_string())
                    ));
                }
                continue;
            }
        };
        let live = state.config.read().await;
        if let Some(updates) = http_updates.as_ref() {
            let _ = updates.send(live.http_mcp.clone());
        }
        log_info(format!(
            "live config reloaded; {}; policyAllow={}; policyConfirm={}; policyDeny={}; pathWriteRoots={}; pathReadOnlyRoots={}; pathDenyRoots={}; mcpServers={}; httpMcpEnabled={}",
            resolved.diagnostic(),
            live.policy.allow.len(),
            live.policy.confirm.len(),
            live.policy.deny.len(),
            live.path_policy.write_roots.len(),
            live.path_policy.read_only_roots.len(),
            live.path_policy.deny_roots.len(),
            live.mcp_servers.len(),
            live.http_mcp.enabled,
        ));
    }
}

async fn reload_live_config_once(state: &AppState) -> Result<config::ResolvedMaxActiveJobs> {
    let candidate = Config::load(&state.config_path)?;
    let mut live = state.config.write().await;
    if !config_matches_runtime(&candidate, state.runtime) {
        return Err(anyhow!("runtime_selector_changed_restart_required"));
    }
    match state.runtime.transport {
        crate::state::Transport::TunnelStdio => candidate.validate_standalone()?,
        crate::state::Transport::LocalUnix => candidate.validate_local()?,
        crate::state::Transport::Hub => candidate.validate_mcp_servers()?,
    }
    let restart_required_fields = config::restart_required_fields(&live, &candidate);
    let live_room_enabled = live.toolsets.is_enabled(config::ToolNamespace::Room);
    let candidate_room_enabled = candidate.toolsets.is_enabled(config::ToolNamespace::Room);
    if candidate_room_enabled && !live_room_enabled {
        room_repository::ensure_repository(&live)?;
    }
    let resolved = apply_live_config_subset(&mut live, candidate);
    if !restart_required_fields.is_empty() {
        log_warn(format!(
            "config changes require restart; fields={}",
            restart_required_fields.join(",")
        ));
    }
    Ok(resolved)
}

fn apply_live_config_subset(live: &mut Config, candidate: Config) -> config::ResolvedMaxActiveJobs {
    let resolved = candidate.limits.max_active_jobs.resolve();
    let workspace_matches = live.workspace_root == candidate.workspace_root;
    live.policy = candidate.policy;
    if workspace_matches {
        live.path_policy = candidate.path_policy;
    }
    live.limits = candidate.limits;
    live.mcp_servers = candidate.mcp_servers;
    live.toolsets = candidate.toolsets;
    live.http_mcp = candidate.http_mcp;
    resolved
}

fn error_code(value: &str) -> String {
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .find(|part| !part.is_empty())
        .unwrap_or("config_reload_failed")
        .chars()
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PathPolicyConfig, Rule, TunnelConfig};
    use crate::config_cli::{PathRootCommand, PathRootKind};
    use crate::exec::PreparedBatchElement;
    use crate::mcp::McpServerConfig;
    use agentic_gpt_protocol::{
        AgentMessage, BootstrapReadRequest, HubCommand, NotebookAppendRequest, PassageSignificance,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn explicit_browser_runtime_config() -> config::ExplicitBrowserRuntimeConfig {
        config::ExplicitBrowserRuntimeConfig {
            app_version: Some("26.1.2".to_string()),
            channel: Some("prod".to_string()),
            node_repl_path: Some("/opt/runtime/node_repl".to_string()),
            node_path: Some("/opt/runtime/node".to_string()),
            browser_client_path: Some("/opt/runtime/chrome/scripts/browser-client.mjs".to_string()),
            browser_service_path: Some(
                "/opt/runtime/chrome/scripts/browser-service.mjs".to_string(),
            ),
            codex_home: Some("/opt/runtime/home".to_string()),
            codex_cli_path: Some("/opt/runtime/codex".to_string()),
            node_module_dirs: Vec::new(),
        }
    }

    #[tokio::test]
    async fn explicit_runtime_precedes_desktop_discovery_and_invalid_does_not_fallback() {
        let mut config = Config::default_config().unwrap();
        config.browser.runtime = Some(explicit_browser_runtime_config());
        let context = resolve_browser_runtime(&config)
            .await
            .expect("explicit runtime selected");
        assert_eq!(context.descriptor.app_version, "26.1.2");
        config.browser.runtime.as_mut().unwrap().node_path = Some("relative/node".to_string());
        assert!(resolve_browser_runtime(&config).await.is_none());
    }

    #[test]
    fn invalid_explicit_runtime_keeps_app_startup_fail_open() {
        let mut config = Config::default_config().unwrap();
        config.browser.runtime = Some(explicit_browser_runtime_config());
        config.browser.runtime.as_mut().unwrap().node_path = Some("relative/node".to_string());
        let root = unique_temp_dir("explicit-browser-fail-open");
        config.workspace_root = root.join("workspace");
        let state = build_app_state(
            root.join("config.json"),
            config,
            RuntimeModel::local(CapabilityProfile::Normal),
            false,
            None,
        )
        .unwrap();
        assert!(state.browser_runtime.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn managed_config(auto_provision: bool) -> Config {
        let mut config = Config::default_config().unwrap();
        config.browser.runtime = None;
        config.browser.managed.enabled = true;
        config.browser.managed.auto_provision = auto_provision;
        config
    }

    fn test_browser_descriptor(
        label: &str,
        codex_home: &Path,
        codex_cli_path: Option<&str>,
    ) -> browser_runtime::BrowserRuntimeDescriptor {
        let root = PathBuf::from(format!("/tmp/agentic-browser-source-{label}"));
        browser_runtime::BrowserRuntimeDescriptor {
            app_version: format!("test-{label}"),
            channel: "prod".to_string(),
            node_repl_path: root.join("cua_node/bin/node_repl"),
            node_path: root.join("cua_node/bin/node"),
            browser_client_path: root.join("chrome/scripts/browser-client.mjs"),
            browser_service_path: root.join("chrome/scripts/browser-service.mjs"),
            codex_home: codex_home.to_path_buf(),
            codex_cli_path: codex_cli_path.map(PathBuf::from),
            node_module_dirs: Vec::new(),
            trusted_code_paths: vec![codex_home.to_path_buf()],
            docs_root: root.join("chrome/docs"),
        }
    }

    fn injected_browser_sources(
        cache_root: PathBuf,
        codex_home: PathBuf,
        cache: Option<browser_runtime::BrowserRuntimeDescriptor>,
        provision: Option<browser_runtime::BrowserRuntimeDescriptor>,
        desktop: Option<browser_runtime::BrowserRuntimeDescriptor>,
        provision_calls: Arc<AtomicUsize>,
        desktop_calls: Arc<AtomicUsize>,
    ) -> BrowserRuntimeSources {
        let cache = Arc::new(cache);
        let provision = Arc::new(provision);
        let desktop = Arc::new(desktop);
        BrowserRuntimeSources {
            managed_cache_root: Arc::new(move || Ok(cache_root.clone())),
            managed_codex_home: Arc::new(move || Ok(codex_home.clone())),
            managed_target: Arc::new(|| Ok("test-target")),
            managed_discover: Arc::new(move |_, _, _| {
                cache
                    .as_ref()
                    .clone()
                    .ok_or_else(|| anyhow!("test_managed_cache_miss"))
            }),
            managed_provision: Arc::new(move |_| {
                provision_calls.fetch_add(1, Ordering::SeqCst);
                let provision = provision.clone();
                Box::pin(async move {
                    provision
                        .as_ref()
                        .clone()
                        .ok_or_else(|| anyhow!("test_managed_provision_failed"))
                })
            }),
            desktop_registry_path: Arc::new(|| {
                Ok(PathBuf::from("/tmp/test-desktop-registry.json"))
            }),
            desktop_discover: Arc::new(move |_| {
                desktop_calls.fetch_add(1, Ordering::SeqCst);
                desktop
                    .as_ref()
                    .clone()
                    .ok_or_else(|| anyhow!("test_desktop_unavailable"))
            }),
        }
    }

    #[tokio::test]
    async fn managed_cache_precedes_desktop_and_preserves_descriptor_fields() {
        let root = unique_temp_dir("browser-source-cache");
        let codex_home = root.join("browser-runtime").join("codex-home");
        let provision_calls = Arc::new(AtomicUsize::new(0));
        let desktop_calls = Arc::new(AtomicUsize::new(0));
        let cached = test_browser_descriptor("cache", &codex_home, None);
        let desktop = test_browser_descriptor("desktop", &codex_home, Some("/tmp/desktop-cli"));
        let sources = injected_browser_sources(
            root.clone(),
            codex_home.clone(),
            Some(cached),
            Some(test_browser_descriptor(
                "provision",
                &codex_home,
                Some("/tmp/provision-cli"),
            )),
            Some(desktop),
            provision_calls.clone(),
            desktop_calls.clone(),
        );

        let context = resolve_browser_runtime_with_sources(&managed_config(false), &sources)
            .await
            .expect("managed cache selected");
        assert_eq!(context.descriptor.app_version, "test-cache");
        assert_eq!(context.descriptor.channel, "prod");
        assert_eq!(context.descriptor.codex_home, codex_home);
        assert_eq!(context.descriptor.codex_cli_path, None);
        assert_eq!(provision_calls.load(Ordering::SeqCst), 0);
        assert_eq!(desktop_calls.load(Ordering::SeqCst), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(context.descriptor.codex_home.clone())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn auto_provision_is_gated_and_failures_fall_back_to_desktop() {
        let root = unique_temp_dir("browser-source-provision");
        let codex_home = root.join("codex-home");
        let provision_calls = Arc::new(AtomicUsize::new(0));
        let desktop_calls = Arc::new(AtomicUsize::new(0));
        let provisioned = test_browser_descriptor("provision", &codex_home, None);
        let desktop = test_browser_descriptor("desktop", &codex_home, None);

        let enabled_sources = injected_browser_sources(
            root.clone(),
            codex_home.clone(),
            None,
            Some(provisioned),
            Some(desktop.clone()),
            provision_calls.clone(),
            desktop_calls.clone(),
        );
        let context = resolve_browser_runtime_with_sources(&managed_config(true), &enabled_sources)
            .await
            .expect("managed provision selected");
        assert_eq!(context.descriptor.app_version, "test-provision");
        assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
        assert_eq!(desktop_calls.load(Ordering::SeqCst), 0);

        let disabled_sources = injected_browser_sources(
            root.clone(),
            codex_home.clone(),
            None,
            Some(test_browser_descriptor(
                "disabled-provision",
                &codex_home,
                None,
            )),
            Some(desktop),
            provision_calls.clone(),
            desktop_calls.clone(),
        );
        let context =
            resolve_browser_runtime_with_sources(&managed_config(false), &disabled_sources)
                .await
                .expect("desktop fallback selected");
        assert_eq!(context.descriptor.app_version, "test-desktop");
        assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
        assert_eq!(desktop_calls.load(Ordering::SeqCst), 1);

        let failed_sources = injected_browser_sources(
            root.clone(),
            codex_home.clone(),
            None,
            None,
            Some(test_browser_descriptor("failed-desktop", &codex_home, None)),
            provision_calls.clone(),
            desktop_calls.clone(),
        );
        let context = resolve_browser_runtime_with_sources(&managed_config(true), &failed_sources)
            .await
            .expect("desktop fallback after provisioning failure");
        assert_eq!(context.descriptor.app_version, "test-failed-desktop");
        assert_eq!(provision_calls.load(Ordering::SeqCst), 2);
        assert_eq!(desktop_calls.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unavailable_sources_fail_open_without_host_io() {
        let root = unique_temp_dir("browser-source-unavailable");
        let codex_home = root.join("codex-home");
        let provision_calls = Arc::new(AtomicUsize::new(0));
        let desktop_calls = Arc::new(AtomicUsize::new(0));
        let sources = injected_browser_sources(
            root.clone(),
            codex_home,
            None,
            None,
            None,
            provision_calls.clone(),
            desktop_calls.clone(),
        );
        assert!(
            resolve_browser_runtime_with_sources(&managed_config(true), &sources)
                .await
                .is_none()
        );
        assert_eq!(provision_calls.load(Ordering::SeqCst), 1);
        assert_eq!(desktop_calls.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn builder_accepts_pre_resolved_browser_context_without_discovery() {
        let root = unique_temp_dir("browser-builder-resolved");
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root.join("workspace");
        let descriptor = test_browser_descriptor("builder", &root.join("codex-home"), None);
        let launch_spec =
            browser_runtime::build_node_repl_launch_spec(&descriptor, &BTreeMap::new()).unwrap();
        let context = BrowserRuntimeContext::new(descriptor, launch_spec);
        let state = build_app_state(
            root.join("config.json"),
            config,
            RuntimeModel::local(CapabilityProfile::Normal),
            false,
            Some(context),
        )
        .unwrap();
        assert_eq!(
            state
                .browser_runtime
                .as_ref()
                .unwrap()
                .descriptor
                .app_version,
            "test-builder"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cli_version_uses_crate_version() {
        let error = match Cli::try_parse_from(["agentic-gpt", "--version"]) {
            Ok(_) => panic!("--version unexpectedly parsed as a runnable command"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
        let rendered = error.to_string();
        assert!(rendered.contains("agentic-gpt 0.9.1"));
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn sse_post_status_classification_stops_on_stale_connection() {
        assert_eq!(
            hub::classify_sse_post_status(reqwest::StatusCode::OK),
            hub::SsePostStatus::Delivered
        );
        assert_eq!(
            hub::classify_sse_post_status(reqwest::StatusCode::CONFLICT),
            hub::SsePostStatus::Stale
        );
        assert_eq!(
            hub::classify_sse_post_status(reqwest::StatusCode::BAD_GATEWAY),
            hub::SsePostStatus::Retry
        );
    }

    #[test]
    fn run_as_room_uses_workspace_default_repository_root() {
        let config = Config::default_config().unwrap();
        assert_eq!(config.agent_id, "laptop");
        assert_eq!(
            room_repository::repository_root(&config),
            config.workspace_root.join("room")
        );
    }

    #[test]
    fn public_run_has_only_a_config_path_and_no_profile_override() {
        let cli = Cli::try_parse_from(["agentic-gpt", "run"]).unwrap();
        assert!(matches!(cli.command, Commands::Run { config: None }));
        assert!(Cli::try_parse_from(["agentic-gpt", "run", "--profile", "room"]).is_err());
    }

    #[test]
    fn local_cli_accepts_config_before_or_after_subcommand() {
        for args in [
            vec![
                "agentic-gpt",
                "local",
                "--config",
                "/tmp/local.json",
                "list-tools",
            ],
            vec![
                "agentic-gpt",
                "local",
                "list-tools",
                "--config",
                "/tmp/local.json",
            ],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(
                cli.command,
                Commands::Local {
                    config: Some(ref path),
                    command: LocalCommand::ListTools,
                } if path == &PathBuf::from("/tmp/local.json")
            ));
        }
    }

    #[test]
    fn local_arguments_are_bounded_objects_from_inline_or_file() {
        assert!(read_local_arguments(None, None).unwrap().is_empty());
        let inline = read_local_arguments(Some(r#"{"value":"ok"}"#.to_string()), None).unwrap();
        assert_eq!(inline["value"], "ok");

        let root = unique_temp_dir("local-arguments");
        let path = root.join("args.json");
        fs::write(&path, br#"{"fromFile":true}"#).unwrap();
        let from_file =
            read_local_arguments(None, Some(path.to_string_lossy().into_owned())).unwrap();
        assert_eq!(from_file["fromFile"], true);

        assert!(read_local_arguments(Some("[]".to_string()), None)
            .unwrap_err()
            .to_string()
            .starts_with("local_arguments_must_be_object"));
        assert!(read_local_arguments(Some("{".to_string()), None)
            .unwrap_err()
            .to_string()
            .starts_with("local_arguments_invalid_json"));
        assert!(
            read_local_arguments(Some("x".repeat(MAX_LOCAL_ARGUMENT_BYTES + 1)), None)
                .unwrap_err()
                .to_string()
                .starts_with("local_arguments_too_large")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn configured_room_repository_root_overrides_default() {
        let mut config = Config::default_config().unwrap();
        let root = unique_temp_dir("configured-room-repository");
        config.room.repository_root = Some(root.clone());
        assert_eq!(room_repository::repository_root(&config), root);
    }

    #[test]
    fn room_timezone_defaults_and_can_be_overridden() {
        let mut config = Config::default_config().unwrap();
        assert_eq!(config.room.timezone, "Asia/Shanghai");
        assert_eq!(config.room.diary_day_boundary_hour, 5);
        config.room.timezone = "UTC".to_string();
        config.room.diary_day_boundary_hour = 3;
        assert_eq!(config.room.timezone, "UTC");
        assert_eq!(config.room.diary_day_boundary_hour, 3);
    }

    fn command_test_state(
        profile: CapabilityProfile,
        workspace_root: PathBuf,
    ) -> (AppState, mpsc::UnboundedReceiver<AgentMessage>) {
        let mut config = Config::default_config().unwrap();
        config.toolsets = if profile == CapabilityProfile::Room {
            config::ToolsetConfig::room()
        } else {
            config::ToolsetConfig::normal()
        };
        config.workspace_root = workspace_root;
        let (tx, rx) = mpsc::unbounded_channel();
        let private_state =
            crate::private_state::PrivateStatePaths::for_test(std::env::temp_dir().join(format!(
                "agentic-test-private-{}",
                uuid::Uuid::new_v4().simple()
            )));
        let job_history = crate::job_history::JobHistoryStore::open(&private_state);
        (
            AppState {
                config_path: PathBuf::from("test-config.json"),
                config: Arc::new(RwLock::new(config)),
                private_state,
                job_history,
                browser_runtime: None,
                runtime: RuntimeModel::hub(profile),
                started_at: chrono::Utc::now(),
                boot_generation: "testboot0001".to_string(),
                supervised: false,
                file_locks: Arc::new(Mutex::new(HashMap::new())),
                jobs: Arc::new(Mutex::new(HashMap::new())),
                hub_sender: Arc::new(Mutex::new(Some(tx))),
                reporting_sender: Arc::new(Mutex::new(None)),
                pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
                temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
                mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
                room_repository_writes: Arc::new(Mutex::new(())),
                skills_writes: Arc::new(Mutex::new(())),
                skill_leases: Arc::new(jobs::SkillLeaseManager::new()),
                skill_installs: Arc::new(skill_installs::InstallManager::new()),
            },
            rx,
        )
    }

    async fn recv_response(rx: &mut mpsc::UnboundedReceiver<AgentMessage>) -> serde_json::Value {
        let message = rx.recv().await.unwrap();
        let AgentMessage::Response { data, .. } = message else {
            panic!("expected agent response");
        };
        data
    }

    #[tokio::test]
    async fn normal_runtime_follows_live_room_toolset_for_bootstrap_dispatch() {
        let workspace = unique_temp_dir("normal-live-room-bootstrap").join("workspace");
        let guides = workspace.join("bootstrap").join("guides");
        fs::create_dir_all(&guides).unwrap();
        fs::write(
            workspace.join("bootstrap").join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room\ndescription: Route guides\nschemaVersion: 1\n---\nstart\n",
        )
        .unwrap();
        fs::write(
            guides.join("guide.md"),
            "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n",
        )
        .unwrap();
        let (state, _rx) = command_test_state(CapabilityProfile::Normal, workspace);

        let disabled = local_service::dispatch(
            state.clone(),
            HubCommand::RoomBootstrap {
                request_id: "req-disabled".to_string(),
            },
            operation::RequestContext::new(operation::RequestIngress::Hub, "room.bootstrap"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(disabled["error"]["code"], "room_toolset_required");

        let mut config = state.config.read().await.clone();
        config.toolsets.enable(config::ToolNamespace::Room);
        *state.config.write().await = config;

        let enabled = local_service::dispatch(
            state,
            HubCommand::RoomBootstrap {
                request_id: "req-enabled".to_string(),
            },
            operation::RequestContext::new(operation::RequestIngress::Hub, "room.bootstrap"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(enabled["entrypoint"]["id"], "room");
        assert_eq!(enabled["guides"][0]["id"], "guide");
    }

    #[tokio::test]
    async fn hub_adapter_and_local_dispatcher_share_toolset_errors() {
        let workspace = unique_temp_dir("dispatcher-parity").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
        let command = HubCommand::RoomBootstrap {
            request_id: "req-parity".to_string(),
        };

        let direct = local_service::dispatch(
            state.clone(),
            command.clone(),
            operation::RequestContext::new(
                operation::RequestIngress::Hub,
                operation::hub_command_name(&command),
            ),
            None,
        )
        .await
        .unwrap();
        hub::handle_hub_command(state, command, None).await.unwrap();
        let adapted = recv_response(&mut rx).await;
        assert_eq!(direct, adapted);
        assert_eq!(direct["error"]["code"], "room_toolset_required");
    }

    #[tokio::test]
    async fn room_mode_dispatches_bootstrap_manifest_and_read() {
        let workspace = unique_temp_dir("room-bootstrap-dispatch").join("workspace");
        let guides = workspace.join("bootstrap").join("guides");
        fs::create_dir_all(&guides).unwrap();
        fs::write(
            workspace.join("bootstrap").join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room\ndescription: Route guides\nschemaVersion: 1\n---\nstart\n",
        )
        .unwrap();
        fs::write(
            guides.join("guide.md"),
            "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n",
        )
        .unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Room, workspace);

        hub::handle_hub_command(
            state.clone(),
            HubCommand::RoomBootstrap {
                request_id: "req-bootstrap".to_string(),
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["schemaVersion"], 1);
        assert_eq!(response["entrypoint"]["id"], "room");
        assert_eq!(response["guides"][0]["id"], "guide");

        hub::handle_hub_command(
            state,
            HubCommand::RoomBootstrapRead {
                request_id: "req-bootstrap-read".to_string(),
                payload: BootstrapReadRequest {
                    id: "guide".to_string(),
                },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["guide"]["id"], "guide");
        assert_eq!(
            response["resource"]["content"],
            "---\nid: guide\nkind: guide\ntitle: Guide\nsummary: Use guide\n---\nbody\n"
        );
    }

    #[tokio::test]
    async fn room_mode_rejects_legacy_jsonl_commands() {
        let workspace = unique_temp_dir("room-legacy-rejected").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Room, workspace);

        hub::handle_hub_command(
            state,
            HubCommand::RoomNotebookAppend {
                request_id: "req-legacy".to_string(),
                payload: NotebookAppendRequest {
                    datetime: None,
                    scope: "agentic".to_string(),
                    significance: PassageSignificance::Anchor,
                    abstract_text: None,
                    content: "legacy".to_string(),
                    tags: Vec::new(),
                },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_legacy_surface_removed");
    }

    #[test]
    fn room_policy_overlay_differs_from_normal_policy() {
        let config = Config::default_config().unwrap();
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "rm",
                &[],
                false
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            policy::policy_decision_for_profile(&config, CapabilityProfile::Room, "rm", &[], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn room_policy_keeps_high_risk_commands_restricted() {
        let config = Config::default_config().unwrap();
        for program in ["sudo", "scp", "mount", "systemctl", "service"] {
            assert_eq!(
                policy::policy_decision_for_profile(
                    &config,
                    CapabilityProfile::Room,
                    program,
                    &[],
                    false
                ),
                PolicyDecision::Confirm
            );
        }
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Room,
                "ssh",
                &[],
                false
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn rule_matches_program_and_args_prefix_structurally() {
        let rule = Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        };
        assert!(rule.matches("python", &["-c".to_string(), "print(1)".to_string()]));
        assert!(!rule.matches("python3", &["-c".to_string()]));
        assert!(!rule.matches("python", &["script.py".to_string()]));
    }

    #[test]
    fn safe_summary_includes_path_roots_and_policy_rules() {
        let root = unique_temp_dir("safe-summary");
        let workspace = root.join("workspace");
        let write_root = root.join("write");
        let read_only_root = root.join("readonly");
        let deny_root = root.join("deny");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&write_root).unwrap();
        fs::create_dir_all(&read_only_root).unwrap();
        fs::create_dir_all(&deny_root).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![write_root.clone()],
            read_only_roots: vec![read_only_root.clone()],
            deny_roots: vec![deny_root.clone()],
        };
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["status".to_string()],
        });
        config.policy.confirm.push(Rule {
            program: "bash".to_string(),
            args_prefix: vec!["-lc".to_string()],
        });
        config.policy.deny.push(Rule {
            program: "rm".to_string(),
            args_prefix: vec!["-rf".to_string()],
        });

        let summary = config.safe_summary();
        assert_eq!(summary.path_policy.write_root_count, 2);
        assert_eq!(summary.path_policy.read_only_root_count, 1);
        assert_eq!(summary.path_policy.deny_root_count, 1);
        assert!(summary
            .path_policy
            .write_roots
            .iter()
            .any(|root| root.path == "workspace" && root.source == "workspaceRoot"));
        assert!(summary
            .path_policy
            .write_roots
            .iter()
            .any(|root| root.path.ends_with("/write") && root.source == "configured"));
        assert!(summary
            .path_policy
            .read_only_roots
            .iter()
            .any(|root| root.path.ends_with("/readonly") && root.source == "configured"));
        assert!(summary
            .path_policy
            .deny_roots
            .iter()
            .any(|root| root.path.ends_with("/deny") && root.source == "configured"));

        assert_eq!(summary.policy_rule_counts.allow, 1);
        assert_eq!(summary.policy_rule_counts.confirm, 1);
        assert_eq!(summary.policy_rule_counts.deny, 1);
        assert!(summary.policy_rules.allow.iter().any(|rule| {
            rule.program == "git" && rule.args_prefix == vec!["status".to_string()]
        }));
        assert!(summary
            .policy_rules
            .confirm
            .iter()
            .any(|rule| { rule.program == "bash" && rule.args_prefix == vec!["-lc".to_string()] }));
        assert!(summary
            .policy_rules
            .deny
            .iter()
            .any(|rule| { rule.program == "rm" && rule.args_prefix == vec!["-rf".to_string()] }));
        assert!(summary
            .policy_rules
            .builtins
            .confirm
            .iter()
            .any(|rule| rule.program == "bash" && rule.args_prefix.is_empty()));
        assert!(summary
            .policy_rules
            .builtins
            .confirm
            .iter()
            .any(|rule| rule.program == "python" && rule.args_prefix == vec!["-c".to_string()]));
        assert!(summary
            .policy_rules
            .builtins
            .deny
            .iter()
            .any(|rule| rule.program == "ssh" && rule.args_prefix.is_empty()));
    }

    #[test]
    fn configured_allow_overrides_need_confirm() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["status".to_string()],
        });
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "git",
                &["status".to_string()],
                true
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_allow_overrides_builtin_confirm() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "curl".to_string(),
            args_prefix: vec!["--version".to_string()],
        });
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "curl",
                &["--version".to_string()],
                false
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_allow_overrides_builtin_deny() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "ssh".to_string(),
            args_prefix: vec!["-V".to_string()],
        });
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "ssh",
                &["-V".to_string()],
                false
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_deny_wins_when_multiple_config_rules_match() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec![],
        });
        config.policy.deny.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["push".to_string()],
        });
        assert_eq!(
            policy::policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "git",
                &["push".to_string()],
                false
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn sudo_requires_credentials() {
        let config = Config::default_config().unwrap();
        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "sudo",
                &["true".to_string()]
            )
            .unwrap_err(),
            "interactive_credential_required"
        );
    }

    #[test]
    fn read_only_system_file_is_allowed() {
        let config = Config::default_config().unwrap();
        assert!(exec::preflight(
            &config,
            &config.workspace_root,
            "cat",
            &["/proc/meminfo".to_string()]
        )
        .is_ok());
        assert!(exec::preflight(&config, &config.workspace_root, "df", &["/".to_string()]).is_ok());
    }

    #[test]
    fn path_policy_allows_write_root_and_blocks_readonly_write() {
        let root = unique_temp_dir("path-policy");
        let workspace = root.join("workspace");
        let downloads = root.join("Downloads");
        let cache = root.join(".cache");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&downloads).unwrap();
        fs::create_dir_all(&cache).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![downloads.clone()],
            read_only_roots: vec![cache.clone()],
            deny_roots: Vec::new(),
        };

        assert!(exec::preflight(
            &config,
            &config.workspace_root,
            "touch",
            &[downloads.join("test-file").to_string_lossy().to_string()]
        )
        .is_ok());
        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "touch",
                &[cache.join("test-file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
        assert!(exec::preflight(
            &config,
            &config.workspace_root,
            "du",
            &[cache.to_string_lossy().to_string()]
        )
        .is_ok());
    }

    #[test]
    fn deny_roots_override_read_and_write() {
        let root = unique_temp_dir("path-deny");
        let workspace = root.join("workspace");
        let downloads = root.join("Downloads");
        let secret = downloads.join("secret");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&secret).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![downloads.clone()],
            read_only_roots: Vec::new(),
            deny_roots: vec![secret.clone()],
        };

        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "cat",
                &[secret.join("token").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "rm",
                &[secret.join("token").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
    }

    #[test]
    fn unknown_program_defaults_to_write_access() {
        let root = unique_temp_dir("path-unknown");
        let workspace = root.join("workspace");
        let cache = root.join(".cache");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&cache).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: vec![cache.clone()],
            deny_roots: Vec::new(),
        };

        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "custom-tool",
                &[cache.join("file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
    }

    #[test]
    fn batch_confirmation_preview_supports_chinese() {
        let mut config = Config::default_config().unwrap();
        config.confirmation_language = "zh-CN".to_string();
        let element = PreparedBatchElement {
            index: 1,
            program: "python".to_string(),
            args: vec!["-c".to_string(), "print(1)".to_string()],
            working_directory: Some("/tmp".to_string()),
            resolved_working_directory: PathBuf::from("/tmp"),
            decision: PolicyDecision::Confirm,
        };
        let preview = confirmation::batch_confirmation_preview(
            &config,
            std::slice::from_ref(&element),
            std::slice::from_ref(&element),
        );

        assert!(preview.contains("该批次共有 1 条命令，其中 1 条需要确认"));
        assert!(preview.contains("工作目录：/tmp"));
        assert!(preview.contains("是否允许整个批次执行一次？"));
        assert!(!preview.contains("\\n"));
    }

    #[test]
    fn working_directory_must_be_existing_writable_directory() {
        let root = unique_temp_dir("working-dir-policy");
        let workspace = root.join("workspace");
        let subdir = workspace.join("subdir");
        let cache = root.join("cache");
        let secret = workspace.join("secret");
        fs::create_dir_all(&subdir).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&secret).unwrap();
        fs::write(workspace.join("file"), "not a directory").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: vec![cache],
            deny_roots: vec![secret.clone()],
        };

        assert_eq!(
            exec::resolve_working_directory(&config, None).unwrap(),
            workspace.canonicalize().unwrap()
        );
        assert_eq!(
            exec::resolve_working_directory(&config, Some("subdir")).unwrap(),
            subdir.canonicalize().unwrap()
        );
        assert_eq!(
            exec::resolve_working_directory(&config, Some("file")).unwrap_err(),
            "working_directory_not_directory"
        );
        assert_eq!(
            exec::resolve_working_directory(&config, Some("missing")).unwrap_err(),
            "working_directory_not_found"
        );
        assert_eq!(
            exec::resolve_working_directory(&config, Some("secret")).unwrap_err(),
            "working_directory_denied"
        );
        assert_eq!(
            exec::resolve_working_directory(
                &config,
                Some(root.join("cache").to_string_lossy().as_ref())
            )
            .unwrap_err(),
            "working_directory_outside_allowed_roots"
        );
    }

    #[test]
    fn relative_path_arguments_are_resolved_from_working_directory() {
        let root = unique_temp_dir("working-dir-relative");
        let workspace = root.join("workspace");
        let subdir = workspace.join("subdir");
        fs::create_dir_all(&subdir).unwrap();
        fs::write(subdir.join("target.txt"), "ok").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        };

        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "cat",
                &["./target.txt".to_string()]
            )
            .unwrap_err(),
            "path_not_found"
        );
        assert!(exec::preflight(&config, &subdir, "cat", &["./target.txt".to_string()]).is_ok());
    }

    #[test]
    fn symlink_to_denied_path_is_rejected() {
        let root = unique_temp_dir("path-symlink");
        let workspace = root.join("workspace");
        let secret = root.join("secret");
        let link = workspace.join("secret-link");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&secret).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: vec![secret],
        };

        #[cfg(unix)]
        assert_eq!(
            exec::preflight(
                &config,
                &config.workspace_root,
                "cat",
                &[link.to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
    }

    #[test]
    fn load_old_config_without_path_policy_adds_defaults() {
        let root = unique_temp_dir("old-config");
        let config_path = root.join("config.json");
        let config = Config::default_config().unwrap();
        let mut value = serde_json::to_value(config).unwrap();
        value.as_object_mut().unwrap().remove("pathPolicy");
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert!(!loaded.path_policy.write_roots.is_empty());
        assert!(!loaded.path_policy.read_only_roots.is_empty());
        assert!(!loaded.path_policy.deny_roots.is_empty());
    }

    #[test]
    fn load_partial_path_policy_uses_workspace_derived_defaults_for_missing_lists() {
        let root = unique_temp_dir("partial-config");
        let config_path = root.join("config.json");
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["pathPolicy"] = serde_json::json!({
            "writeRoots": [root.join("write")]
        });
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.path_policy.write_roots.len(), 1);
        let defaults = crate::config::default_path_policy(&loaded.workspace_root);
        assert_eq!(loaded.path_policy.read_only_roots, defaults.read_only_roots);
        assert_eq!(loaded.path_policy.deny_roots, defaults.deny_roots);
    }

    #[test]
    fn old_rule_ids_are_ignored_when_loading_config() {
        let root = unique_temp_dir("old-rule-id");
        let config_path = root.join("config.json");
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["policy"]["allow"] = serde_json::json!([
            {
                "id": "legacy-id",
                "program": "bash",
                "argsPrefix": []
            }
        ]);
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.policy.allow.len(), 1);
        assert_eq!(loaded.policy.allow[0].program, "bash");
        assert!(serde_json::to_value(&loaded.policy.allow[0])
            .unwrap()
            .get("id")
            .is_none());
    }

    #[test]
    fn remove_rule_matches_command_without_uuid() {
        let mut rules = vec![Rule {
            program: "bash".to_string(),
            args_prefix: Vec::new(),
        }];

        policy::remove_rule(&mut rules, "bash", &[]).unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn remove_rule_matches_command_and_args_prefix() {
        let mut rules = vec![
            Rule {
                program: "python".to_string(),
                args_prefix: vec!["-c".to_string()],
            },
            Rule {
                program: "python".to_string(),
                args_prefix: vec!["script.py".to_string()],
            },
        ];

        policy::remove_rule(&mut rules, "python", &["-c".to_string()]).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].args_prefix, vec!["script.py".to_string()]);
    }

    #[test]
    fn remove_rule_refuses_ambiguous_non_interactive_match() {
        let mut rules = vec![
            Rule {
                program: "bash".to_string(),
                args_prefix: Vec::new(),
            },
            Rule {
                program: "bash".to_string(),
                args_prefix: Vec::new(),
            },
        ];

        let error =
            policy::remove_rule_with_interactive(&mut rules, "bash", &[], false).unwrap_err();
        assert!(error.to_string().contains("multiple_matching_rules"));
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn path_root_remove_matches_expanded_equivalent_path() {
        let root = unique_temp_dir("path-cli");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        let mut policy = PathPolicyConfig::default();

        policy::mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Add {
                path: target.clone(),
            },
        );
        assert_eq!(policy.write_roots.len(), 1);
        policy::mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Remove {
                path: target.join("..").join("target"),
            },
        );
        assert!(policy.write_roots.is_empty());
    }

    #[test]
    fn standalone_reload_replaces_the_frozen_live_subset() {
        let mut live = Config::default_config().unwrap();
        let original_agent_id = live.agent_id.clone();
        let original_workspace = live.workspace_root.clone();
        let original_path_policy = live.path_policy.clone();
        let mut candidate = live.clone();
        candidate.agent_id = "must-not-reload".to_string();
        candidate.workspace_root = PathBuf::from("/tmp/must-not-reload");
        candidate.policy.allow.push(Rule {
            program: "printf".to_string(),
            args_prefix: Vec::new(),
        });
        candidate.path_policy.write_roots = vec![PathBuf::from("/tmp/live")];
        candidate.limits.max_active_jobs = config::MaxActiveJobs::Explicit(9);
        candidate.limits.max_file_search_context_lines = 20;
        live.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "streamable-http".to_string(),
                url: Some("https://old.example/mcp".to_string()),
                auth: None,
            },
        );
        let in_flight = live.mcp_servers["primary"].clone();
        candidate.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "streamable-http".to_string(),
                url: Some("https://new.example/mcp".to_string()),
                auth: None,
            },
        );

        let candidate_toolsets = candidate.toolsets.clone();
        let resolved = apply_live_config_subset(&mut live, candidate);

        assert_eq!(live.agent_id, original_agent_id);
        assert_eq!(live.workspace_root, original_workspace);
        assert!(live
            .policy
            .allow
            .iter()
            .any(|rule| rule.program == "printf"));
        assert_eq!(live.path_policy, original_path_policy);
        assert_eq!(resolved.resolved, 9);
        assert_eq!(
            live.limits.max_active_jobs,
            config::MaxActiveJobs::Explicit(9)
        );
        assert_eq!(live.limits.max_file_search_context_lines, 20);
        assert_eq!(live.toolsets, candidate_toolsets);
        assert_eq!(
            live.mcp_servers["primary"].url.as_deref(),
            Some("https://new.example/mcp")
        );
        assert!(!live.mcp_servers["primary"].enabled);
        assert_eq!(
            in_flight.url.as_deref(),
            Some("https://old.example/mcp"),
            "an already-cloned in-flight definition retains the old endpoint"
        );
    }

    #[tokio::test]
    async fn wp2_reload_preserves_live_path_policy_when_workspace_changes() -> anyhow::Result<()> {
        let root = unique_temp_dir("standalone-live-path-policy-boundary");
        fs::create_dir_all(&root)?;
        let config_path = root.join("config.json");
        let live_workspace = root.join("workspace-live");
        let candidate_workspace = root.join("workspace-candidate");
        let live_write_root = live_workspace.join("write");
        let live_read_only_root = live_workspace.join("read-only");
        let live_deny_root = live_workspace.join("deny");
        let candidate_write_root = candidate_workspace.join("write");
        let candidate_read_only_root = candidate_workspace.join("read-only");
        let candidate_deny_root = candidate_workspace.join("deny");
        for path in [
            &live_write_root,
            &live_read_only_root,
            &live_deny_root,
            &candidate_write_root,
            &candidate_read_only_root,
            &candidate_deny_root,
        ] {
            fs::create_dir_all(path)?;
        }

        let (mut state, _rx) =
            command_test_state(CapabilityProfile::Normal, live_workspace.clone());
        state.config_path = config_path.clone();
        state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);

        let mut initial = state.config.read().await.clone();
        initial.workspace_root = live_workspace.clone();
        initial.tunnel = Some(TunnelConfig {
            tunnel_id: "tunnel_test".to_string(),
            api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
            ..TunnelConfig::default()
        });
        initial.path_policy = PathPolicyConfig {
            write_roots: vec![live_write_root],
            read_only_roots: vec![live_read_only_root],
            deny_roots: vec![live_deny_root],
        };
        let live_path_policy = initial.path_policy.clone();
        *state.config.write().await = initial.clone();

        let mut candidate = initial;
        candidate.workspace_root = candidate_workspace;
        candidate.path_policy = PathPolicyConfig {
            write_roots: vec![candidate_write_root],
            read_only_roots: vec![candidate_read_only_root],
            deny_roots: vec![candidate_deny_root],
        };
        candidate.policy.allow.push(Rule {
            program: "printf".to_string(),
            args_prefix: Vec::new(),
        });
        fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

        reload_live_config_once(&state).await?;

        let live = state.config.read().await.clone();
        assert_eq!(live.workspace_root, live_workspace);
        assert_eq!(live.path_policy, live_path_policy);
        assert!(live
            .policy
            .allow
            .iter()
            .any(|rule| rule.program == "printf"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }
    #[tokio::test]
    async fn wp2_hub_live_reload_preserves_restart_fields() -> anyhow::Result<()> {
        let root = unique_temp_dir("hub-live-config-boundary");
        fs::create_dir_all(&root)?;
        let config_path = root.join("config.json");
        let live_workspace = root.join("workspace-live");
        let candidate_workspace = root.join("workspace-candidate");
        let live_write_root = live_workspace.join("write");
        let candidate_write_root = candidate_workspace.join("write");
        fs::create_dir_all(&live_write_root)?;
        fs::create_dir_all(&candidate_write_root)?;

        let (mut state, _rx) =
            command_test_state(CapabilityProfile::Normal, live_workspace.clone());
        state.config_path = config_path.clone();
        state.runtime = RuntimeModel::hub(CapabilityProfile::Normal);

        let mut initial = state.config.read().await.clone();
        initial.mode = RuntimeMode::Hub;
        initial.workspace_root = live_workspace.clone();
        initial.hub.agent_secret = "hub-secret".to_string();
        initial.path_policy.write_roots = vec![live_write_root];
        let live_hub_url = initial.hub.url.clone();
        let live_path_policy = initial.path_policy.clone();
        *state.config.write().await = initial.clone();

        let mut candidate = initial;
        candidate.hub.url = "http://127.0.0.1:18787".to_string();
        candidate.workspace_root = candidate_workspace;
        candidate.path_policy.write_roots = vec![candidate_write_root];
        candidate.policy.allow.push(Rule {
            program: "printf".to_string(),
            args_prefix: Vec::new(),
        });
        fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

        reload_live_config_once(&state).await?;

        let live = state.config.read().await.clone();
        assert_eq!(live.mode, RuntimeMode::Hub);
        assert_eq!(live.hub.url, live_hub_url);
        assert_eq!(live.workspace_root, live_workspace);
        assert_eq!(live.path_policy, live_path_policy);
        assert!(live
            .policy
            .allow
            .iter()
            .any(|rule| rule.program == "printf"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn standalone_live_reload_applies_valid_mcp_map_and_rejects_invalid_candidate() {
        let root = unique_temp_dir("standalone-live-mcp-reload");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, workspace.clone());
        state.config_path = config_path.clone();
        state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);

        let mut initial = state.config.read().await.clone();
        initial.workspace_root = workspace;
        initial.tunnel = Some(TunnelConfig {
            tunnel_id: "tunnel_test".to_string(),
            api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
            ..TunnelConfig::default()
        });
        initial.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "streamable-http".to_string(),
                url: Some("https://old.example/mcp".to_string()),
                auth: None,
            },
        );
        *state.config.write().await = initial.clone();

        let mut valid = initial.clone();
        valid.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "streamable-http".to_string(),
                url: Some("https://new.example/mcp".to_string()),
                auth: None,
            },
        );
        valid.mcp_servers.insert(
            "local".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "stdio".to_string(),
                url: Some("node ./local-server.mjs".to_string()),
                auth: None,
            },
        );
        valid.limits.max_file_search_context_lines = 20;
        fs::write(&config_path, serde_json::to_vec_pretty(&valid).unwrap()).unwrap();
        reload_live_config_once(&state).await.unwrap();
        let live_after_valid = state.config.read().await.clone();
        assert_eq!(live_after_valid.mcp_servers, valid.mcp_servers);
        assert_eq!(live_after_valid.limits.max_file_search_context_lines, 20);
        assert_eq!(live_after_valid.toolsets, valid.toolsets);

        let mut invalid = valid;
        invalid.mcp_servers.get_mut("primary").unwrap().transport = "sse".to_string();
        fs::write(&config_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let error = reload_live_config_once(&state)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("unsupported_mcp_transport"));
        assert_eq!(
            state.config.read().await.mcp_servers,
            live_after_valid.mcp_servers,
            "invalid disk changes must not partially replace the live map"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn standalone_live_reload_bootstraps_room_repository_before_maintenance_submit(
    ) -> anyhow::Result<()> {
        let root = unique_temp_dir("standalone-live-room-reload");
        let config_path = root.join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace)?;
        let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, workspace.clone());
        state.config_path.clone_from(&config_path);
        state.runtime = RuntimeModel::tunnel(CapabilityProfile::Normal, false);
        let mut initial = state.config.read().await.clone();
        initial.workspace_root = workspace.clone();
        initial.tunnel = Some(TunnelConfig {
            tunnel_id: "tunnel_test".to_string(),
            api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
            ..TunnelConfig::default()
        });
        let live_room_root = workspace.join("room-live");
        let candidate_room_root = workspace.join("room-candidate");
        initial.room.repository_root = Some(live_room_root.clone());
        assert!(!initial.toolsets.is_enabled(config::ToolNamespace::Room));
        assert!(!room_repository::repository_root(&initial).exists());
        *state.config.write().await = initial.clone();

        let mut candidate = initial.clone();
        candidate.room.repository_root = Some(candidate_room_root.clone());
        candidate.toolsets.enable(config::ToolNamespace::Room);
        fs::write(&config_path, serde_json::to_vec_pretty(&candidate)?)?;

        reload_live_config_once(&state).await?;

        let live = state.config.read().await.clone();
        assert_eq!(live.room, initial.room);
        assert!(!candidate_room_root.exists());
        let room_root = room_repository::repository_root(&live);
        assert_eq!(room_root, live_room_root);
        assert!(room_root.join(".git").is_dir());
        for relative in room_repository::scaffold_paths() {
            assert!(
                room_root.join(relative).is_file(),
                "missing Room scaffold file: {relative}"
            );
        }

        let response = room_maintenance::submit(
            &state,
            agentic_gpt_protocol::RoomMaintenanceSubmitRequest {
                items: vec![agentic_gpt_protocol::RoomMaintenanceRequestItem {
                    slot: agentic_gpt_protocol::RoomMaintenanceSlot::Notebook,
                    payload: serde_json::json!({
                        "path": "Notebook/live-reload.md",
                        "title": "Live reload",
                        "body": "Bootstrapped",
                    }),
                }],
                mode: Some(agentic_gpt_protocol::RoomMaintenanceExecutionMode::Local),
                wait_seconds: None,
            },
        )
        .await?;
        assert_eq!(
            response.state,
            agentic_gpt_protocol::RoomMaintenanceSubmissionState::Applied
        );
        assert!(response.local_applied);
        assert_eq!(
            fs::read_to_string(room_root.join("Notebook/live-reload.md"))?,
            "# Live reload\n\nBootstrapped\n"
        );

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn notification_delivery_rejects_unsupported_channel() {
        let response = notify::deliver_freedesktop_notification(
            agentic_gpt_protocol::UserNotifyDeliveryRequest {
                channel_key: "hub::ntfy".to_string(),
                title: "Hello".to_string(),
                body: "World".to_string(),
                actions: Vec::new(),
                priority: None,
            },
        )
        .await;
        assert!(!response.delivered);
        assert_eq!(response.reason.as_deref(), Some("unsupported_channel"));
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
