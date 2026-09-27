use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::registry;
use crate::state::McpProfile;

#[derive(Parser)]
#[command(name = "agentic-gpt-hub")]
#[command(version)]
#[command(about = "VPS Hub for Agentic GPT")]
pub(crate) struct Cli {
    #[arg(long, env = "AGENTIC_GPT_HUB_DB")]
    pub(crate) db: Option<PathBuf>,
    #[arg(long, env = "AGENTIC_GPT_HUB_CONFIG")]
    pub(crate) config: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: HubCommandCli,
}

#[derive(Subcommand)]
pub(crate) enum HubCommandCli {
    Init,
    Serve {
        #[arg(long, env = "AGENTIC_GPT_HUB_BIND", default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
        #[arg(long, env = "AGENTIC_GPT_API_KEY")]
        api_key: String,
        #[arg(long, env = "AGENTIC_GPT_PUBLIC_BASE_URL")]
        public_base_url: Option<String>,
        #[arg(
            long,
            env = "AGENTIC_GPT_HUB_MCP_PROFILE",
            value_enum,
            default_value_t = McpProfile::Full
        )]
        mcp_profile: McpProfile,
    },
    Agent {
        #[command(subcommand)]
        command: registry::AgentCommand,
    },
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn cli_version_uses_crate_version() {
        let error = match Cli::try_parse_from(["agentic-gpt-hub", "--version"]) {
            Ok(_) => panic!("--version unexpectedly parsed as a runnable command"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
        let rendered = error.to_string();
        assert!(rendered.contains("agentic-gpt-hub 0.9.1"));
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
    }
}
