mod config_keys;

use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::Subcommand;

use crate::{
    cli_i18n::{self, UiLanguage},
    config::{
        self, acquire_config_mutation_lock,
        mcp_servers::{self, McpConfigCommand},
        ordered_config_json, write_config_with_backup, Config, ToolNamespace,
    },
    config_setup::SetupSeed,
    config_templates::{self, InitInput, InitSummary, RuntimeMode, SecretValue},
    policy::{self, PolicyDecision},
    WorkerProfile,
};

#[derive(clap::Args, Clone, Default)]
pub(crate) struct ConfigInitArgs {
    #[arg(long, value_enum)]
    pub(crate) mode: Option<RuntimeMode>,
    #[arg(long, value_enum)]
    pub(crate) profile: Option<WorkerProfile>,
    #[arg(long)]
    pub(crate) non_interactive: bool,
    #[arg(long)]
    pub(crate) http_mcp_enabled: Option<bool>,
    #[arg(long)]
    pub(crate) http_mcp_host: Option<String>,
    #[arg(long)]
    pub(crate) http_mcp_port: Option<u16>,
    #[arg(long)]
    pub(crate) http_mcp_public_url: Option<String>,
    #[arg(long)]
    pub(crate) http_mcp_bearer_token: Option<String>,
    #[arg(long)]
    pub(crate) http_mcp_allow_hosts: Option<String>,
    #[arg(long)]
    pub(crate) tunnel_id: Option<String>,
    #[arg(long)]
    pub(crate) tunnel_api_key: Option<String>,
    #[arg(long)]
    pub(crate) hub_url: Option<String>,
    #[arg(long, value_parser = ["websocket", "sse"])]
    pub(crate) hub_transport: Option<String>,
    #[arg(long)]
    pub(crate) agent_id: Option<String>,
    #[arg(
        long,
        help = "Agent secret (visible to local process inspection and shell history; interactive hidden input is preferred)"
    )]
    pub(crate) agent_secret: Option<String>,
}

#[derive(clap::Args, Clone, Default)]
pub(crate) struct ConfigImportArgs {
    /// Legacy or external JSON source. If omitted, import the selected --config path.
    #[arg(value_name = "SOURCE")]
    pub(crate) source: Option<PathBuf>,
}

pub(crate) fn init_non_interactive(
    config_path: &Path,
    args: &ConfigInitArgs,
    language: UiLanguage,
) -> Result<InitSummary> {
    let mut input = InitInput::non_interactive_defaults(language);
    input.mode = args.mode.unwrap_or(input.mode);
    input.profile = args.profile.unwrap_or(input.profile);
    input.http_mcp_enabled = args.http_mcp_enabled;
    input.http_mcp_host = args.http_mcp_host.clone();
    input.http_mcp_port = args.http_mcp_port;
    input.http_mcp_public_url = args.http_mcp_public_url.clone();
    input.http_mcp_bearer_token = args
        .http_mcp_bearer_token
        .as_ref()
        .map(|value| SecretValue::new(value.clone()));
    input.http_mcp_allow_hosts = args.http_mcp_allow_hosts.clone();
    if let Some(public_url) = args.http_mcp_public_url.as_deref() {
        config::normalize_http_mcp_public_url(public_url)?;
    }
    input.tunnel_id = args.tunnel_id.clone();
    input.tunnel_api_key = args.tunnel_api_key.clone();
    input.hub_url = args.hub_url.clone();
    input.hub_transport = args.hub_transport.clone();
    input.agent_id = args.agent_id.clone();
    input.agent_secret = args
        .agent_secret
        .as_ref()
        .map(|value| SecretValue::new(value.clone()));

    let built = config_templates::build_config(input)?;
    let _lock = acquire_config_mutation_lock(config_path)?;
    write_config_with_backup(config_path, &built.config)?;
    Ok(InitSummary {
        mode: built.mode,
        profile: built.profile,
        config_path: config_path.to_path_buf(),
        pending: built.pending,
    })
}

pub(crate) fn setup_seed_from_args(args: &ConfigInitArgs) -> SetupSeed {
    SetupSeed {
        mode: args.mode,
        profile: args.profile,
        http_mcp_enabled: args.http_mcp_enabled,
        http_mcp_host: args.http_mcp_host.clone(),
        http_mcp_port: args.http_mcp_port,
        http_mcp_public_url: args.http_mcp_public_url.clone(),
        http_mcp_bearer_token: args
            .http_mcp_bearer_token
            .as_ref()
            .map(|value| SecretValue::new(value.clone())),
        http_mcp_allow_hosts: args.http_mcp_allow_hosts.clone(),
        imported_base: None,
        tunnel_id: args.tunnel_id.clone(),
        tunnel_api_key: args.tunnel_api_key.clone(),
        hub_url: args.hub_url.clone(),
        hub_transport: args.hub_transport.clone(),
        agent_id: args.agent_id.clone(),
        agent_secret: args
            .agent_secret
            .as_ref()
            .map(|value| SecretValue::new(value.clone())),
    }
}

pub(crate) fn should_use_interactive_init(
    non_interactive: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    !non_interactive && stdin_is_terminal && stdout_is_terminal && stderr_is_terminal
}

fn process_should_use_interactive_init(non_interactive: bool) -> bool {
    should_use_interactive_init(
        non_interactive,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

fn interactive_init_required_message(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => {
            "Interactive config init requires a TTY; re-run with --non-interactive for piped or scripted use."
        }
        UiLanguage::ZhCn => "交互式配置初始化需要 TTY；管道或脚本场景请使用 --non-interactive。",
    }
}

fn handle_init(config_path: &Path, args: ConfigInitArgs, language: UiLanguage) -> Result<()> {
    let (summary, print_pending) = if args.non_interactive {
        (init_non_interactive(config_path, &args, language)?, true)
    } else if process_should_use_interactive_init(args.non_interactive) {
        match crate::config_tui::run_config_tui(config_path, setup_seed_from_args(&args), language)
        {
            Ok(summary) => (summary, false),
            Err(error) if error.to_string() == "config_init_cancelled" => {
                println!("{}", cli_i18n::text(language).cancelled);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    } else {
        return Err(anyhow!(interactive_init_required_message(language)));
    };
    let _ = (summary.mode, summary.profile);
    println!(
        "{} {}",
        cli_i18n::text(language).initialized,
        summary.config_path.display()
    );
    if print_pending {
        for action in summary.pending {
            eprintln!("{}", cli_i18n::pending_action_text(action, language));
        }
    }
    Ok(())
}

fn handle_import(config_path: &Path, args: ConfigImportArgs, language: UiLanguage) -> Result<()> {
    let source_path = args.source.unwrap_or_else(|| config_path.to_path_buf());
    let imported = Config::import(&source_path)?;
    for warning in &imported.warnings {
        eprintln!("config import: field {warning}");
    }
    if !process_should_use_interactive_init(false) {
        return Err(anyhow!(interactive_init_required_message(language)));
    }
    let seed = SetupSeed {
        mode: Some(imported.config.mode),
        profile: Some(imported.config.profile),
        imported_base: Some(imported.config),
        ..SetupSeed::default()
    };
    match crate::config_tui::run_config_tui(config_path, seed, language) {
        Ok(summary) => {
            println!(
                "{} {}",
                cli_i18n::text(language).initialized,
                summary.config_path.display()
            );
            Ok(())
        }
        Err(error) if error.to_string() == "config_init_cancelled" => {
            println!("{}", cli_i18n::text(language).cancelled);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    Init(ConfigInitArgs),
    Import(ConfigImportArgs),
    Show,
    Keys {
        #[arg(long, value_enum)]
        section: Option<config_keys::ConfigSection>,
        #[arg(long)]
        json: bool,
    },
    Set {
        key: String,
        value: String,
    },
    Unset {
        key: String,
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
    Toolset {
        #[command(subcommand)]
        command: ToolsetCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ToolsetCommand {
    Ls,
    Enable {
        #[arg(value_enum)]
        namespace: ToolNamespace,
    },
    Disable {
        #[arg(value_enum)]
        namespace: ToolNamespace,
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

pub(crate) async fn handle_config(
    config_path: PathBuf,
    command: ConfigCommand,
    language: UiLanguage,
) -> Result<()> {
    match command {
        ConfigCommand::Init(args) => handle_init(&config_path, args, language)?,
        ConfigCommand::Import(args) => handle_import(&config_path, args, language)?,
        ConfigCommand::Show => {
            let config = Config::load(&config_path)?;
            println!("{}", ordered_config_json(&config)?);
        }
        ConfigCommand::Keys { section, json } => {
            config_keys::print_config_keys(section, json, language)?;
        }
        ConfigCommand::Set { key, value } => {
            let _lock = acquire_config_mutation_lock(&config_path)?;
            let mut config = Config::load_or_default_locked(&config_path)?;
            config_keys::apply_config_key(&mut config, &key, &value)?;
            write_config_with_backup(&config_path, &config)?;
        }
        ConfigCommand::Unset { key } => {
            let _lock = acquire_config_mutation_lock(&config_path)?;
            let mut config = Config::load_or_default_locked(&config_path)?;
            config_keys::unset_config_key(&mut config, &key)?;
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
        ConfigCommand::Mcp { command } => mcp_servers::mutate_servers(config_path, command)?,
        ConfigCommand::Toolset { command } => handle_toolset(&config_path, command, language)?,
    }
    Ok(())
}
fn handle_toolset(config_path: &Path, command: ToolsetCommand, language: UiLanguage) -> Result<()> {
    let _lock = acquire_config_mutation_lock(config_path)?;
    let mut config = Config::load_locked(config_path)?;
    match command {
        ToolsetCommand::Ls => println!("{}", render_toolsets(&config, language)),
        ToolsetCommand::Enable { namespace } => {
            config.toolsets.enable(namespace);
            write_config_with_backup(config_path, &config)?;
            println!("{}", toolset_mutation_message(namespace, true, language));
        }
        ToolsetCommand::Disable { namespace } => {
            config.toolsets.disable(namespace);
            write_config_with_backup(config_path, &config)?;
            println!("{}", toolset_mutation_message(namespace, false, language));
        }
    }
    Ok(())
}

fn render_toolsets(config: &Config, language: UiLanguage) -> String {
    ToolNamespace::all()
        .iter()
        .copied()
        .map(|namespace| {
            let enabled = config.toolsets.is_enabled(namespace);
            let status = match (language, enabled) {
                (UiLanguage::En, true) => "enabled",
                (UiLanguage::En, false) => "disabled",
                (UiLanguage::ZhCn, true) => "启用",
                (UiLanguage::ZhCn, false) => "禁用",
            };
            format!(
                "[{status}]\t{}\t{}",
                namespace.as_str(),
                toolset_description(namespace, language)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn toolset_description(namespace: ToolNamespace, language: UiLanguage) -> &'static str {
    match (namespace, language) {
        (ToolNamespace::Agent, UiLanguage::En) => {
            "Agent runtime information and health diagnostics."
        }
        (ToolNamespace::Agent, UiLanguage::ZhCn) => "Agent 运行信息与健康诊断。",
        (ToolNamespace::File, UiLanguage::En) => "Workspace file reading, search, and editing.",
        (ToolNamespace::File, UiLanguage::ZhCn) => "工作区文件读取、搜索与编辑。",
        (ToolNamespace::Mcp, UiLanguage::En) => "Downstream MCP server discovery and tool calls.",
        (ToolNamespace::Mcp, UiLanguage::ZhCn) => "下游 MCP 服务发现与工具调用。",
        (ToolNamespace::Process, UiLanguage::En) => {
            "Managed local process execution and lifecycle control."
        }
        (ToolNamespace::Process, UiLanguage::ZhCn) => "受管本地进程执行与生命周期控制。",
        (ToolNamespace::Skills, UiLanguage::En) => {
            "Skill discovery, installation, activation, and execution."
        }
        (ToolNamespace::Skills, UiLanguage::ZhCn) => "技能发现、安装、启用与执行。",
        (ToolNamespace::Tmux, UiLanguage::En) => "Persistent tmux session and pane operations.",
        (ToolNamespace::Tmux, UiLanguage::ZhCn) => "持久化 tmux 会话与窗格操作。",
        (ToolNamespace::Browser, UiLanguage::En) => {
            "Browser runtime manual and persistent JavaScript sessions."
        }
        (ToolNamespace::Browser, UiLanguage::ZhCn) => "浏览器运行时手册与持久化 JavaScript 会话。",
        (ToolNamespace::Room, UiLanguage::En) => {
            "Room bootstrap, diary, notebook, state, and maintenance tools."
        }
        (ToolNamespace::Room, UiLanguage::ZhCn) => "Room 引导、日记、笔记本、状态与维护工具。",
    }
}

fn toolset_mutation_message(
    namespace: ToolNamespace,
    enabled: bool,
    language: UiLanguage,
) -> String {
    match (language, enabled) {
        (UiLanguage::En, true) => format!("Enabled toolset: {namespace}."),
        (UiLanguage::En, false) => format!("Disabled toolset: {namespace}."),
        (UiLanguage::ZhCn, true) => format!("已启用工具集：{namespace}。"),
        (UiLanguage::ZhCn, false) => format!("已禁用工具集：{namespace}。"),
    }
}

#[cfg(test)]
mod tests {
    use super::config_keys::{apply_config_key, CONFIG_KEYS};
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::config::RoomMaintenanceMode;
    use clap::Parser as _;

    #[test]
    fn toolset_commands_dispatch_and_reject_unknown_namespaces() {
        let cli = Cli::try_parse_from(["agentic-gpt", "config", "toolset", "ls"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommand::Toolset {
                    command: ToolsetCommand::Ls
                },
                ..
            }
        ));

        let cli =
            Cli::try_parse_from(["agentic-gpt", "config", "toolset", "enable", "file"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommand::Toolset {
                    command: ToolsetCommand::Enable {
                        namespace: ToolNamespace::File
                    }
                },
                ..
            }
        ));

        let cli =
            Cli::try_parse_from(["agentic-gpt", "config", "toolset", "disable", "room"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommand::Toolset {
                    command: ToolsetCommand::Disable {
                        namespace: ToolNamespace::Room
                    }
                },
                ..
            }
        ));

        assert!(
            Cli::try_parse_from(["agentic-gpt", "config", "toolset", "enable", "unknown",])
                .is_err()
        );
    }

    #[test]
    fn toolset_enable_and_disable_persist_across_config_loads() {
        let root = std::env::temp_dir().join(format!(
            "agentic-config-cli-toolset-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.json");
        let mut config = Config::default_config().unwrap();
        config.toolsets.disable(ToolNamespace::File);
        write_config_with_backup(&path, &config).unwrap();

        handle_toolset(
            &path,
            ToolsetCommand::Enable {
                namespace: ToolNamespace::File,
            },
            UiLanguage::En,
        )
        .unwrap();
        assert!(Config::load(&path)
            .unwrap()
            .toolsets
            .is_enabled(ToolNamespace::File));

        handle_toolset(
            &path,
            ToolsetCommand::Disable {
                namespace: ToolNamespace::File,
            },
            UiLanguage::En,
        )
        .unwrap();
        assert!(!Config::load(&path)
            .unwrap()
            .toolsets
            .is_enabled(ToolNamespace::File));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn toolset_listing_describes_all_namespaces_and_feedback_is_localized() {
        let config = Config::default_config().unwrap();

        let english = render_toolsets(&config, UiLanguage::En);
        assert_eq!(english.lines().count(), ToolNamespace::all().len());
        assert!(
            english.contains("[enabled]\tagent\tAgent runtime information and health diagnostics.")
        );
        assert!(english.contains(
            "[enabled]\tbrowser\tBrowser runtime manual and persistent JavaScript sessions."
        ));
        assert!(english.contains(
            "[disabled]\troom\tRoom bootstrap, diary, notebook, state, and maintenance tools."
        ));

        let chinese = render_toolsets(&config, UiLanguage::ZhCn);
        assert!(chinese.contains("[启用]\tfile\t工作区文件读取、搜索与编辑。"));
        assert!(chinese.contains("[禁用]\troom\tRoom 引导、日记、笔记本、状态与维护工具。"));
        assert_eq!(
            toolset_mutation_message(ToolNamespace::File, true, UiLanguage::ZhCn),
            "已启用工具集：file。"
        );
        assert_eq!(
            toolset_mutation_message(ToolNamespace::Room, false, UiLanguage::En),
            "Disabled toolset: room."
        );
    }

    #[test]
    fn interactive_init_requires_all_three_terminals_and_no_non_interactive_flag() {
        let cases = [
            (true, true, true, true, false),
            (false, false, true, true, false),
            (false, true, false, true, false),
            (false, true, true, false, false),
            (false, false, false, false, false),
            (false, true, true, true, true),
        ];

        for (non_interactive, stdin, stdout, stderr, expected) in cases {
            assert_eq!(
                should_use_interactive_init(non_interactive, stdin, stdout, stderr),
                expected,
                "unexpected interactive-init decision"
            );
        }
    }

    #[test]
    fn registry_applies_new_scalar_and_list_keys() {
        let mut config = Config::default_config().unwrap();
        apply_config_key(&mut config, "displayName", "Desk Agent").unwrap();
        apply_config_key(&mut config, "backupLimit", "7").unwrap();
        apply_config_key(
            &mut config,
            "sandbox.requiredRuntimePaths",
            r#"["/usr","/opt/runtime"]"#,
        )
        .unwrap();
        apply_config_key(&mut config, "limits.maxConcurrentTasks", "4").unwrap();
        apply_config_key(&mut config, "limits.maxActiveProcesses", "auto").unwrap();
        apply_config_key(&mut config, "limits.maxFileSearchContextLines", "12").unwrap();

        assert_eq!(config.display_name, "Desk Agent");
        assert_eq!(config.backup_limit, 7);
        assert_eq!(config.sandbox.required_runtime_paths.len(), 2);
        assert_eq!(config.limits.max_concurrent_tasks, 4);
        assert_eq!(config.limits.max_file_search_context_lines, 12);
    }

    #[test]
    fn registry_updates_room_repository_and_maintenance_settings() {
        let mut config = Config::default_config().unwrap();
        apply_config_key(&mut config, "room.repositoryRoot", "/tmp/repository").unwrap();
        apply_config_key(&mut config, "room.maintenance.mode", "workflow").unwrap();
        apply_config_key(&mut config, "room.maintenance.autoPush", "true").unwrap();
        assert_eq!(
            config.room.repository_root.as_deref(),
            Some(std::path::Path::new("/tmp/repository"))
        );
        assert_eq!(config.room.maintenance.mode, RoomMaintenanceMode::Workflow);
        assert!(config.room.maintenance.auto_push);

        apply_config_key(&mut config, "room.repositoryRoot", "null").unwrap();
        assert!(config.room.repository_root.is_none());
        assert!(apply_config_key(&mut config, "room.maintenance.mode", "invalid").is_err());
        assert!(apply_config_key(&mut config, "room.notebookRoot", "null").is_err());
    }

    #[test]
    fn registry_keys_are_unique_and_have_bilingual_metadata() {
        let mut seen = std::collections::BTreeSet::new();
        for spec in CONFIG_KEYS {
            assert!(seen.insert(spec.key), "duplicate key: {}", spec.key);
            assert!(!spec.description.en.is_empty());
            assert!(!spec.description.zh_cn.is_empty());
            assert!(!spec.example.is_empty());
        }
    }

    #[test]
    fn setup_seed_conversion_preserves_editable_flags_and_redacts_agent_secret() {
        let marker = "setup-seed-secret-marker";
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Room),
            tunnel_id: Some("seed-tunnel".to_string()),
            tunnel_api_key: Some("env:TUNNEL_KEY".to_string()),
            hub_url: Some("https://hub.example.com".to_string()),
            hub_transport: Some("sse".to_string()),
            agent_id: Some("desk".to_string()),
            agent_secret: Some(marker.to_string()),
            ..ConfigInitArgs::default()
        };

        let seed = setup_seed_from_args(&args);
        assert_eq!(seed.mode, args.mode);
        assert_eq!(seed.profile, args.profile);
        assert_eq!(seed.tunnel_id.as_deref(), Some("seed-tunnel"));
        assert_eq!(seed.tunnel_api_key.as_deref(), Some("env:TUNNEL_KEY"));
        assert_eq!(seed.hub_url.as_deref(), Some("https://hub.example.com"));
        assert_eq!(seed.hub_transport.as_deref(), Some("sse"));
        assert_eq!(seed.agent_id.as_deref(), Some("desk"));
        assert_eq!(seed.agent_secret.as_ref().unwrap().expose(), marker);
        assert!(!format!("{seed:?}").contains(marker));
    }
}
