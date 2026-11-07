// Startup, runtime selection, and live configuration reload orchestration.
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{watch, Mutex, RwLock};
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, Config, RuntimeMode},
    http_server, hub, instance_lock, local_control, private_state, process, process_history,
    room_repository, skill_installs, skills,
    state::{AppState, BrowserRuntimeContext, CapabilityProfile, RuntimeModel},
    stdio_server, supervisor, tmux,
    utils::{ensure_parent, log_info, log_warn},
};

pub(crate) async fn run(config_path: PathBuf) -> Result<()> {
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
    ensure_tmux_session_if_enabled(&initial).await;
    log_info(format!(
        "config loaded; agentId={}; hub.url={}; workspaceRoot={}; sandbox={}; {}",
        initial.agent_id,
        initial.hub.url,
        initial.workspace_root.display(),
        if initial.policy.enabled && initial.policy.bubblewrap {
            "enabled"
        } else {
            "disabled"
        },
        initial.limits.max_active_processes.resolve().diagnostic()
    ));
    let state = build_app_state(config_path.clone(), initial, runtime, false, None)?;
    state.skill_installs.recover(state.clone()).await?;
    crate::event_notifications::recover_event_notifications(&state).await?;
    tokio::spawn(watch_live_config(state.clone(), false, None));
    let secret = state.secret.clone();
    let email = EmailBackground::start(state.clone());
    let browser = BrowserBackground::start(state.clone());
    let result = tokio::select! {
        result = hub::connect_loop(state) => result,
        result = wait_for_local_shutdown_signal() => result,
    };
    browser.stop().await;
    email.stop().await;
    secret.shutdown().await;
    result
}

pub(crate) async fn run_stdio_worker(
    config_path: PathBuf,
    profile: CapabilityProfile,
    supervisor_token: Option<String>,
    startup_config: Option<PathBuf>,
) -> Result<()> {
    supervisor::authorize_worker(supervisor_token.as_deref())?;
    if startup_config.is_some() {
        supervisor::watch_tunnel_parent()?;
    }
    let supervised = supervisor_token.is_some();
    let mut config = Config::load(&config_path)?;
    if let Some(path) = startup_config {
        let startup = supervisor::load_startup_config(&path)?;
        crate::config_runtime::preserve_startup(&mut config, &startup);
    }
    if config.mode != RuntimeMode::Standalone || config.profile.capability_profile() != profile {
        return Err(anyhow!("stdio_worker_config_mismatch"));
    }
    config.validate_standalone()?;
    config.ensure_workspace()?;
    log_info(format!(
        "standalone worker config loaded; {}; policyAllow={}; policyConfirm={}; policyDeny={}; pathWriteAllow={}; pathReadAllow={}; pathDeny={}",
        config.limits.max_active_processes.resolve().diagnostic(),
        config.policy.command.allow.len(),
        config.policy.command.confirm.len(),
        config.policy.command.deny.len(),
        config.policy.path.write.len(),
        config.policy.path.read_only.len(),
        config.policy.path.deny.len(),
    ));
    let hub_enabled = config.auxiliary_hub_enabled();
    let agent_id = config.agent_id.clone();
    let state = build_app_state(
        config_path,
        config,
        RuntimeModel::tunnel(profile, hub_enabled),
        supervised,
        None,
    )?;
    state.skill_installs.recover(state.clone()).await?;
    crate::event_notifications::recover_event_notifications(&state).await?;
    let listener = local_control::bind(&agent_id).await?;
    log_info(format!(
        "local MCP ingress ready; transport=unix; path={}",
        listener.path().display()
    ));
    let initial_http_mcp = state.config.read().await.http_mcp.clone();
    let (http_updates, http_config) = watch::channel(initial_http_mcp);
    *state.config_runtime.http_updates.write().await = Some(http_updates.clone());
    tokio::spawn(watch_live_config(
        state.clone(),
        supervised,
        Some(http_updates),
    ));
    tokio::spawn(hub::connect_loop(state.clone()));
    let mut local_task = tokio::spawn(listener.serve(state.clone()));
    let http_shutdown = CancellationToken::new();
    let mut http_task = tokio::spawn(http_server::run(
        state.clone(),
        http_config,
        http_shutdown.clone(),
    ));
    let secret = state.secret.clone();
    let email = EmailBackground::start(state.clone());
    let browser = BrowserBackground::start(state.clone());
    let stdio = stdio_server::serve_stdio(state);
    tokio::pin!(stdio);
    let result = tokio::select! {
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
    };
    browser.stop().await;
    email.stop().await;
    secret.shutdown().await;
    result
}

struct EmailBackground {
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<Result<()>>,
}

struct BrowserBackground {
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl BrowserBackground {
    fn start(state: AppState) -> Self {
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(crate::browser_discovery::run_background(
            state,
            shutdown.clone(),
        ));
        Self { shutdown, task }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        if let Err(error) = self.task.await {
            log_warn(format!("browser preparation task failed: {error}"));
        }
    }
}

impl EmailBackground {
    fn start(state: AppState) -> Self {
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(crate::email::run(state, shutdown.clone()));
        Self { shutdown, task }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        match self.task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log_warn(format!("email service failed: {error}")),
            Err(error) => log_warn(format!("email service shutdown failed: {error}")),
        }
    }
}
pub(crate) fn build_app_state(
    config_path: PathBuf,
    config: Config,
    runtime: RuntimeModel,
    supervised: bool,
    browser_runtime: Option<Arc<BrowserRuntimeContext>>,
) -> Result<AppState> {
    build_app_state_with_private_state(
        config_path,
        config,
        runtime,
        supervised,
        browser_runtime,
        private_state::prepare()?,
    )
}

pub(crate) fn build_app_state_with_private_state(
    config_path: PathBuf,
    config: Config,
    runtime: RuntimeModel,
    supervised: bool,
    browser_runtime: Option<Arc<BrowserRuntimeContext>>,
    private_state: private_state::PrivateStatePaths,
) -> Result<AppState> {
    let room_error = if config.toolsets.uses_room_repository() {
        room_repository::ensure_repository(&config)
            .err()
            .map(|error| {
                let code = error_code(&error.to_string());
                log_warn(format!("room repository unavailable; errorCode={code}"));
                code
            })
    } else {
        None
    };
    let max_concurrent_skill_installs = config.skills.max_concurrent_installs;
    let event_store = crate::event_store::EventStore::open(&private_state)?;
    let email = crate::email::EmailService::open(&private_state)?;
    let secret = crate::secret::SecretService::open(&private_state)?;
    let process_history = process_history::ProcessHistoryStore::open(&private_state);
    if let Err(error) = process_history.recover_active(chrono::Utc::now()) {
        log_warn(format!("process history recovery failed: {error}"));
    }
    let skill_installs_root = private_state.skill_installs.clone();
    let config_runtime = Arc::new(crate::config_runtime::ConfigRuntime::new(
        &config,
        browser_runtime.is_some(),
        room_error,
    ));
    let browser_runtime_cell = Arc::new(crate::state::BrowserRuntimeSlot::new(browser_runtime));
    Ok(AppState {
        config_path,
        transport_agent_id: config.agent_id.clone(),
        config: Arc::new(RwLock::new(config)),
        private_state,
        event_store,
        email,
        secret,
        process_history,
        browser_runtime: browser_runtime_cell,
        config_runtime,
        runtime,
        started_at: chrono::Utc::now(),
        boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        supervised,
        file_locks: Arc::new(Mutex::new(HashMap::new())),
        processes: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        reporting_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        mcp_concurrency: Arc::new(process::McpConcurrency::new()),
        room_repository_writes: Arc::new(Mutex::new(())),
        skills_writes: Arc::new(Mutex::new(())),
        skill_leases: Arc::new(skills::SkillLeaseManager::new()),
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
    ensure_tmux_session_if_enabled(&config).await;
    let agent_id = config.agent_id.clone();
    let hub_enabled = config.auxiliary_hub_enabled();
    let state = build_app_state(
        config_path,
        config,
        RuntimeModel::local_with_hub(profile, hub_enabled),
        false,
        None,
    )?;
    state.skill_installs.recover(state.clone()).await?;
    crate::event_notifications::recover_event_notifications(&state).await?;
    tokio::spawn(watch_live_config(state.clone(), false, None));
    let listener = local_control::bind(&agent_id).await?;
    log_info(format!(
        "local MCP ingress ready; transport=unix; path={}",
        listener.path().display()
    ));
    let hub_task = Some(tokio::spawn(hub::connect_loop(state.clone())));
    let secret = state.secret.clone();
    let email = EmailBackground::start(state.clone());
    let browser = BrowserBackground::start(state.clone());
    let mut local_task = tokio::spawn(listener.serve(state));
    let result = tokio::select! {
        result = &mut local_task => match result {
            Ok(result) => result,
            Err(_) => Err(anyhow!("local_mcp_listener_task_failed")),
        },
        signal = wait_for_local_shutdown_signal() => {
            local_task.abort();
            let _ = local_task.await;
            signal
        }
    };
    if let Some(task) = hub_task {
        task.abort();
        let _ = task.await;
    }
    browser.stop().await;
    email.stop().await;
    secret.shutdown().await;
    result
}

async fn ensure_tmux_session_if_enabled(config: &Config) {
    if !config
        .toolsets
        .is_enabled(crate::config::ToolNamespace::Tmux)
    {
        return;
    }
    if let Err(error) = tmux::ensure_default_session(&config.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
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
async fn watch_live_config(
    state: AppState,
    supervised: bool,
    _http_updates: Option<watch::Sender<config::HttpMcpConfig>>,
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
        log_info(format!(
            "live config reloaded; {}; policyAllow={}; policyConfirm={}; policyDeny={}; pathWriteAllow={}; pathReadAllow={}; pathDeny={}; mcpServers={}; httpMcpEnabled={}",
            resolved.diagnostic(),
            live.policy.command.allow.len(),
            live.policy.command.confirm.len(),
            live.policy.command.deny.len(),
            live.policy.path.write.len(),
            live.policy.path.read_only.len(),
            live.policy.path.deny.len(),
            live.mcp_servers.len(),
            live.http_mcp.enabled,
        ));
    }
}

pub(crate) async fn reload_live_config_once(
    state: &AppState,
) -> Result<config::ResolvedMaxActiveProcesses> {
    let _serial = state.config_runtime.apply_lock.lock().await;
    let candidate = Config::load(&state.config_path)?;
    crate::config_apply::validate(&candidate)?;
    let resolved = candidate.limits.max_active_processes.resolve();
    crate::config_runtime::apply_locked(state, candidate).await?;
    Ok(resolved)
}

pub(crate) fn error_code(value: &str) -> String {
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
