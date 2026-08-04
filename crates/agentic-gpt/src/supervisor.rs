//! Standalone Tunnel supervisor.
//!
//! The supervisor is the only process that owns the Agentic runtime lock. It
//! launches the trusted tunnel-client, which in turn launches the hidden
//! stdio worker. The API key is injected into the tunnel-client environment;
//! it is never included in either command line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Result};
use chrono::DateTime;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::config::{validate_secret_reference, Config};
use crate::instance_lock::InstanceLock;
use crate::state::CapabilityProfile;
use crate::tunnel_distribution::ResolvedTunnelClient;
use crate::utils::{agentic_home, ensure_parent, log_error, log_info, log_warn};

pub(crate) const WORKER_AUTH_ENV: &str = "AGENTIC_GPT_SUPERVISOR_TOKEN";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESTART_ATTEMPTS: usize = 5;
const READY_RESET_AFTER: Duration = Duration::from_secs(60);
const DOCTOR_DIAGNOSTIC_LIMIT: usize = 16 * 1024;
const BACKOFFS: [Duration; MAX_RESTART_ATTEMPTS] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

pub(crate) async fn run(config_path: PathBuf, profile: CapabilityProfile) -> Result<()> {
    log_info(format!(
        "standalone supervisor starting; profile={}; config={}",
        profile.label(),
        config_path.display()
    ));
    ensure_parent(&config_path)?;
    let _instance_lock = InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    if !config_path.exists() {
        crate::config::write_config_with_backup(&config_path, &Config::default_config()?)?;
    }
    let config = Config::load(&config_path)?;
    config.validate_standalone()?;
    config.ensure_workspace()?;
    if let Err(error) = crate::tmux::ensure_default_session(&config.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }

    let runtime_paths = RuntimePaths::prepare(&config.agent_id)?;
    let runtime_identity = StartupIdentity::from_config(&config, profile)?;
    let result = async {
        let tunnel = config
            .tunnel
            .as_ref()
            .ok_or_else(|| anyhow!("tunnel_config_required"))?;
        let secret = resolve_secret(&tunnel.api_key)?;
        let resolved = crate::tunnel_distribution::resolve(&config).await?;
        let invocation = Invocation::new(
            config_path.clone(),
            profile,
            tunnel.tunnel_id.clone(),
            secret,
            resolved,
            runtime_paths.clone(),
        )?;
        run_doctor(&invocation).await?;
        let watcher = tokio::spawn(watch_startup_identity(
            config_path.clone(),
            runtime_identity,
        ));
        let result = run_loop(&invocation, &runtime_paths).await;
        watcher.abort();
        result
    }
    .await;
    runtime_paths.cleanup();
    result
}

pub(crate) fn authorize_worker(token: Option<&str>) -> Result<()> {
    let token = token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("stdio_worker_unauthorized"))?;
    if std::env::var(WORKER_AUTH_ENV).ok().as_deref() != Some(token) {
        return Err(anyhow!("stdio_worker_unauthorized"));
    }
    Ok(())
}

fn resolve_secret(reference: &str) -> Result<String> {
    validate_secret_reference(reference)?;
    let value = if let Some(name) = reference.strip_prefix("env:") {
        std::env::var(name).map_err(|_| anyhow!("tunnel_api_key_unavailable"))?
    } else if let Some(path) = reference.strip_prefix("file:") {
        fs::read_to_string(path)
            .map_err(|_| anyhow!("tunnel_api_key_unavailable"))?
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    } else {
        return Err(anyhow!("tunnel_api_key_reference_plaintext_rejected"));
    };
    if value.trim().is_empty() {
        return Err(anyhow!("tunnel_api_key_unavailable"));
    }
    if value.chars().any(char::is_control) {
        return Err(anyhow!("tunnel_api_key_invalid"));
    }
    Ok(value)
}

#[derive(Clone, Debug)]
struct RuntimePaths {
    health_url: PathBuf,
    log: PathBuf,
    pid: PathBuf,
}

impl RuntimePaths {
    fn prepare(agent_id: &str) -> Result<Self> {
        if agent_id.is_empty()
            || !agent_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(anyhow!("runtime_identity_invalid"));
        }
        let root = agentic_home()?
            .join("runtime")
            .join("tunnel")
            .join(agent_id);
        fs::create_dir_all(&root).map_err(|_| anyhow!("runtime_directory_unavailable"))?;
        set_private_dir(&root)?;
        let paths = Self {
            health_url: root.join("health.url"),
            log: root.join("tunnel-client.log"),
            pid: root.join("tunnel-client.pid"),
        };
        fs::write(&paths.log, []).map_err(|_| anyhow!("runtime_directory_unavailable"))?;
        set_private_file(&paths.log)?;
        paths.remove_stale_files();
        Ok(paths)
    }

    fn remove_stale_files(&self) {
        for path in [&self.health_url, &self.pid] {
            let _ = fs::remove_file(path);
        }
    }

    fn cleanup(&self) {
        let _ = fs::remove_file(&self.health_url);
        let _ = fs::remove_file(&self.pid);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartupIdentity {
    agent_id: String,
    agent_secret: String,
    workspace_root: PathBuf,
    hub_url: String,
    hub_transport: String,
    hub_reporting_enabled: bool,
    hub_reporting_detail: crate::config::ReportingDetail,
    skill_max_concurrent_installs: usize,
    tunnel_id: String,
    api_key_reference: String,
    version: Option<String>,
    cache_dir: PathBuf,
    auto_download: bool,
    executable: Option<PathBuf>,
    download_url: Option<String>,
    sha256: Option<String>,
    profile: CapabilityProfile,
}

impl StartupIdentity {
    fn from_config(config: &Config, profile: CapabilityProfile) -> Result<Self> {
        let tunnel = config
            .tunnel
            .as_ref()
            .ok_or_else(|| anyhow!("tunnel_config_required"))?;
        Ok(Self {
            agent_id: config.agent_id.clone(),
            agent_secret: config.agent_secret.clone(),
            workspace_root: config.workspace_root.clone(),
            hub_url: config.hub_url.clone(),
            hub_transport: config.hub_transport.clone(),
            hub_reporting_enabled: tunnel.hub_reporting.enabled,
            hub_reporting_detail: tunnel.hub_reporting.detail,
            skill_max_concurrent_installs: config.skills.max_concurrent_installs,
            tunnel_id: tunnel.tunnel_id.clone(),
            api_key_reference: tunnel.api_key.clone(),
            version: tunnel.client.version.clone(),
            cache_dir: tunnel.client.cache_dir.clone(),
            auto_download: tunnel.client.auto_download,
            executable: tunnel.client.executable.clone(),
            download_url: tunnel.client.download_url.clone(),
            sha256: tunnel.client.sha256.clone(),
            profile,
        })
    }
}

#[derive(Debug, Default)]
struct StartupIdentityWatchState {
    observed_version: Option<SystemTime>,
    warned_version: Option<SystemTime>,
}

impl StartupIdentityWatchState {
    fn new(observed_version: Option<SystemTime>) -> Self {
        Self {
            observed_version,
            warned_version: None,
        }
    }

    fn observe_version(&mut self, version: Option<SystemTime>) -> bool {
        let Some(version) = version else {
            return false;
        };
        if self.observed_version == Some(version) {
            return false;
        }
        self.observed_version = Some(version);
        true
    }

    fn warn_once_for(&mut self, version: SystemTime) -> bool {
        if self.warned_version == Some(version) {
            return false;
        }
        self.warned_version = Some(version);
        true
    }
}

async fn watch_startup_identity(config_path: PathBuf, runtime_identity: StartupIdentity) {
    let mut state = StartupIdentityWatchState::new(
        fs::metadata(&config_path)
            .and_then(|meta| meta.modified())
            .ok(),
    );
    loop {
        sleep(Duration::from_secs(2)).await;
        let modified = fs::metadata(&config_path)
            .and_then(|meta| meta.modified())
            .ok();
        let Some(version) = modified else {
            continue;
        };
        if !state.observe_version(Some(version)) {
            continue;
        }
        let config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(error) => {
                if state.warn_once_for(version) {
                    log_warn(format!(
                        "standalone config reload failed; keeping current runtime; errorCode={}",
                        bounded_error_code(&error.to_string())
                    ));
                }
                continue;
            }
        };
        if let Err(error) = config.validate_standalone() {
            if state.warn_once_for(version) {
                log_warn(format!(
                    "standalone config reload failed; keeping current runtime; errorCode={}",
                    bounded_error_code(&error.to_string())
                ));
            }
            continue;
        }
        let current = match StartupIdentity::from_config(&config, runtime_identity.profile) {
            Ok(current) => current,
            Err(error) => {
                if state.warn_once_for(version) {
                    log_warn(format!(
                        "standalone config reload failed; keeping current runtime; errorCode={}",
                        bounded_error_code(&error.to_string())
                    ));
                }
                continue;
            }
        };
        if current != runtime_identity && state.warn_once_for(version) {
            log_warn("restart_required: standalone tunnel startup identity changed".to_owned());
        }
    }
}

fn bounded_error_code(value: &str) -> String {
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

#[derive(Clone)]
struct Invocation {
    tunnel_id: String,
    secret: String,
    executable: PathBuf,
    worker_command: String,
    worker_token: String,
    paths: RuntimePaths,
}

impl Invocation {
    fn new(
        config_path: PathBuf,
        profile: CapabilityProfile,
        tunnel_id: String,
        secret: String,
        resolved: ResolvedTunnelClient,
        paths: RuntimePaths,
    ) -> Result<Self> {
        let worker =
            std::env::current_exe().map_err(|_| anyhow!("worker_executable_unavailable"))?;
        let worker_token = Uuid::new_v4().to_string();
        let worker_command = format!(
            "{} stdio-worker --config {} --profile {} --supervisor-token {}",
            quote_arg(worker.to_string_lossy()),
            quote_arg(config_path.to_string_lossy()),
            match profile {
                CapabilityProfile::Normal => "normal",
                CapabilityProfile::Room => "room",
            },
            quote_arg(&worker_token),
        );
        Ok(Self {
            tunnel_id,
            secret,
            executable: resolved.path,
            worker_command,
            worker_token,
            paths,
        })
    }

    fn mcp_command(&self) -> String {
        format!("channel=main,command={}", self.worker_command)
    }

    fn common_args(&self, command: &str) -> Vec<String> {
        vec![
            command.to_owned(),
            "--control-plane.tunnel-id".to_owned(),
            self.tunnel_id.clone(),
            "--control-plane.api-key".to_owned(),
            "env:CONTROL_PLANE_API_KEY".to_owned(),
            "--health.listen-addr".to_owned(),
            "127.0.0.1:0".to_owned(),
            "--health.url-file".to_owned(),
            self.paths.health_url.to_string_lossy().into_owned(),
            "--mcp.command".to_owned(),
            self.mcp_command(),
        ]
    }

    fn doctor_args(&self) -> Vec<String> {
        let mut args = vec!["doctor".to_owned(), "--json".to_owned()];
        args.extend(self.common_args_without_command());
        args
    }

    fn run_args(&self) -> Vec<String> {
        let mut args = self.common_args("run");
        args.extend([
            "--log.format".to_owned(),
            "json".to_owned(),
            "--log.file".to_owned(),
            self.paths.log.to_string_lossy().into_owned(),
            "--pid.file".to_owned(),
            self.paths.pid.to_string_lossy().into_owned(),
        ]);
        args
    }

    fn common_args_without_command(&self) -> Vec<String> {
        let args = self.common_args("unused");
        args.into_iter().skip(1).collect()
    }

    fn command_env(&self, command: &mut Command) {
        command
            .env("CONTROL_PLANE_API_KEY", &self.secret)
            .env(WORKER_AUTH_ENV, &self.worker_token)
            .env_remove("OPENAI_API_KEY")
            .env_remove("AGENTIC_TUNNEL_API_KEY");
    }
}

struct SupervisorPolicy {
    startup_timeout: Duration,
    graceful_shutdown_timeout: Duration,
    ready_reset_after: Duration,
    backoffs: Vec<Duration>,
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        Self {
            startup_timeout: STARTUP_TIMEOUT,
            graceful_shutdown_timeout: GRACEFUL_SHUTDOWN_TIMEOUT,
            ready_reset_after: READY_RESET_AFTER,
            backoffs: BACKOFFS.to_vec(),
        }
    }
}

async fn run_loop(invocation: &Invocation, paths: &RuntimePaths) -> Result<()> {
    let policy = SupervisorPolicy::default();
    let mut signals = ShutdownSignals::new()?;
    let mut failures = 0usize;
    loop {
        let mut process = spawn_tunnel(invocation)?;
        let readiness = wait_until_ready(&mut process.child, paths, &policy, &mut signals).await?;
        match readiness {
            Readiness::Shutdown => {
                terminate(&mut process.child, policy.graceful_shutdown_timeout).await;
                process.stop_log_tasks();
                return Ok(());
            }
            Readiness::Ready => {}
            failure @ (Readiness::Exited(_) | Readiness::Timeout) => {
                kill_after_failure(&mut process.child).await;
                process.stop_log_tasks();
                failures += 1;
                let code = match failure {
                    Readiness::Exited(code) => code,
                    Readiness::Timeout => None,
                    _ => unreachable!(),
                };
                match restart_decision(failures, code, &policy.backoffs) {
                    RestartDecision::Permanent => {
                        return Err(anyhow!("standalone_permanent_child_failure"));
                    }
                    RestartDecision::Exhausted => {
                        return Err(anyhow!("standalone_restart_budget_exhausted"));
                    }
                    RestartDecision::Retry(delay) => {
                        if sleep_or_shutdown(delay, &mut signals).await {
                            return Ok(());
                        }
                    }
                }
                continue;
            }
        }

        match wait_running(&mut process.child, &policy, &mut signals, &mut failures).await? {
            RunResult::Shutdown => {
                terminate(&mut process.child, policy.graceful_shutdown_timeout).await;
                process.stop_log_tasks();
                return Ok(());
            }
            RunResult::Exited(code) => {
                process.stop_log_tasks();
                failures += 1;
                match restart_decision(failures, code, &policy.backoffs) {
                    RestartDecision::Permanent => {
                        return Err(anyhow!("standalone_permanent_child_failure"));
                    }
                    RestartDecision::Exhausted => {
                        return Err(anyhow!("standalone_restart_budget_exhausted"));
                    }
                    RestartDecision::Retry(delay) => {
                        if sleep_or_shutdown(delay, &mut signals).await {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

struct RunningProcess {
    child: Child,
    log_tasks: Vec<JoinHandle<()>>,
}

impl RunningProcess {
    fn stop_log_tasks(&mut self) {
        for task in self.log_tasks.drain(..) {
            task.abort();
        }
    }
}

fn spawn_tunnel(invocation: &Invocation) -> Result<RunningProcess> {
    let mut command = Command::new(&invocation.executable);
    command
        .args(invocation.run_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    invocation.command_env(&mut command);
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("tunnel_client_spawn_failed"))?;
    let mut log_tasks = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        log_tasks.push(tokio::spawn(forward_log(
            stdout,
            "tunnel.stdout",
            invocation.secret.clone(),
            invocation.worker_token.clone(),
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        log_tasks.push(tokio::spawn(forward_log(
            stderr,
            "tunnel.stderr",
            invocation.secret.clone(),
            invocation.worker_token.clone(),
        )));
    }
    Ok(RunningProcess { child, log_tasks })
}

async fn run_doctor(invocation: &Invocation) -> Result<()> {
    let mut command = Command::new(&invocation.executable);
    command
        .args(invocation.doctor_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    invocation.command_env(&mut command);
    let output = command
        .output()
        .await
        .map_err(|_| anyhow!("tunnel_doctor_spawn_failed"))?;
    if !output.status.success() {
        let exit_code = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned());
        let redactions = [invocation.secret.as_str(), invocation.worker_token.as_str()];
        let stdout = bounded_redacted_output(&output.stdout, &redactions);
        let stderr = bounded_redacted_output(&output.stderr, &redactions);
        return Err(anyhow!(
            "tunnel_doctor_failed: exit_code={exit_code}; stdout={stdout:?}; stderr={stderr:?}"
        ));
    }
    Ok(())
}

fn bounded_redacted_output(output: &[u8], secrets: &[&str]) -> String {
    let value = String::from_utf8_lossy(output).into_owned();
    let mut value = redact_sensitive(value, secrets);
    if value.len() > DOCTOR_DIAGNOSTIC_LIMIT {
        let mut end = DOCTOR_DIAGNOSTIC_LIMIT;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push_str("...[truncated]");
    }
    value.trim().to_owned()
}

fn redact_sensitive(mut value: String, secrets: &[&str]) -> String {
    for secret in secrets {
        if !secret.is_empty() {
            value = value.replace(secret, "[REDACTED]");
        }
    }
    value
}

async fn forward_log<R>(reader: R, component: &'static str, secret: String, worker_token: String)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let redactions = [secret.as_str(), worker_token.as_str()];
        let line = redact_sensitive(line, &redactions);
        let fallback = if component.ends_with("stderr") {
            ForwardedLevel::Warn
        } else {
            ForwardedLevel::Info
        };
        let forwarded = parse_forwarded_log(&line, fallback);
        let message = format!("{component}: {}", forwarded.message);
        match forwarded.level {
            ForwardedLevel::Info => log_info(message),
            ForwardedLevel::Warn => log_warn(message),
            ForwardedLevel::Error => log_error(message),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForwardedLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Eq, PartialEq)]
struct ForwardedLog {
    level: ForwardedLevel,
    message: String,
}

fn parse_forwarded_log(line: &str, fallback: ForwardedLevel) -> ForwardedLog {
    let Some((first, rest)) = line.split_once(' ') else {
        return ForwardedLog {
            level: fallback,
            message: line.to_owned(),
        };
    };
    let (level_token, message) = if DateTime::parse_from_rfc3339(first).is_ok() {
        let Some((level, message)) = rest.split_once(' ') else {
            return ForwardedLog {
                level: fallback,
                message: rest.to_owned(),
            };
        };
        (level, message)
    } else if matches!(first, "INFO" | "WARN" | "ERROR") {
        (first, rest)
    } else {
        return ForwardedLog {
            level: fallback,
            message: line.to_owned(),
        };
    };
    let level = match level_token {
        "INFO" => ForwardedLevel::Info,
        "WARN" => ForwardedLevel::Warn,
        "ERROR" => ForwardedLevel::Error,
        _ => {
            return ForwardedLog {
                level: fallback,
                message: if DateTime::parse_from_rfc3339(first).is_ok() {
                    rest.to_owned()
                } else {
                    line.to_owned()
                },
            }
        }
    };
    ForwardedLog {
        level,
        message: message.to_owned(),
    }
}

enum Readiness {
    Ready,
    Exited(Option<i32>),
    Timeout,
    Shutdown,
}

async fn wait_until_ready(
    child: &mut Child,
    paths: &RuntimePaths,
    policy: &SupervisorPolicy,
    signals: &mut ShutdownSignals,
) -> Result<Readiness> {
    let deadline = Instant::now() + policy.startup_timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| anyhow!("tunnel_client_wait_failed"))?
        {
            return Ok(Readiness::Exited(status.code()));
        }
        if let Some(base_url) = read_health_url(&paths.health_url) {
            if health_ready(&base_url).await {
                log_info("standalone tunnel ready".to_owned());
                return Ok(Readiness::Ready);
            }
        }
        if Instant::now() >= deadline {
            return Ok(Readiness::Timeout);
        }
        tokio::select! {
            _ = sleep(HEALTH_POLL_INTERVAL) => {}
            _ = signals.next() => return Ok(Readiness::Shutdown),
        }
    }
}

enum RunResult {
    Exited(Option<i32>),
    Shutdown,
}

#[derive(Debug, Eq, PartialEq)]
enum RestartDecision {
    Retry(Duration),
    Permanent,
    Exhausted,
}

fn restart_decision(
    failure_count: usize,
    exit_code: Option<i32>,
    backoffs: &[Duration],
) -> RestartDecision {
    if exit_code == Some(2) {
        return RestartDecision::Permanent;
    }
    backoffs
        .get(failure_count.saturating_sub(1))
        .copied()
        .map(RestartDecision::Retry)
        .unwrap_or(RestartDecision::Exhausted)
}

async fn wait_running(
    child: &mut Child,
    policy: &SupervisorPolicy,
    signals: &mut ShutdownSignals,
    failures: &mut usize,
) -> Result<RunResult> {
    let reset_sleep = sleep(policy.ready_reset_after);
    tokio::pin!(reset_sleep);
    let mut reset = false;
    loop {
        if reset {
            tokio::select! {
                _ = signals.next() => return Ok(RunResult::Shutdown),
                status = child.wait() => {
                    let status = status.map_err(|_| anyhow!("tunnel_client_wait_failed"))?;
                    return Ok(RunResult::Exited(status.code()));
                }
            }
        } else {
            tokio::select! {
                _ = signals.next() => return Ok(RunResult::Shutdown),
                status = child.wait() => {
                    let status = status.map_err(|_| anyhow!("tunnel_client_wait_failed"))?;
                    return Ok(RunResult::Exited(status.code()));
                }
                _ = &mut reset_sleep => {
                    *failures = 0;
                    reset = true;
                    log_info("standalone tunnel readiness reset restart budget".to_owned());
                }
            }
        }
    }
}

async fn sleep_or_shutdown(delay: Duration, signals: &mut ShutdownSignals) -> bool {
    tokio::select! {
        _ = sleep(delay) => false,
        _ = signals.next() => true,
    }
}

struct ShutdownSignals {
    interrupt: Signal,
    terminate: Signal,
}

impl ShutdownSignals {
    fn new() -> Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    async fn next(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }
}

fn read_health_url(path: &Path) -> Option<String> {
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let parsed = reqwest::Url::parse(value).ok()?;
    if parsed.scheme() != "http"
        || !parsed
            .host_str()
            .is_some_and(|host| host == "127.0.0.1" || host == "localhost")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(value.to_owned())
}

fn health_client(builder: reqwest::ClientBuilder) -> Option<reqwest::Client> {
    builder
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
}

async fn health_ready(base_url: &str) -> bool {
    let Some(client) = health_client(reqwest::Client::builder()) else {
        return false;
    };
    health_ready_with_client(&client, base_url).await
}

async fn health_ready_with_client(client: &reqwest::Client, base_url: &str) -> bool {
    let base_url = base_url.trim_end_matches('/');
    let health = format!("{base_url}/healthz");
    let ready = format!("{base_url}/readyz");
    matches!(client.get(health).send().await, Ok(response) if response.status().is_success())
        && matches!(client.get(ready).send().await, Ok(response) if response.status().is_success())
}

async fn terminate(child: &mut Child, grace: Duration) {
    send_process_signal(child.id(), signal_term());
    if timeout(grace, child.wait()).await.is_err() {
        send_process_signal(child.id(), signal_kill());
        let _ = child.kill().await;
        let _ = timeout(Duration::from_secs(1), child.wait()).await;
    }
}

async fn kill_after_failure(child: &mut Child) {
    send_process_signal(child.id(), signal_kill());
    let _ = child.kill().await;
    let _ = timeout(Duration::from_secs(1), child.wait()).await;
}

fn quote_arg(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if !value
        .bytes()
        .any(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\\'))
    {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
fn backoff_delay(attempt: usize) -> Duration {
    BACKOFFS
        .get(attempt.saturating_sub(1))
        .copied()
        .unwrap_or(*BACKOFFS.last().unwrap())
}

fn set_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow!("runtime_directory_unavailable"))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| anyhow!("runtime_directory_unavailable"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn send_process_signal(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid {
        unsafe {
            let _ = libc::kill(-(pid as libc::pid_t), signal);
        }
    }
}

#[cfg(not(unix))]
fn send_process_signal(_pid: Option<u32>, _signal: i32) {}

#[cfg(unix)]
fn signal_term() -> i32 {
    libc::SIGTERM
}

#[cfg(not(unix))]
fn signal_term() -> i32 {
    15
}

#[cfg(unix)]
fn signal_kill() -> i32 {
    libc::SIGKILL
}

#[cfg(not(unix))]
fn signal_kill() -> i32 {
    9
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn worker_command_quotes_paths_and_never_contains_api_key() {
        let token = "worker-token";
        let command = format!(
            "{} stdio-worker --config {} --profile normal --supervisor-token {}",
            quote_arg("/tmp/agentic worker"),
            quote_arg("/tmp/config with spaces.json"),
            token
        );
        assert!(command.contains("\"/tmp/agentic worker\""));
        assert!(!command.contains("runtime-api-key"));
    }

    #[test]
    fn mcp_binding_preserves_worker_tokenization() {
        let invocation = Invocation {
            tunnel_id: "tunnel_test".to_owned(),
            secret: "runtime-secret".to_owned(),
            executable: PathBuf::from("/tmp/tunnel-client"),
            worker_command:
                "\"/tmp/agentic worker\" stdio-worker --config \"/tmp/config with spaces.json\""
                    .to_owned(),
            worker_token: "worker-token".to_owned(),
            paths: RuntimePaths {
                health_url: PathBuf::from("/tmp/health.url"),
                log: PathBuf::from("/tmp/tunnel.log"),
                pid: PathBuf::from("/tmp/tunnel.pid"),
            },
        };

        assert_eq!(
            invocation.mcp_command(),
            "channel=main,command=\"/tmp/agentic worker\" stdio-worker --config \"/tmp/config with spaces.json\""
        );
    }

    #[test]
    fn doctor_diagnostic_output_is_bounded_and_redacted() {
        let output = format!(
            "prefix-secret-value-{}",
            "x".repeat(DOCTOR_DIAGNOSTIC_LIMIT)
        );
        let diagnostic = bounded_redacted_output(output.as_bytes(), &["secret-value"]);
        assert!(diagnostic.contains("[REDACTED]"));
        assert!(!diagnostic.contains("secret-value"));
        assert!(diagnostic.ends_with("...[truncated]"));
    }

    #[test]
    fn forwarded_child_lines_preserve_known_severity_and_strip_timestamp() {
        let timestamp = "2026-07-25T16:00:00+00:00";
        assert_eq!(
            parse_forwarded_log(
                &redact_sensitive(
                    format!("{timestamp} INFO child-ready secret").to_string(),
                    &["secret"],
                ),
                ForwardedLevel::Warn,
            ),
            ForwardedLog {
                level: ForwardedLevel::Info,
                message: "child-ready [REDACTED]".to_string(),
            }
        );
        let unknown = parse_forwarded_log("not-a-timestamp child output", ForwardedLevel::Info);
        assert_eq!(unknown.level, ForwardedLevel::Info);
        assert_eq!(unknown.message, "not-a-timestamp child output");
        let stderr = parse_forwarded_log(
            &format!("{timestamp} TRACE child warning"),
            ForwardedLevel::Warn,
        );
        assert_eq!(stderr.level, ForwardedLevel::Warn);
        assert_eq!(stderr.message, "TRACE child warning");
    }

    #[test]
    fn forwarded_journal_lines_preserve_untimestamped_severity_after_redaction() {
        for (level, expected) in [
            ("INFO", ForwardedLevel::Info),
            ("WARN", ForwardedLevel::Warn),
            ("ERROR", ForwardedLevel::Error),
        ] {
            let line = redact_sensitive(format!("{level} child secret"), &["secret"]);
            let parsed = parse_forwarded_log(&line, ForwardedLevel::Warn);
            assert_eq!(parsed.level, expected);
            assert_eq!(parsed.message, "child [REDACTED]");
        }
    }

    #[test]
    fn restart_identity_warning_compares_to_immutable_runtime_and_warns_once() {
        let runtime = test_startup_identity();
        let mut changed = runtime.clone();
        changed.agent_id = "agent-a".to_owned();
        let runtime_version = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let changed_version = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
        let returned_version = SystemTime::UNIX_EPOCH + Duration::from_secs(3);
        let mut state = StartupIdentityWatchState::new(Some(runtime_version));

        let mut warning_count = 0;
        for (version, current) in [
            (changed_version, &changed),
            (changed_version, &changed),
            (returned_version, &runtime),
        ] {
            if state.observe_version(Some(version))
                && current != &runtime
                && state.warn_once_for(version)
            {
                warning_count += 1;
            }
        }
        assert_eq!(warning_count, 1);
        assert_eq!(state.observed_version, Some(returned_version));
        assert_eq!(state.warned_version, Some(changed_version));
    }

    fn test_startup_identity() -> StartupIdentity {
        let mut config = Config::default_config().unwrap();
        config.tunnel = Some(crate::config::TunnelConfig::default());
        StartupIdentity::from_config(&config, CapabilityProfile::Normal).unwrap()
    }

    #[test]
    fn retry_schedule_is_bounded_and_exponential() {
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(5), Duration::from_secs(16));
        assert_eq!(backoff_delay(6), Duration::from_secs(16));
    }

    #[test]
    fn restart_decision_covers_retry_permanent_and_exhausted() {
        assert_eq!(
            restart_decision(1, None, &BACKOFFS),
            RestartDecision::Retry(Duration::from_secs(1))
        );
        assert_eq!(
            restart_decision(6, None, &BACKOFFS),
            RestartDecision::Exhausted
        );
        assert_eq!(
            restart_decision(1, Some(2), &BACKOFFS),
            RestartDecision::Permanent
        );
    }

    #[tokio::test]
    async fn health_probe_disables_configured_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let health_task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let response =
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                let _ = stream.write_all(response).await;
            }
        });
        let proxy = reqwest::Proxy::all("http://127.0.0.1:9").unwrap();
        let client = health_client(reqwest::Client::builder().proxy(proxy)).unwrap();

        assert!(health_ready_with_client(&client, &base_url).await);
        health_task.await.unwrap();
    }

    #[test]
    fn health_url_accepts_only_local_http_endpoints() {
        let root = std::env::temp_dir().join(format!("agentic-health-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("health.url");
        fs::write(&path, "http://127.0.0.1:1234\n").unwrap();
        assert_eq!(
            read_health_url(&path).as_deref(),
            Some("http://127.0.0.1:1234")
        );
        fs::write(&path, "https://127.0.0.1:1234\n").unwrap();
        assert!(read_health_url(&path).is_none());
        fs::write(&path, "http://example.com:1234\n").unwrap();
        assert!(read_health_url(&path).is_none());
    }

    #[test]
    fn runtime_paths_reject_path_injection() {
        assert_eq!(
            RuntimePaths::prepare("../escape").unwrap_err().to_string(),
            "runtime_identity_invalid"
        );
    }

    #[test]
    fn stale_health_and_pid_files_are_removed_before_start() {
        let root = std::env::temp_dir().join(format!("agentic-stale-runtime-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let paths = RuntimePaths {
            health_url: root.join("health.url"),
            log: root.join("tunnel.log"),
            pid: root.join("tunnel.pid"),
        };
        fs::write(&paths.health_url, "stale").unwrap();
        fs::write(&paths.pid, "stale").unwrap();
        paths.remove_stale_files();
        assert!(!paths.health_url.exists());
        assert!(!paths.pid.exists());
    }

    #[test]
    fn secret_reference_normalizes_trailing_line_endings_and_rejects_controls() {
        let root = std::env::temp_dir().join(format!("agentic-secret-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();

        let valid = root.join("valid");
        fs::write(&valid, "secret-value\r\n\n").unwrap();
        let value = resolve_secret(&format!("file:{}", valid.display())).unwrap();
        assert_eq!(value, "secret-value");

        let embedded_newline = root.join("embedded-newline");
        fs::write(&embedded_newline, "secret\nvalue\n").unwrap();
        assert_eq!(
            resolve_secret(&format!("file:{}", embedded_newline.display()))
                .unwrap_err()
                .to_string(),
            "tunnel_api_key_invalid"
        );

        let empty = root.join("empty");
        fs::write(&empty, "\r\n\n").unwrap();
        assert_eq!(
            resolve_secret(&format!("file:{}", empty.display()))
                .unwrap_err()
                .to_string(),
            "tunnel_api_key_unavailable"
        );

        assert_eq!(
            resolve_secret("literal-secret").unwrap_err().to_string(),
            "tunnel_api_key_reference_plaintext_rejected"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn doctor_failure_surfaces_redacted_stdout_stderr_and_exit_code() {
        let root = std::env::temp_dir().join(format!("agentic-doctor-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let script = root.join("fake-doctor.sh");
        fs::write(
            &script,
            "#!/bin/sh\nprintf 'public stdout %s\\n' \"$CONTROL_PLANE_API_KEY\"\nprintf 'public stderr %s token=%s\\n' \"$CONTROL_PLANE_API_KEY\" \"$AGENTIC_GPT_SUPERVISOR_TOKEN\" >&2\nexit 7\n",
        )
        .unwrap();
        set_executable(&script);
        let invocation = Invocation {
            tunnel_id: "tunnel_test".to_owned(),
            secret: "runtime-secret".to_owned(),
            executable: script,
            worker_command: "agentic-gpt stdio-worker --config config.json".to_owned(),
            worker_token: "worker-token".to_owned(),
            paths: RuntimePaths {
                health_url: root.join("health.url"),
                log: root.join("tunnel.log"),
                pid: root.join("tunnel.pid"),
            },
        };

        let error = run_doctor(&invocation).await.unwrap_err().to_string();
        assert!(error.contains("tunnel_doctor_failed"));
        assert!(error.contains("exit_code=7"));
        assert!(error.contains("public stdout [REDACTED]"));
        assert!(error.contains("public stderr [REDACTED] token=[REDACTED]"));
        assert!(!error.contains("runtime-secret"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn fake_tunnel_verifies_args_environment_health_and_shutdown() {
        let root = std::env::temp_dir().join(format!("agentic-supervisor-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let args_log = root.join("args.log");
        let script = root.join("fake-tunnel.sh");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let health_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let response =
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                let _ = stream.write_all(response).await;
            }
        });
        let script_text = format!(
            "#!/bin/sh\nif [ \"$1\" = \"doctor\" ]; then\n  printf '%s\\n' \"$@\" > '{}'\n  printf 'SECRET=%s\\n' \"$CONTROL_PLANE_API_KEY\" >> '{}'\n  exit 0\nfi\nprintf '%s\\n' \"$@\" >> '{}'\nurl_file=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--health.url-file\" ]; then shift; url_file=\"$1\"; fi\n  shift\ndone\nprintf 'http://127.0.0.1:{}\\n' > \"$url_file\"\nsleep 60\n",
            args_log.display(),
            args_log.display(),
            args_log.display(),
            port,
        );
        fs::write(&script, script_text).unwrap();
        set_executable(&script);
        let paths = RuntimePaths {
            health_url: root.join("health.url"),
            log: root.join("tunnel.log"),
            pid: root.join("tunnel.pid"),
        };
        let invocation = Invocation {
            tunnel_id: "tunnel_test".to_owned(),
            secret: "runtime-secret".to_owned(),
            executable: script,
            worker_command: "agentic-gpt stdio-worker --config config.json".to_owned(),
            worker_token: "worker-token".to_owned(),
            paths: paths.clone(),
        };

        run_doctor(&invocation).await.unwrap();
        let mut process = spawn_tunnel(&invocation).unwrap();
        let mut signals = ShutdownSignals::new().unwrap();
        let policy = SupervisorPolicy {
            startup_timeout: Duration::from_secs(2),
            graceful_shutdown_timeout: Duration::from_secs(1),
            ready_reset_after: Duration::from_secs(10),
            backoffs: vec![Duration::from_millis(1)],
        };
        assert!(matches!(
            wait_until_ready(&mut process.child, &paths, &policy, &mut signals)
                .await
                .unwrap(),
            Readiness::Ready
        ));
        let args = fs::read_to_string(&args_log).unwrap();
        let argv = args.split("SECRET=").next().unwrap_or_default();
        assert!(args.contains("--mcp.command"));
        assert!(args.contains("stdio-worker"));
        assert!(!argv.contains("runtime-secret"));
        assert!(args.contains("SECRET=runtime-secret"));
        terminate(&mut process.child, policy.graceful_shutdown_timeout).await;
        process.stop_log_tasks();
        health_task.abort();
    }

    fn set_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
