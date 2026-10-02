#[path = "runtime/agent_info.rs"]
mod agent_info;
#[path = "storage/audit.rs"]
mod audit;
#[path = "room/bootstrap.rs"]
mod bootstrap;
#[path = "browser/browser_discovery.rs"]
mod browser_discovery;
#[path = "browser/browser_distribution.rs"]
mod browser_distribution;
#[path = "browser/browser_kernel.rs"]
mod browser_kernel;
#[path = "browser/browser_manager.rs"]
mod browser_manager;
#[path = "browser/browser_manual.rs"]
mod browser_manual;
#[path = "browser/browser_runtime.rs"]
mod browser_runtime;
#[path = "ui/cli.rs"]
mod cli;
#[path = "ui/cli_i18n.rs"]
mod cli_i18n;
#[path = "config/config.rs"]
mod config;
#[path = "config/config_cli.rs"]
mod config_cli;
#[path = "config/setup/mod.rs"]
mod config_setup;
#[path = "config/config_templates.rs"]
mod config_templates;
#[path = "ui/config_tui/mod.rs"]
mod config_tui;
#[path = "operations/confirmation.rs"]
mod confirmation;
#[path = "operations/event_notifications.rs"]
mod event_notifications;
#[path = "storage/event_store.rs"]
mod event_store;
#[path = "process/exec.rs"]
mod exec;
#[path = "files/file_ops.rs"]
mod file_ops;
#[path = "ingress/http_oauth.rs"]
mod http_oauth;
#[path = "ingress/http_server.rs"]
mod http_server;
#[path = "ingress/hub.rs"]
mod hub;
#[path = "runtime/instance_lock.rs"]
mod instance_lock;
#[path = "ingress/local_control.rs"]
mod local_control;
#[path = "operations/local_service.rs"]
mod local_service;
#[path = "mcp/mcp.rs"]
mod mcp;
#[path = "runtime/notify.rs"]
mod notify;
#[path = "operations/operation.rs"]
mod operation;
#[path = "operations/operation_result.rs"]
mod operation_result;
#[path = "operations/policy.rs"]
mod policy;
#[path = "storage/private_state.rs"]
mod private_state;
#[path = "process/managed.rs"]
mod process;
#[path = "storage/process_history.rs"]
mod process_history;
#[path = "room/room_maintenance.rs"]
mod room_maintenance;
#[path = "room/room_reads.rs"]
mod room_reads;
#[path = "room/room_repository.rs"]
mod room_repository;

#[path = "skills/skill_installs.rs"]
mod skill_installs;
#[path = "skills/skills.rs"]
mod skills;
#[path = "runtime/startup.rs"]
mod startup;
#[path = "runtime/state.rs"]
mod state;
#[path = "ingress/stdio_server.rs"]
mod stdio_server;
#[path = "runtime/supervisor.rs"]
mod supervisor;
#[path = "tmux/tmux.rs"]
mod tmux;
#[path = "storage/transport_ledger.rs"]
mod transport_ledger;
#[path = "ui/tui/mod.rs"]
mod tui;
#[path = "runtime/tunnel_distribution.rs"]
mod tunnel_distribution;
#[path = "support/utils.rs"]
mod utils;

use anyhow::Result;

pub(crate) use config::WorkerProfile;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    cli::run().await
}

#[cfg(test)]
#[path = "runtime/main_tests.rs"]
mod tests;
