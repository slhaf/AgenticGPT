mod agent_info;
mod audit;
mod bootstrap;
mod config;
mod confirmation;
mod diary;
mod exec;
mod file_ops;
mod hub;
mod instance_lock;
mod jobs;
mod local_control;
mod local_service;
mod mcp;
mod notebook;
mod notify;
mod policy;
mod skill_installs;
mod skills;
mod state;
mod stdio_server;
mod supervisor;
mod tmux;
mod transport_ledger;
mod tunnel_distribution;
mod utils;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use config::{normalize_confirmation_language, write_config_with_backup, Config, ReportingDetail};
use mcp::McpConfigCommand;
use policy::PolicyDecision;
use serde_json::{Map, Value};
use state::{AppState, CapabilityProfile, RuntimeModel};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration};
use utils::{config_path, ensure_parent, log_info, log_warn};

#[derive(Parser)]
#[command(name = "agentic-gpt")]
#[command(version)]
#[command(about = "Linux local agent for Agentic GPT")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    RunAsRoom {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    RunAsStandalone {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = WorkerProfile::Normal)]
        profile: WorkerProfile,
    },
    RunAsLocal {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = WorkerProfile::Normal)]
        profile: WorkerProfile,
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
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Tmux {
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: TmuxCommand,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum WorkerProfile {
    Normal,
    Room,
}

impl WorkerProfile {
    fn capability_profile(self) -> CapabilityProfile {
        match self {
            Self::Normal => CapabilityProfile::Normal,
            Self::Room => CapabilityProfile::Room,
        }
    }
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

#[derive(Subcommand)]
enum ConfigCommand {
    Init,
    Show,
    Set {
        key: String,
        value: String,
    },
    Allow {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Confirm {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Deny {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Path {
        #[command(subcommand)]
        command: PathCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpConfigCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum RuleCommand {
    Add {
        program: String,
        args_prefix: Vec<String>,
    },
    Remove {
        program: String,
        args_prefix: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum PathCommand {
    List,
    Write {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Readonly {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Deny {
        #[command(subcommand)]
        command: PathRootCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum PathRootCommand {
    Add { path: PathBuf },
    Remove { path: PathBuf },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PathRootKind {
    Write,
    Readonly,
    Deny,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { config } => {
            run(
                config_path(config),
                RuntimeModel::hub(CapabilityProfile::Normal),
            )
            .await
        }
        Commands::RunAsRoom { config } => {
            run(
                config_path(config),
                RuntimeModel::hub(CapabilityProfile::Room),
            )
            .await
        }
        Commands::RunAsStandalone { config, profile } => {
            supervisor::run(config_path(config), profile.capability_profile()).await
        }
        Commands::RunAsLocal { config, profile } => {
            run_as_local(config_path(config), profile.capability_profile()).await
        }
        Commands::Local { config, command } => handle_local(config_path(config), command).await,
        Commands::StdioWorker {
            config,
            profile,
            supervisor_token,
        } => run_stdio_worker(config, profile.capability_profile(), supervisor_token).await,
        Commands::Config { config, command } => handle_config(config_path(config), command).await,
        Commands::Tmux { config, command } => handle_tmux(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf, runtime: RuntimeModel) -> Result<()> {
    log_info(format!(
        "agentic-gpt starting; runtime={}; hubMode={}; config={}",
        runtime.label(),
        runtime.hub_mode.label(),
        config_path.display(),
    ));
    ensure_parent(&config_path)?;
    let _instance_lock = instance_lock::InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    if !config_path.exists() {
        write_config_with_backup(&config_path, &Config::default_config()?)?;
        log_info("default config created".to_string());
    }
    let initial = Config::load(&config_path)?;
    initial.validate_mcp_servers()?;
    initial.ensure_workspace()?;
    if let Err(error) = tmux::ensure_default_session(&initial.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }
    log_info(format!(
        "config loaded; agentId={}; hubUrl={}; workspaceRoot={}; sandbox={}; {}",
        initial.agent_id,
        initial.hub_url,
        initial.workspace_root.display(),
        if initial.sandbox.enabled {
            "enabled"
        } else {
            "disabled"
        },
        initial.limits.max_active_jobs.resolve().diagnostic()
    ));
    let state = build_app_state(config_path.clone(), initial, runtime, false);
    state.skill_installs.recover(state.clone()).await?;
    tokio::spawn(watch_config(state.clone()));
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
    let state = build_app_state(
        config_path,
        config,
        RuntimeModel::tunnel(profile, reporting_enabled),
        supervised,
    );
    state.skill_installs.recover(state.clone()).await?;
    tokio::spawn(watch_standalone_live_config(state.clone(), supervised));
    if reporting_enabled {
        tokio::spawn(hub::connect_loop(state.clone()));
    }
    let listener = local_control::bind(&agent_id).await?;
    log_info(format!(
        "local MCP ingress ready; transport=unix; path={}",
        listener.path().display()
    ));
    let mut local_task = tokio::spawn(listener.serve(state.clone()));
    let stdio = stdio_server::serve_stdio(state);
    tokio::pin!(stdio);
    tokio::select! {
        result = &mut stdio => {
            local_task.abort();
            let _ = local_task.await;
            result
        }
        result = &mut local_task => {
            match result {
                Ok(result) => result,
                Err(_) => Err(anyhow!("local_mcp_listener_task_failed")),
            }
        }
    }
}

const MAX_LOCAL_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;

fn build_app_state(
    config_path: PathBuf,
    config: Config,
    runtime: RuntimeModel,
    supervised: bool,
) -> AppState {
    let max_concurrent_skill_installs = config.skills.max_concurrent_installs;
    AppState {
        config_path,
        config: Arc::new(RwLock::new(config)),
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
        notebook_writes: Arc::new(Mutex::new(())),
        skills_writes: Arc::new(Mutex::new(())),
        skill_leases: Arc::new(jobs::SkillLeaseManager::new()),
        skill_installs: Arc::new(skill_installs::InstallManager::with_concurrency(
            max_concurrent_skill_installs,
        )),
    }
}

async fn run_as_local(config_path: PathBuf, profile: CapabilityProfile) -> Result<()> {
    log_info(format!(
        "local agent starting; profile={}; config={}",
        profile.label(),
        config_path.display()
    ));
    ensure_parent(&config_path)?;
    let _instance_lock = instance_lock::InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    if !config_path.exists() {
        write_config_with_backup(&config_path, &Config::default_config()?)?;
        log_info("default config created".to_string());
    }
    let config = Config::load(&config_path)?;
    config.validate_local()?;
    config.ensure_workspace()?;
    if let Err(error) = tmux::ensure_default_session(&config.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }
    let agent_id = config.agent_id.clone();
    let state = build_app_state(config_path, config, RuntimeModel::local(profile), false);
    state.skill_installs.recover(state.clone()).await?;
    tokio::spawn(watch_standalone_live_config(state.clone(), false));
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

async fn handle_tmux(config_path: PathBuf, command: TmuxCommand) -> Result<()> {
    match command {
        TmuxCommand::List => println!(
            "{}",
            serde_json::to_string_pretty(&tmux::list_sessions().await)?
        ),
        TmuxCommand::Attach { session } => tmux::attach(&session)
            .await
            .map_err(|error| anyhow!(error))?,
        TmuxCommand::Create { name, cwd } => {
            let config = Config::load_or_default(&config_path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &tmux::create_session_for_config(&config, &name, &cwd).await
                )?
            );
        }
        TmuxCommand::Close { name } => println!(
            "{}",
            serde_json::to_string_pretty(&tmux::close_session_local(&name).await)?
        ),
    }
    Ok(())
}

async fn handle_config(config_path: PathBuf, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Init => {
            let config = Config::default_config()?;
            write_config_with_backup(&config_path, &config)?;
            println!("initialized {}", config_path.display());
        }
        ConfigCommand::Show => {
            let config = Config::load(&config_path)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load_or_default(&config_path)?;
            match key.as_str() {
                "workspaceRoot" => {
                    let old_workspace = config.workspace_root.clone();
                    let new_workspace = PathBuf::from(value);
                    for root in &mut config.path_policy.write_roots {
                        if policy::paths_match(root, &old_workspace) {
                            *root = new_workspace.clone();
                        }
                    }
                    config.workspace_root = new_workspace;
                }
                "confirmationProvider" => config
                    .confirmation_provider
                    .set_legacy(&value)
                    .map_err(|error| anyhow!(error))?,
                "confirmationLanguage" => {
                    config.confirmation_language = normalize_confirmation_language(&value)
                }
                "sandbox.enabled" => config.sandbox.enabled = value.parse::<bool>()?,
                "room.notebookRoot" => config.room.notebook_root = Some(PathBuf::from(value)),
                "room.timezone" => config.room.timezone = value,
                "room.diaryDayBoundaryHour" => {
                    let hour = value.parse::<u32>()?;
                    if hour > 23 {
                        return Err(anyhow!(
                            "room.diaryDayBoundaryHour must be an integer from 0 to 23"
                        ));
                    }
                    config.room.diary_day_boundary_hour = hour;
                }
                "skills.maxFiles" => config.skills.max_files = value.parse()?,
                "skills.maxFileBytes" => config.skills.max_file_bytes = value.parse()?,
                "skills.maxPackageBytes" => config.skills.max_package_bytes = value.parse()?,
                "skills.maxSkillMdBytes" => config.skills.max_skill_md_bytes = value.parse()?,
                "skills.maxInlineBytes" => config.skills.max_inline_bytes = value.parse()?,
                "skills.connectTimeoutSecs" => {
                    config.skills.connect_timeout_secs = value.parse()?
                }
                "skills.requestTimeoutSecs" => {
                    config.skills.request_timeout_secs = value.parse()?
                }
                "skills.idleTimeoutSecs" => config.skills.idle_timeout_secs = value.parse()?,
                "skills.maxRedirects" => config.skills.max_redirects = value.parse()?,
                "skills.maxConcurrentInstalls" => {
                    config.skills.max_concurrent_installs = value.parse()?
                }
                "skills.maxParallelDownloads" => {
                    config.skills.max_parallel_downloads = value.parse()?
                }
                "skills.maxAttempts" => config.skills.max_attempts = value.parse()?,
                "skills.totalDeadlineSecs" => config.skills.total_deadline_secs = value.parse()?,
                "skills.allowedHosts" => {
                    config.skills.allowed_hosts = serde_json::from_str(&value)?
                }
                "tunnel.tunnelId" => tunnel_config(&mut config).tunnel_id = value,
                "tunnel.apiKey" => tunnel_config(&mut config).api_key = value,
                "tunnel.client.version" => {
                    tunnel_config(&mut config).client.version =
                        if value == "null" { None } else { Some(value) }
                }
                "tunnel.client.cacheDir" => {
                    tunnel_config(&mut config).client.cache_dir = PathBuf::from(value)
                }
                "tunnel.client.autoDownload" => {
                    tunnel_config(&mut config).client.auto_download = value.parse()?
                }
                "tunnel.client.executable" => {
                    tunnel_config(&mut config).client.executable = if value == "null" {
                        None
                    } else {
                        Some(PathBuf::from(value))
                    }
                }
                "tunnel.client.downloadUrl" => {
                    tunnel_config(&mut config).client.download_url =
                        if value == "null" { None } else { Some(value) }
                }
                "tunnel.client.sha256" => {
                    tunnel_config(&mut config).client.sha256 =
                        if value == "null" { None } else { Some(value) }
                }
                "tunnel.hubReporting.enabled" => {
                    tunnel_config(&mut config).hub_reporting.enabled = value.parse()?
                }
                "tunnel.hubReporting.detail" => {
                    tunnel_config(&mut config).hub_reporting.detail = match value.as_str() {
                        "metadata" => ReportingDetail::Metadata,
                        "full" => ReportingDetail::Full,
                        _ => {
                            return Err(anyhow!(
                                "tunnel hub reporting detail must be metadata or full"
                            ))
                        }
                    }
                }
                "hubUrl" | "workerUrl" => config.hub_url = value,
                "hubTransport" => {
                    let normalized = value.to_lowercase();
                    if normalized != "websocket" && normalized != "sse" {
                        return Err(anyhow!("hubTransport must be websocket or sse"));
                    }
                    config.hub_transport = normalized;
                }
                "agentId" => config.agent_id = value,
                "agentSecret" => config.agent_secret = value,
                _ => return Err(anyhow!("unsupported config key: {key}")),
            }
            write_config_with_backup(&config_path, &config)?;
        }
        ConfigCommand::Allow { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Allow, command)?
        }
        ConfigCommand::Confirm { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Confirm, command)?
        }
        ConfigCommand::Deny { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Deny, command)?
        }
        ConfigCommand::Path { command } => policy::mutate_path_policy(config_path, command)?,
        ConfigCommand::Mcp { command } => mcp::mutate_servers(config_path, command)?,
    }
    Ok(())
}

fn tunnel_config(config: &mut Config) -> &mut config::TunnelConfig {
    config
        .tunnel
        .get_or_insert_with(config::TunnelConfig::default)
}

async fn watch_config(state: AppState) {
    let mut last_modified = fs::metadata(&state.config_path)
        .and_then(|meta| meta.modified())
        .ok();
    loop {
        sleep(Duration::from_secs(2)).await;
        let modified = fs::metadata(&state.config_path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_some() && modified != last_modified {
            match Config::load(&state.config_path).and_then(|config| {
                config.validate_mcp_servers()?;
                Ok(config)
            }) {
                Ok(config) => {
                    let _ = config.ensure_workspace();
                    log_info(format!(
                        "config reloaded; agentId={}; workspaceRoot={}; sandbox={}; {}; mcpServers={}",
                        config.agent_id,
                        config.workspace_root.display(),
                        if config.sandbox.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        config.limits.max_active_jobs.resolve().diagnostic(),
                        config.mcp_servers.len(),
                    ));
                    *state.config.write().await = config;
                    last_modified = modified;
                }
                Err(_) => {
                    log_warn("config reload failed; keeping previous config".to_string());
                }
            }
        }
    }
}

async fn watch_standalone_live_config(state: AppState, supervised: bool) {
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

        let resolved = match reload_standalone_live_config_once(&state).await {
            Ok(resolved) => resolved,
            Err(error) => {
                if !supervised {
                    log_warn(format!(
                        "standalone live config reload rejected; keeping previous subset; errorCode={}",
                        error_code(&error.to_string())
                    ));
                }
                continue;
            }
        };
        let live = state.config.read().await;
        log_info(format!(
            "standalone live config reloaded; {}; policyAllow={}; policyConfirm={}; policyDeny={}; pathWriteRoots={}; pathReadOnlyRoots={}; pathDenyRoots={}; mcpServers={}",
            resolved.diagnostic(),
            live.policy.allow.len(),
            live.policy.confirm.len(),
            live.policy.deny.len(),
            live.path_policy.write_roots.len(),
            live.path_policy.read_only_roots.len(),
            live.path_policy.deny_roots.len(),
            live.mcp_servers.len(),
        ));
    }
}

async fn reload_standalone_live_config_once(
    state: &AppState,
) -> Result<config::ResolvedMaxActiveJobs> {
    let candidate = Config::load(&state.config_path)?;
    match state.runtime.transport {
        crate::state::Transport::TunnelStdio => candidate.validate_standalone()?,
        crate::state::Transport::LocalUnix => candidate.validate_local()?,
        crate::state::Transport::Hub => candidate.validate_mcp_servers()?,
    }
    let mut live = state.config.write().await;
    Ok(apply_standalone_live_subset(&mut live, candidate))
}

fn apply_standalone_live_subset(
    live: &mut Config,
    candidate: Config,
) -> config::ResolvedMaxActiveJobs {
    let resolved = candidate.limits.max_active_jobs.resolve();
    live.policy = candidate.policy;
    live.path_policy = candidate.path_policy;
    live.limits = candidate.limits;
    live.mcp_servers = candidate.mcp_servers;
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
    use crate::exec::PreparedBatchElement;
    use crate::mcp::McpServerConfig;
    use agentic_gpt_protocol::{
        AgentMessage, BootstrapReadRequest, HubCommand, NotebookAppendRequest,
        NotebookRemoveRequest, NotebookUpdateRequest, PassageSignificance,
    };
    use tokio::sync::mpsc;
    use uuid::Uuid;

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
    fn run_as_room_reuses_same_base_config_identity_and_workspace() {
        let config = Config::default_config().unwrap();
        assert_eq!(config.agent_id, "laptop");
        assert_eq!(
            notebook::notebook_root(&config),
            config.workspace_root.join("notebook")
        );
    }

    #[test]
    fn standalone_cli_defaults_to_normal_profile_and_accepts_room() {
        let cli = Cli::try_parse_from(["agentic-gpt", "run-as-standalone"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::RunAsStandalone {
                profile: WorkerProfile::Normal,
                ..
            }
        ));
        let cli =
            Cli::try_parse_from(["agentic-gpt", "run-as-standalone", "--profile", "room"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::RunAsStandalone {
                profile: WorkerProfile::Room,
                ..
            }
        ));
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
    fn configured_room_notebook_root_overrides_default() {
        let mut config = Config::default_config().unwrap();
        let root = unique_temp_dir("configured-notebook-root");
        config.room.notebook_root = Some(root.clone());
        assert_eq!(notebook::notebook_root(&config), root);
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

    #[test]
    fn normal_mode_room_command_error_is_structured() {
        let value = hub::room_agent_required_error();
        assert_eq!(value["error"]["code"], "room_agent_required");
        assert_eq!(
            value["error"]["message"],
            "room commands require run-as-room"
        );
    }

    fn command_test_state(
        profile: CapabilityProfile,
        workspace_root: PathBuf,
    ) -> (AppState, mpsc::UnboundedReceiver<AgentMessage>) {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        let (tx, rx) = mpsc::unbounded_channel();
        (
            AppState {
                config_path: PathBuf::from("test-config.json"),
                config: Arc::new(RwLock::new(config)),
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
                notebook_writes: Arc::new(Mutex::new(())),
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
    async fn normal_mode_rejects_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("normal-room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
        hub::handle_hub_command(
            state.clone(),
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: "psg_1".to_string(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: None,
                    tags: None,
                },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_agent_required");

        hub::handle_hub_command(
            state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest {
                    id: "psg_1".to_string(),
                },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_agent_required");
    }

    #[tokio::test]
    async fn hub_adapter_and_local_dispatcher_share_capability_errors() {
        let workspace = unique_temp_dir("dispatcher-parity").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
        let command = HubCommand::RoomBootstrap {
            request_id: "req-parity".to_string(),
        };

        let direct = local_service::dispatch(state.clone(), command.clone())
            .await
            .unwrap();
        hub::handle_hub_command(state, command, None).await.unwrap();
        let adapted = recv_response(&mut rx).await;
        assert_eq!(direct, adapted);
        assert_eq!(direct["error"]["code"], "room_agent_required");
    }

    #[tokio::test]
    async fn normal_mode_rejects_bootstrap_commands() {
        let workspace = unique_temp_dir("normal-room-bootstrap").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Normal, workspace);
        hub::handle_hub_command(
            state.clone(),
            HubCommand::RoomBootstrap {
                request_id: "req-bootstrap".to_string(),
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            recv_response(&mut rx).await["error"]["code"],
            "room_agent_required"
        );

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
        assert_eq!(
            recv_response(&mut rx).await["error"]["code"],
            "room_agent_required"
        );
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
    async fn room_mode_executes_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(CapabilityProfile::Room, workspace);
        let appended = notebook::append(
            &state,
            NotebookAppendRequest {
                datetime: None,
                scope: "agentic".to_string(),
                significance: PassageSignificance::Anchor,
                abstract_text: "original".to_string(),
                content: "details".to_string(),
                tags: vec![],
            },
        )
        .await
        .unwrap();
        hub::handle_hub_command(
            state.clone(),
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: appended.id.clone(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: Some("updated details".to_string()),
                    tags: Some(vec!["tag".to_string()]),
                },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["updated"], true);
        assert_eq!(response["id"], appended.id);

        hub::handle_hub_command(
            state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest { id: appended.id },
            },
            None,
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["removed"], true);
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
    fn load_partial_path_policy_defaults_missing_lists_to_empty() {
        let root = unique_temp_dir("partial-config");
        let config_path = root.join("config.json");
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["pathPolicy"] = serde_json::json!({
            "writeRoots": [root.join("write")]
        });
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.path_policy.write_roots.len(), 1);
        assert!(loaded.path_policy.read_only_roots.is_empty());
        assert!(loaded.path_policy.deny_roots.is_empty());
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
            },
        );
        let in_flight = live.mcp_servers["primary"].clone();
        candidate.mcp_servers.insert(
            "primary".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "streamable-http".to_string(),
                url: Some("https://new.example/mcp".to_string()),
            },
        );

        let resolved = apply_standalone_live_subset(&mut live, candidate);

        assert_eq!(live.agent_id, original_agent_id);
        assert_eq!(live.workspace_root, original_workspace);
        assert!(live
            .policy
            .allow
            .iter()
            .any(|rule| rule.program == "printf"));
        assert_eq!(
            live.path_policy.write_roots,
            vec![PathBuf::from("/tmp/live")]
        );
        assert_eq!(resolved.resolved, 9);
        assert_eq!(
            live.limits.max_active_jobs,
            config::MaxActiveJobs::Explicit(9)
        );
        assert_eq!(live.limits.max_file_search_context_lines, 20);
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
    async fn standalone_live_reload_applies_valid_mcp_map_and_rejects_invalid_candidate() {
        let root = unique_temp_dir("standalone-live-mcp-reload");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (mut state, _rx) = command_test_state(CapabilityProfile::Normal, workspace.clone());
        state.config_path = config_path.clone();

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
            },
        );
        valid.mcp_servers.insert(
            "local".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "stdio".to_string(),
                url: Some("node ./local-server.mjs".to_string()),
            },
        );
        valid.limits.max_file_search_context_lines = 20;
        fs::write(&config_path, serde_json::to_vec_pretty(&valid).unwrap()).unwrap();
        reload_standalone_live_config_once(&state).await.unwrap();
        let live_after_valid = state.config.read().await.clone();
        assert_eq!(live_after_valid.mcp_servers, valid.mcp_servers);
        assert_eq!(live_after_valid.limits.max_file_search_context_lines, 20);

        let mut invalid = valid;
        invalid.mcp_servers.get_mut("primary").unwrap().transport = "sse".to_string();
        fs::write(&config_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
        let error = reload_standalone_live_config_once(&state)
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
