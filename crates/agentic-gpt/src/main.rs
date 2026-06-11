mod confirmation;
mod exec;
mod hub;
mod mcp;
mod notebook;
mod notify;
mod sessions;

use agentic_gpt_protocol::{
    AgentRole, PolicyCounts, SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary,
    SafePathRoot, SafePolicyRules, SafeRule, SafeSandboxSummary,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use mcp::{McpConfigCommand, McpServerConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const DEFAULT_BACKUP_LIMIT: usize = 5;
const EXEC_TIMEOUT_SECS: u64 = 30;
const CONFIRM_TIMEOUT_SECS: u64 = 45;
const STDOUT_MAX: usize = 64 * 1024;
const STDERR_MAX: usize = 64 * 1024;
const SESSION_TAIL_MAX: usize = 32 * 1024;
const RECONNECT_DELAY_SECS: u64 = 3;
const CONNECT_TIMEOUT_SECS: u64 = 20;
const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const HEARTBEAT_ACK_TIMEOUT_SECS: u64 = 45;

#[derive(Parser)]
#[command(name = "agentic-gpt")]
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
    Config {
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: ConfigCommand,
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
enum RuleCommand {
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
enum PathCommand {
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
enum PathRootCommand {
    Add { path: PathBuf },
    Remove { path: PathBuf },
}

#[derive(Clone, Copy, Debug)]
enum PathRootKind {
    Write,
    Readonly,
    Deny,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunMode {
    Normal,
    Room,
}

impl RunMode {
    fn role(self) -> AgentRole {
        match self {
            RunMode::Normal => AgentRole::Normal,
            RunMode::Room => AgentRole::Room,
        }
    }

    fn label(self) -> &'static str {
        match self {
            RunMode::Normal => "normal",
            RunMode::Room => "room",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    agent_id: String,
    display_name: String,
    #[serde(alias = "workerUrl")]
    hub_url: String,
    agent_secret: String,
    workspace_root: PathBuf,
    backup_limit: usize,
    confirmation_provider: ConfirmationProviderConfig,
    #[serde(default = "default_confirmation_language")]
    confirmation_language: String,
    sandbox: SandboxConfig,
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    path_policy: PathPolicyConfig,
    policy: PolicyConfig,
    limits: LimitsConfig,
    #[serde(default = "default_room_config")]
    room: RoomConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmationProviderConfig {
    provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SandboxConfig {
    enabled: bool,
    bubblewrap_path: String,
    required_runtime_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PathPolicyConfig {
    #[serde(default)]
    write_roots: Vec<PathBuf>,
    #[serde(default)]
    read_only_roots: Vec<PathBuf>,
    #[serde(default)]
    deny_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyConfig {
    allow: Vec<Rule>,
    confirm: Vec<Rule>,
    deny: Vec<Rule>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    program: String,
    args_prefix: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LimitsConfig {
    max_concurrent_tasks: usize,
    max_active_sessions: usize,
    session_idle_timeout_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notebook_root: Option<PathBuf>,
    timezone: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditRecord {
    task_id: Option<String>,
    session_id: Option<String>,
    time: DateTime<Utc>,
    program: String,
    args: Vec<String>,
    working_directory: Option<String>,
    need_confirm: bool,
    policy_decision: String,
    confirmation_result: Option<String>,
    exit_code: Option<i32>,
    duration_ms: u128,
    truncated: bool,
    request_source: String,
    reject_reason: Option<String>,
}

#[derive(Clone)]
struct AppState {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    run_mode: RunMode,
    sessions: Arc<Mutex<HashMap<String, sessions::ManagedSession>>>,
    hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    temporary_mcp_allows: Arc<Mutex<Vec<confirmation::TemporaryMcpAllow>>>,
    notebook_writes: Arc<Mutex<()>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { config } => run(config_path(config), RunMode::Normal).await,
        Commands::RunAsRoom { config } => run(config_path(config), RunMode::Room).await,
        Commands::Config { config, command } => handle_config(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf, run_mode: RunMode) -> Result<()> {
    log_info(format!(
        "agentic-gpt starting; mode={}; config={}",
        run_mode.label(),
        config_path.display(),
    ));
    ensure_parent(&config_path)?;
    if !config_path.exists() {
        write_config_with_backup(&config_path, &Config::default_config()?)?;
        log_info("default config created".to_string());
    }
    let initial = Config::load(&config_path)?;
    initial.ensure_workspace()?;
    log_info(format!(
        "config loaded; agentId={}; hubUrl={}; workspaceRoot={}; sandbox={}",
        initial.agent_id,
        initial.hub_url,
        initial.workspace_root.display(),
        if initial.sandbox.enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
    let state = AppState {
        config_path: config_path.clone(),
        config: Arc::new(RwLock::new(initial)),
        run_mode,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        notebook_writes: Arc::new(Mutex::new(())),
    };
    tokio::spawn(watch_config(state.clone()));
    hub::connect_loop(state).await
}

fn mcp_tool_command_preview(
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let arguments = serde_json::to_string_pretty(arguments)
        .unwrap_or_else(|_| serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string()));
    format!(
        "MCP Tool Call\nServer: {server_id}\nTool: {tool_name}\nArguments:\n{}",
        truncate_chars(&arguments, 2000)
    )
}

fn risk_level(program: &str) -> String {
    if risky_file_mutation(program) {
        "HIGH"
    } else if matches!(
        program,
        "curl" | "wget" | "docker" | "systemctl" | "service"
    ) {
        "MEDIUM"
    } else {
        "LOW"
    }
    .to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn risky_file_mutation(program: &str) -> bool {
    matches!(
        program,
        "rm" | "mv" | "chmod" | "chown" | "git" | "python" | "node"
    )
}

fn policy_decision(
    config: &Config,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    policy_decision_for_mode(config, RunMode::Normal, program, args, need_confirm)
}

fn policy_decision_for_mode(
    config: &Config,
    run_mode: RunMode,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    let mut decision = if need_confirm {
        PolicyDecision::Confirm
    } else {
        PolicyDecision::Allow
    };
    for rule in builtin_rules(run_mode, PolicyDecision::Confirm) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Confirm);
        }
    }
    for rule in builtin_rules(run_mode, PolicyDecision::Deny) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Deny);
        }
    }

    let mut configured_decision = None;
    for rule in &config.policy.allow {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Allow);
        }
    }
    for rule in &config.policy.confirm {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Confirm);
        }
    }
    for rule in &config.policy.deny {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Deny);
        }
    }

    configured_decision.unwrap_or(decision)
}

impl Rule {
    fn matches(&self, program: &str, args: &[String]) -> bool {
        self.program == program
            && args.len() >= self.args_prefix.len()
            && self
                .args_prefix
                .iter()
                .zip(args.iter())
                .all(|(expected, actual)| expected == actual)
    }
}

fn builtin_rules(run_mode: RunMode, decision: PolicyDecision) -> Vec<Rule> {
    let programs = match decision {
        PolicyDecision::Deny => vec!["su", "mkfs", "dd", "ssh"],
        PolicyDecision::Confirm if run_mode == RunMode::Room => {
            vec!["sudo", "mount", "systemctl", "service", "scp"]
        }
        PolicyDecision::Confirm => vec![
            "sudo",
            "rm",
            "mv",
            "chmod",
            "chown",
            "mount",
            "systemctl",
            "service",
            "docker",
            "scp",
            "curl",
            "wget",
            "bash",
            "sh",
            "zsh",
            "fish",
            "perl",
            "ruby",
        ],
        PolicyDecision::Allow => vec![],
    };
    let mut rules = programs
        .into_iter()
        .map(|program| Rule {
            program: program.to_string(),
            args_prefix: vec![],
        })
        .collect::<Vec<_>>();
    if decision == PolicyDecision::Confirm && run_mode == RunMode::Normal {
        rules.push(Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        });
        rules.push(Rule {
            program: "node".to_string(),
            args_prefix: vec!["-e".to_string()],
        });
    }
    rules
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
                        if paths_match(root, &old_workspace) {
                            *root = new_workspace.clone();
                        }
                    }
                    config.workspace_root = new_workspace;
                }
                "confirmationProvider" => config.confirmation_provider.provider = value,
                "confirmationLanguage" => {
                    config.confirmation_language = normalize_confirmation_language(&value)
                }
                "sandbox.enabled" => config.sandbox.enabled = value.parse::<bool>()?,
                "room.notebookRoot" => config.room.notebook_root = Some(PathBuf::from(value)),
                "room.timezone" => config.room.timezone = value,
                "hubUrl" | "workerUrl" => config.hub_url = value,
                "agentId" => config.agent_id = value,
                "agentSecret" => config.agent_secret = value,
                _ => return Err(anyhow!("unsupported config key: {key}")),
            }
            write_config_with_backup(&config_path, &config)?;
        }
        ConfigCommand::Allow { command } => {
            mutate_rule(config_path, PolicyDecision::Allow, command)?
        }
        ConfigCommand::Confirm { command } => {
            mutate_rule(config_path, PolicyDecision::Confirm, command)?
        }
        ConfigCommand::Deny { command } => mutate_rule(config_path, PolicyDecision::Deny, command)?,
        ConfigCommand::Path { command } => mutate_path_policy(config_path, command)?,
        ConfigCommand::Mcp { command } => mcp::mutate_servers(config_path, command)?,
    }
    Ok(())
}

fn mutate_rule(config_path: PathBuf, decision: PolicyDecision, command: RuleCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    let rules = match decision {
        PolicyDecision::Allow => &mut config.policy.allow,
        PolicyDecision::Confirm => &mut config.policy.confirm,
        PolicyDecision::Deny => &mut config.policy.deny,
    };
    match command {
        RuleCommand::Add {
            program,
            args_prefix,
        } => {
            let rule = Rule {
                program,
                args_prefix,
            };
            println!("added {}", rule_display(&rule));
            rules.push(rule);
        }
        RuleCommand::Remove {
            program,
            args_prefix,
        } => {
            remove_rule(rules, &program, &args_prefix)?;
        }
    }
    write_config_with_backup(&config_path, &config)
}

fn remove_rule(rules: &mut Vec<Rule>, program: &str, args_prefix: &[String]) -> Result<()> {
    remove_rule_with_interactive(rules, program, args_prefix, io::stdin().is_terminal())
}

fn remove_rule_with_interactive(
    rules: &mut Vec<Rule>,
    program: &str,
    args_prefix: &[String],
    interactive: bool,
) -> Result<()> {
    let matches = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.program == program && rule.args_prefix == args_prefix)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(anyhow!(
            "rule not found: {}",
            command_preview(program, args_prefix)
        )),
        1 => {
            let removed = rules.remove(matches[0]);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ if interactive => {
            let selected = choose_rule_interactively(rules, &matches)?;
            let removed = rules.remove(selected);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ => {
            eprintln!(
                "multiple rules match {}; rerun in an interactive terminal or provide a more specific args prefix:",
                command_preview(program, args_prefix)
            );
            for index in matches {
                eprintln!("  {}", rule_display(&rules[index]));
            }
            Err(anyhow!("multiple_matching_rules"))
        }
    }
}

fn choose_rule_interactively(rules: &[Rule], matches: &[usize]) -> Result<usize> {
    println!("multiple matching rules:");
    for (ordinal, index) in matches.iter().enumerate() {
        println!("  {}) {}", ordinal + 1, rule_display(&rules[*index]));
    }
    print!("select rule to remove [1-{}]: ", matches.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selected = input
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid selection"))?;
    if selected == 0 || selected > matches.len() {
        return Err(anyhow!("selection out of range"));
    }
    Ok(matches[selected - 1])
}

fn rule_display(rule: &Rule) -> String {
    command_preview(&rule.program, &rule.args_prefix)
}

fn mutate_path_policy(config_path: PathBuf, command: PathCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    match command {
        PathCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.path_policy)?);
            return Ok(());
        }
        PathCommand::Write { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Write, command)
        }
        PathCommand::Readonly { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Readonly, command)
        }
        PathCommand::Deny { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Deny, command)
        }
    }
    write_config_with_backup(&config_path, &config)
}

fn mutate_path_roots(policy: &mut PathPolicyConfig, kind: PathRootKind, command: PathRootCommand) {
    match command {
        PathRootCommand::Add { path } => {
            let roots = roots_for_kind(policy, kind);
            if !roots.iter().any(|existing| paths_match(existing, &path)) {
                roots.push(path);
            }
        }
        PathRootCommand::Remove { path } => {
            let roots = roots_for_kind(policy, kind);
            roots.retain(|existing| !paths_match(existing, &path));
        }
    }
}

fn roots_for_kind(policy: &mut PathPolicyConfig, kind: PathRootKind) -> &mut Vec<PathBuf> {
    match kind {
        PathRootKind::Write => &mut policy.write_roots,
        PathRootKind::Readonly => &mut policy.read_only_roots,
        PathRootKind::Deny => &mut policy.deny_roots,
    }
}

fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (
        exec::expand_pathbuf(left).and_then(|path| exec::canonicalize_existing_or_parent(&path)),
        exec::expand_pathbuf(right).and_then(|path| exec::canonicalize_existing_or_parent(&path)),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
            if let Ok(config) = Config::load(&state.config_path) {
                let _ = config.ensure_workspace();
                log_info(format!(
                    "config reloaded; agentId={}; workspaceRoot={}; sandbox={}",
                    config.agent_id,
                    config.workspace_root.display(),
                    if config.sandbox.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                *state.config.write().await = config;
                last_modified = modified;
            } else {
                log_warn("config reload failed; keeping previous config".to_string());
            }
        }
    }
}

fn default_confirmation_language() -> String {
    "en".to_string()
}

fn normalize_confirmation_language(language: &str) -> String {
    let language = language.trim();
    if language.eq_ignore_ascii_case("zh")
        || language.eq_ignore_ascii_case("zh-cn")
        || language.eq_ignore_ascii_case("zh_cn")
        || language.eq_ignore_ascii_case("cn")
    {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

fn confirmation_language_is_zh(config: &Config) -> bool {
    normalize_confirmation_language(&config.confirmation_language) == "zh-CN"
}

impl Config {
    fn default_config() -> Result<Self> {
        let base = agentic_home()?;
        Ok(Self {
            agent_id: "laptop".to_string(),
            display_name: hostname_fallback(),
            hub_url: "http://localhost:8787".to_string(),
            agent_secret: "change-me".to_string(),
            workspace_root: base.join("workspace"),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            confirmation_provider: ConfirmationProviderConfig {
                provider: "freedesktop-then-hub".to_string(),
            },
            confirmation_language: default_confirmation_language(),
            mcp_servers: BTreeMap::new(),
            sandbox: SandboxConfig {
                enabled: false,
                bubblewrap_path: "bwrap".to_string(),
                required_runtime_paths: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/etc/ssl"),
                ],
            },
            path_policy: default_path_policy(&base.join("workspace")),
            policy: PolicyConfig::default(),
            limits: LimitsConfig {
                max_concurrent_tasks: 2,
                max_active_sessions: 4,
                session_idle_timeout_secs: 3600,
            },
            room: default_room_config(),
        })
    }

    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let has_path_policy = value.get("pathPolicy").is_some();
        let mut config: Self = serde_json::from_value(value)?;
        if !has_path_policy {
            config.path_policy = default_path_policy(&config.workspace_root);
        }
        Ok(config)
    }

    fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Self::default_config()
        }
    }

    fn ensure_workspace(&self) -> Result<()> {
        fs::create_dir_all(&self.workspace_root)?;
        Ok(())
    }

    fn safe_summary(&self) -> SafeConfigSummary {
        let write_roots = safe_write_roots(self);
        SafeConfigSummary {
            workspace_root: workspace_root_summary(&self.workspace_root),
            sandbox: SafeSandboxSummary {
                enabled: self.sandbox.enabled,
                mode: if self.sandbox.enabled {
                    "bubblewrap"
                } else {
                    "disabled"
                }
                .to_string(),
            },
            path_policy: SafePathPolicySummary {
                write_root_count: write_roots.len(),
                read_only_root_count: self.path_policy.read_only_roots.len(),
                deny_root_count: self.path_policy.deny_roots.len(),
                write_roots,
                read_only_roots: safe_path_roots(
                    &self.path_policy.read_only_roots,
                    &self.workspace_root,
                    "configured",
                ),
                deny_roots: safe_path_roots(
                    &self.path_policy.deny_roots,
                    &self.workspace_root,
                    "configured",
                ),
            },
            policy_rule_counts: PolicyCounts {
                allow: self.policy.allow.len(),
                confirm: self.policy.confirm.len(),
                deny: self.policy.deny.len(),
            },
            policy_rules: SafePolicyRules {
                allow: safe_rules(&self.policy.allow),
                confirm: safe_rules(&self.policy.confirm),
                deny: safe_rules(&self.policy.deny),
                builtins: SafeBuiltinPolicyRules {
                    confirm: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Confirm)),
                    deny: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Deny)),
                },
            },
            confirmation_provider: self.confirmation_provider.provider.clone(),
        }
    }
}

fn workspace_root_summary(workspace_root: &Path) -> String {
    if *workspace_root == agentic_home().unwrap_or_default().join("workspace") {
        "default".to_string()
    } else {
        workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("workspace:{name}"))
            .unwrap_or_else(|| "configured".to_string())
    }
}

fn safe_write_roots(config: &Config) -> Vec<SafePathRoot> {
    let mut has_workspace_root = false;
    let mut roots = config
        .path_policy
        .write_roots
        .iter()
        .map(|root| {
            let is_workspace_root = paths_match(root, &config.workspace_root);
            has_workspace_root |= is_workspace_root;
            SafePathRoot {
                path: safe_path_display(root, &config.workspace_root),
                source: if is_workspace_root {
                    "workspaceRoot"
                } else {
                    "configured"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    if !has_workspace_root {
        roots.insert(
            0,
            SafePathRoot {
                path: "workspace".to_string(),
                source: "workspaceRoot".to_string(),
            },
        );
    }
    roots
}

fn safe_path_roots(roots: &[PathBuf], workspace_root: &Path, source: &str) -> Vec<SafePathRoot> {
    roots
        .iter()
        .map(|root| SafePathRoot {
            path: safe_path_display(root, workspace_root),
            source: source.to_string(),
        })
        .collect()
}

fn safe_path_display(path: &Path, workspace_root: &Path) -> String {
    if paths_match(path, workspace_root) {
        return "workspace".to_string();
    }
    let raw = path.to_string_lossy().to_string();
    if raw == "~" || raw.starts_with("~/") {
        return raw;
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    raw
}

fn safe_rules(rules: &[Rule]) -> Vec<SafeRule> {
    rules
        .iter()
        .map(|rule| SafeRule {
            program: rule.program.clone(),
            args_prefix: rule.args_prefix.clone(),
        })
        .collect()
}

fn default_room_config() -> RoomConfig {
    RoomConfig {
        notebook_root: None,
        timezone: "Asia/Shanghai".to_string(),
    }
}

fn default_path_policy(workspace_root: &Path) -> PathPolicyConfig {
    PathPolicyConfig {
        write_roots: vec![
            workspace_root.to_path_buf(),
            PathBuf::from("~/Documents"),
            PathBuf::from("~/Downloads"),
            PathBuf::from("/tmp"),
        ],
        read_only_roots: vec![
            PathBuf::from("~/.cache"),
            PathBuf::from("~/.local/share"),
            PathBuf::from("/etc/os-release"),
            PathBuf::from("/proc/meminfo"),
            PathBuf::from("/proc/cpuinfo"),
            PathBuf::from("/proc/loadavg"),
            PathBuf::from("/proc/uptime"),
            PathBuf::from("/sys/class/power_supply"),
            PathBuf::from("/sys/class/thermal"),
        ],
        deny_roots: vec![
            PathBuf::from("~/.ssh"),
            PathBuf::from("~/.gnupg"),
            PathBuf::from("~/.local/share/keyrings"),
            PathBuf::from("~/.password-store"),
            PathBuf::from("~/.mozilla"),
            PathBuf::from("~/.config/google-chrome"),
            PathBuf::from("~/.config/chromium"),
            PathBuf::from("~/.config/Code/User/globalStorage"),
            PathBuf::from("~/.npmrc"),
            PathBuf::from("~/.pypirc"),
            PathBuf::from("~/.cargo/credentials"),
            PathBuf::from("~/.docker/config.json"),
            PathBuf::from("~/.kube"),
            PathBuf::from("~/.aws"),
            PathBuf::from("~/.config/gcloud"),
            PathBuf::from("~/.config/gh"),
            PathBuf::from("~/.config/hub"),
            PathBuf::from("~/.config/clash"),
            PathBuf::from("~/.config/clash-verge"),
        ],
    }
}

fn write_config_with_backup(path: &Path, config: &Config) -> Result<()> {
    ensure_parent(path)?;
    if path.exists() {
        let backup_dir = path.parent().unwrap().join("backups");
        fs::create_dir_all(&backup_dir)?;
        let backup = backup_dir.join(format!("config.{}.json", Utc::now().timestamp_millis()));
        fs::copy(path, backup)?;
        prune_backups(&backup_dir, config.backup_limit)?;
    }
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn prune_backups(dir: &Path, limit: usize) -> Result<()> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("config."))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    while entries.len() > limit {
        if let Some(entry) = entries.first() {
            fs::remove_file(entry.path())?;
        }
        entries.remove(0);
    }
    Ok(())
}

fn write_audit(config: &Config, record: AuditRecord) -> Result<()> {
    let path = agentic_home()?.join("audit.log");
    ensure_parent(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    let _ = config;
    Ok(())
}

fn config_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        agentic_home()
            .unwrap_or_else(|_| PathBuf::from(".agentic_gpt"))
            .join("config.json")
    })
}

fn agentic_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory not found")?;
    Ok(home.join(".agentic_gpt"))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "agentic-gpt-linux".to_string())
}

fn command_preview(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("{part:?}")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn log_info(message: String) {
    log_line("INFO", message);
}

fn log_warn(message: String) {
    log_line("WARN", message);
}

fn log_line(level: &str, message: String) {
    eprintln!("{} {level} {message}", Utc::now().to_rfc3339());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::PreparedBatchElement;
    use agentic_gpt_protocol::{
        AgentMessage, BatchExecRequest, ExecElement, HubCommand, NotebookAppendRequest,
        NotebookRemoveRequest, NotebookUpdateRequest, PassageSignificance,
    };

    #[test]
    fn run_modes_declare_expected_roles() {
        assert_eq!(RunMode::Normal.role(), AgentRole::Normal);
        assert_eq!(RunMode::Room.role(), AgentRole::Room);
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
        config.room.timezone = "UTC".to_string();
        assert_eq!(config.room.timezone, "UTC");
    }

    #[test]
    fn normal_mode_room_command_error_is_structured() {
        let value = hub::room_agent_required_error();
        assert_eq!(value["error"]["code"], "room_agent_required");
        assert_eq!(
            value["error"]["message"],
            "room notebook commands require run-as-room"
        );
    }

    fn command_test_state(
        run_mode: RunMode,
        workspace_root: PathBuf,
    ) -> (AppState, mpsc::UnboundedReceiver<Message>) {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        let (tx, rx) = mpsc::unbounded_channel();
        (
            AppState {
                config_path: PathBuf::from("test-config.json"),
                config: Arc::new(RwLock::new(config)),
                run_mode,
                sessions: Arc::new(Mutex::new(HashMap::new())),
                hub_sender: Arc::new(Mutex::new(Some(tx))),
                pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
                temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
                notebook_writes: Arc::new(Mutex::new(())),
            },
            rx,
        )
    }

    async fn recv_response(rx: &mut mpsc::UnboundedReceiver<Message>) -> serde_json::Value {
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text response");
        };
        let message = serde_json::from_str::<AgentMessage>(&text).unwrap();
        let AgentMessage::Response { data, .. } = message else {
            panic!("expected agent response");
        };
        data
    }

    #[tokio::test]
    async fn normal_mode_rejects_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("normal-room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(RunMode::Normal, workspace);
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
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_agent_required");
    }

    #[tokio::test]
    async fn room_mode_executes_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(RunMode::Room, workspace);
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
            policy_decision_for_mode(&config, RunMode::Normal, "rm", &[], false),
            PolicyDecision::Confirm
        );
        assert_eq!(
            policy_decision_for_mode(&config, RunMode::Room, "rm", &[], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn room_policy_keeps_high_risk_commands_restricted() {
        let config = Config::default_config().unwrap();
        for program in ["sudo", "scp", "mount", "systemctl", "service"] {
            assert_eq!(
                policy_decision_for_mode(&config, RunMode::Room, program, &[], false),
                PolicyDecision::Confirm
            );
        }
        assert_eq!(
            policy_decision_for_mode(&config, RunMode::Room, "ssh", &[], false),
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
            policy_decision(&config, "git", &["status".to_string()], true),
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
            policy_decision(&config, "curl", &["--version".to_string()], false),
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
            policy_decision(&config, "ssh", &["-V".to_string()], false),
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
            policy_decision(&config, "git", &["push".to_string()], false),
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

    #[tokio::test]
    async fn batch_rejects_entire_group_when_confirmation_is_unavailable() {
        let root = unique_temp_dir("batch-confirm-unavailable");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("marker.txt"), "untouched").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.confirmation_provider.provider = "none".to_string();
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        };

        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            run_mode: RunMode::Normal,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
        };

        let result = exec::run_batch_task(
            state,
            "batch_test".to_string(),
            BatchExecRequest {
                agent_id: "test-agent".to_string(),
                elements: vec![
                    ExecElement {
                        program: "pwd".to_string(),
                        args: Vec::new(),
                        working_directory: None,
                    },
                    ExecElement {
                        program: "bash".to_string(),
                        args: vec!["-lc".to_string(), "echo changed > marker.txt".to_string()],
                        working_directory: None,
                    },
                ],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
            },
        )
        .await;

        assert_eq!(result.status, "rejected");
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].result.status, "skipped");
        assert_eq!(
            result.results[0].result.reject_reason.as_deref(),
            Some("batch_rejected")
        );
        assert_eq!(result.results[1].result.status, "rejected");
        assert_eq!(
            result.results[1].result.reject_reason.as_deref(),
            Some("batch_confirmation_confirmation_provider_unavailable")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("marker.txt")).unwrap(),
            "untouched"
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
            reject_reason: None,
        };
        let preview =
            confirmation::batch_confirmation_preview(&config, &[element.clone()], &[element]);

        assert!(preview.contains("该批次共有 1 条命令，其中 1 条需要确认"));
        assert!(preview.contains("工作目录：/tmp"));
        assert!(preview.contains("是否允许整个批次执行一次？"));
        assert!(!preview.contains("\\n"));
    }

    #[test]
    fn batch_prepare_detects_confirm_and_reject_before_execution() {
        let root = unique_temp_dir("batch-prepare");
        let workspace = root.join("workspace");
        let secret = workspace.join("secret");
        fs::create_dir_all(&secret).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: vec![secret],
        };

        let confirm = exec::prepare_batch_element(
            &config,
            0,
            ExecElement {
                program: "bash".to_string(),
                args: vec!["-lc".to_string(), "echo hi".to_string()],
                working_directory: None,
            },
            None,
            false,
        );
        assert_eq!(confirm.decision, PolicyDecision::Confirm);
        assert!(confirm.reject_reason.is_none());

        let rejected = exec::prepare_batch_element(
            &config,
            1,
            ExecElement {
                program: "cat".to_string(),
                args: vec!["./secret/token".to_string()],
                working_directory: None,
            },
            None,
            false,
        );
        assert_eq!(rejected.reject_reason.as_deref(), Some("path_denied"));
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

        remove_rule(&mut rules, "bash", &[]).unwrap();
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

        remove_rule(&mut rules, "python", &["-c".to_string()]).unwrap();
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

        let error = remove_rule_with_interactive(&mut rules, "bash", &[], false).unwrap_err();
        assert!(error.to_string().contains("multiple_matching_rules"));
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn path_root_remove_matches_expanded_equivalent_path() {
        let root = unique_temp_dir("path-cli");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        let mut policy = PathPolicyConfig::default();

        mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Add {
                path: target.clone(),
            },
        );
        assert_eq!(policy.write_roots.len(), 1);
        mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Remove {
                path: target.join("..").join("target"),
            },
        );
        assert!(policy.write_roots.is_empty());
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
