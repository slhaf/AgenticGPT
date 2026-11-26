#[path = "runtime/agent_info.rs"]
mod agent_info;
#[path = "storage/audit.rs"]
mod audit;
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
#[path = "config/config_api.rs"]
mod config_api;
#[path = "config/config_apply.rs"]
mod config_apply;
#[path = "config/config_catalog.rs"]
mod config_catalog;
#[path = "config/config_cli.rs"]
mod config_cli;
#[path = "runtime/config_runtime.rs"]
mod config_runtime;
#[path = "config/setup/mod.rs"]
mod config_setup;
#[path = "config/config_templates.rs"]
mod config_templates;
#[path = "ui/config_tui/mod.rs"]
mod config_tui;
#[path = "operations/confirmation.rs"]
mod confirmation;
#[path = "room/email/mod.rs"]
mod email;
#[path = "room/email_files.rs"]
mod email_files;
#[path = "room/email_render.rs"]
mod email_render;
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
#[path = "operations/mcp_events.rs"]
mod mcp_events;
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
#[path = "process/model.rs"]
mod process_model;
#[path = "room/secret/mod.rs"]
mod secret;

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

fn main() -> Result<()> {
    // Keep Obscura and V8 on a short-lived, single-threaded worker lifecycle.
    if std::env::args().nth(1).as_deref() == Some("__email-render-worker") {
        return email_render::worker();
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(cli::run())
}

#[cfg(test)]
#[path = "runtime/main_tests.rs"]
mod tests;
