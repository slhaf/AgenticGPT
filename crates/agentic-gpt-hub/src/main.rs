#[path = "support/agentic_result.rs"]
mod agentic_result;
mod agents;
#[path = "runtime/cli.rs"]
mod cli;
#[path = "runtime/config.rs"]
mod config;
#[path = "runtime/confirmation.rs"]
mod confirmation;
#[path = "storage/db.rs"]
mod db;
#[path = "storage/event_feedback.rs"]
mod event_feedback;
#[path = "runtime/instance_lock.rs"]
mod instance_lock;
#[path = "ingress/mcp/mcp_server.rs"]
mod mcp_server;
#[path = "notifications/notify.rs"]
mod notify;
#[path = "ingress/oauth.rs"]
mod oauth;
#[path = "agents/registry.rs"]
mod registry;
#[path = "room/room.rs"]
mod room;
#[path = "ingress/http/routes.rs"]
mod routes;
#[path = "storage/runs.rs"]
mod runs;
#[path = "runtime/server.rs"]
mod server;
#[path = "runtime/state.rs"]
mod state;
#[path = "support/utils.rs"]
mod utils;

pub(crate) use config::{HubConfig, NtfyConfig, MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS};

use anyhow::Result;
use clap::Parser;

use crate::db::{init_db, open_db};
use crate::registry::handle_agent_command;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentic_gpt_hub=info,tower_http=info,axum=info".into()),
        )
        .init();

    let cli = cli::Cli::parse();
    let db_path = cli.db.unwrap_or_else(config::default_db_path);
    let config_path = cli.config.unwrap_or_else(config::default_config_path);
    let config = HubConfig::load_or_default(&config_path)?;
    match cli.command {
        cli::HubCommandCli::Init => {
            let conn = open_db(&db_path)?;
            init_db(&conn)?;
            event_feedback::init(&conn)?;
            config.write_if_missing(&config_path)?;
            println!("initialized {}", db_path.display());
            println!("config {}", config_path.display());
        }
        cli::HubCommandCli::Serve {
            bind,
            api_key,
            public_base_url,
            mcp_profile,
        } => {
            let _instance_lock =
                instance_lock::InstanceLock::acquire(&db_path, ".serve.lock", "hub")?;
            config.write_if_missing(&config_path)?;
            let conn = open_db(&db_path)?;
            init_db(&conn)?;
            event_feedback::init(&conn)?;
            event_feedback::recover_after_restart(&conn)?;
            server::serve(bind, api_key, public_base_url, mcp_profile, conn, config).await?;
        }
        cli::HubCommandCli::Agent { command } => {
            let conn = open_db(&db_path)?;
            init_db(&conn)?;
            event_feedback::init(&conn)?;
            handle_agent_command(&conn, command)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime/main_tests.rs"]
mod tests;
