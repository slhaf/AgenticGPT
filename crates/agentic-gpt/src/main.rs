mod audit;
mod config;
mod confirmation;
mod diary;
mod exec;
mod hub;
mod instance_lock;
mod mcp;
mod notebook;
mod notify;
mod policy;
mod sessions;
mod state;
mod tmux;
mod transport_ledger;
mod utils;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use config::{normalize_confirmation_language, write_config_with_backup, Config};
use mcp::McpConfigCommand;
use policy::PolicyDecision;
use state::{AppState, RunMode};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration};
use utils::{config_path, ensure_parent, log_info, log_warn};

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
    Tmux {
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: TmuxCommand,
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
        Commands::Run { config } => run(config_path(config), RunMode::Normal).await,
        Commands::RunAsRoom { config } => run(config_path(config), RunMode::Room).await,
        Commands::Config { config, command } => handle_config(config_path(config), command).await,
        Commands::Tmux { config, command } => handle_tmux(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf, run_mode: RunMode) -> Result<()> {
    log_info(format!(
        "agentic-gpt starting; mode={}; config={}",
        run_mode.label(),
        config_path.display(),
    ));
    ensure_parent(&config_path)?;
    let _instance_lock = instance_lock::InstanceLock::acquire(&config_path, ".run.lock", "agent")?;
    if !config_path.exists() {
        write_config_with_backup(&config_path, &Config::default_config()?)?;
        log_info("default config created".to_string());
    }
    let initial = Config::load(&config_path)?;
    initial.ensure_workspace()?;
    if let Err(error) = tmux::ensure_default_session(&initial.workspace_root).await {
        log_warn(format!("default tmux session unavailable: {error}"));
    }
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
                "confirmationProvider" => config.confirmation_provider.provider = value,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PathPolicyConfig, Rule};
    use crate::exec::PreparedBatchElement;
    use crate::policy::policy_decision;
    use agentic_gpt_protocol::{
        AgentMessage, BatchExecRequest, ExecElement, HubCommand, NotebookAppendRequest,
        NotebookRemoveRequest, NotebookUpdateRequest, PassageSignificance,
    };
    use tokio::sync::mpsc;
    use uuid::Uuid;

    #[test]
    fn run_modes_declare_expected_roles() {
        assert_eq!(
            RunMode::Normal.role(),
            agentic_gpt_protocol::AgentRole::Normal
        );
        assert_eq!(RunMode::Room.role(), agentic_gpt_protocol::AgentRole::Room);
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
            "room notebook commands require run-as-room"
        );
    }

    fn command_test_state(
        run_mode: RunMode,
        workspace_root: PathBuf,
    ) -> (AppState, mpsc::UnboundedReceiver<AgentMessage>) {
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
            policy::policy_decision_for_mode(&config, RunMode::Normal, "rm", &[], false),
            PolicyDecision::Confirm
        );
        assert_eq!(
            policy::policy_decision_for_mode(&config, RunMode::Room, "rm", &[], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn room_policy_keeps_high_risk_commands_restricted() {
        let config = Config::default_config().unwrap();
        for program in ["sudo", "scp", "mount", "systemctl", "service"] {
            assert_eq!(
                policy::policy_decision_for_mode(&config, RunMode::Room, program, &[], false),
                PolicyDecision::Confirm
            );
        }
        assert_eq!(
            policy::policy_decision_for_mode(&config, RunMode::Room, "ssh", &[], false),
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
