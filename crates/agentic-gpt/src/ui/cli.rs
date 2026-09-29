// Command-line definitions and command handlers.
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use tokio::time::{sleep, Duration};

use crate::{
    cli_i18n::{self, LanguageChoice},
    config::{Config, WorkerProfile},
    config_cli::{self, ConfigCommand},
    local_control, operation, startup,
    state::RuntimeModel,
    tmux, tui,
    utils::config_path,
};
pub(crate) async fn run() -> Result<()> {
    let args = std::env::args_os().collect::<Vec<_>>();
    let choice = cli_i18n::prescan_language(&args).unwrap_or(LanguageChoice::Auto);
    let language = cli_i18n::resolve_language(choice, &cli_i18n::ProcessLocale);
    let cli = match cli_i18n::parse_cli(args, &cli_i18n::ProcessLocale) {
        Ok((cli, _)) => cli,
        Err(error) => cli_i18n::exit_with_cli_error(error, language),
    };
    match cli.command {
        Commands::Run { config } => startup::run(config_path(config)).await,
        Commands::Local { config, command } => handle_local(config_path(config), command).await,
        Commands::StdioWorker {
            config,
            profile,
            supervisor_token,
        } => {
            startup::run_stdio_worker(config, profile.capability_profile(), supervisor_token).await
        }
        Commands::Config { config, command } => {
            config_cli::handle_config(config_path(config), command, language).await
        }
        Commands::Tui { config } => handle_tui(config_path(config), language).await,
        Commands::Tmux { config, command } => handle_tmux(config_path(config), command).await,
    }
}

pub(crate) const MAX_LOCAL_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "agentic-gpt")]
#[command(version)]
#[command(about = "Linux local agent for Agentic GPT")]
pub(crate) struct Cli {
    #[arg(long, global = true, value_enum, default_value_t = LanguageChoice::Auto)]
    pub(crate) language: LanguageChoice,
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
pub(crate) enum LocalCommand {
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
pub(crate) enum TmuxCommand {
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

pub(crate) fn read_local_arguments(
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
            let client = match local_control::LocalProcessClient::connect(&poll_path).await {
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
                match client.list_processes(100).await {
                    Ok(page) => {
                        if sender.send(tui::ProcessUpdate::Processes(page)).is_err() {
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
