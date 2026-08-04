use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::Subcommand;

use crate::{
    config::{
        self, normalize_confirmation_language, write_config_with_backup, Config, ReportingDetail,
    },
    mcp::{self, McpConfigCommand},
    policy::{self, PolicyDecision},
};

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
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

pub(crate) async fn handle_config(config_path: PathBuf, command: ConfigCommand) -> Result<()> {
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
