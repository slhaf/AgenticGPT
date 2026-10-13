use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::config::{acquire_config_mutation_lock, write_config_with_backup, Config, PolicyConfig};
use crate::config_cli::{PathCommand, PathRootCommand, RuleCommand};
use crate::utils::{risk_level, risky_file_mutation};

#[path = "policy_patterns.rs"]
mod patterns;
#[path = "shell_parser.rs"]
pub(crate) mod shell_parser;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
}

impl PolicyDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Confirm => "confirm",
            Self::Deny => "deny",
        }
    }
}

pub(crate) fn validate_command_pattern(pattern: &str) -> Result<()> {
    patterns::command_tokens(pattern)
        .map(|_| ())
        .map_err(|error| anyhow!(error))
}

fn decide(policy: &PolicyConfig, matches: impl Fn(&str) -> bool) -> PolicyDecision {
    for (action, patterns) in [
        (PolicyDecision::Deny, &policy.deny),
        (PolicyDecision::Confirm, &policy.confirm),
        (PolicyDecision::Allow, &policy.allow),
    ] {
        if patterns.iter().any(|pattern| matches(pattern)) {
            return action;
        }
    }
    policy.default_action
}

pub(crate) fn command_policy_decision(
    config: &Config,
    program: &str,
    args: &[String],
) -> PolicyDecision {
    if !config.policy.enabled {
        return PolicyDecision::Allow;
    }
    decide(&config.policy.command, |pattern| {
        patterns::command_matches(pattern, program, args)
    })
}

pub(crate) fn shell_policy_decision(config: &Config, command: &str) -> PolicyDecision {
    if !config.policy.enabled {
        return PolicyDecision::Allow;
    }
    let extraction = shell_parser::extract_literal_commands(command);
    // The sole wildcard applies even when no literal invocation can be recovered.
    let global = decide(&config.policy.command, |pattern| pattern.trim() == "*");
    if config
        .policy
        .command
        .deny
        .iter()
        .any(|pattern| pattern.trim() == "*")
    {
        return PolicyDecision::Deny;
    }
    let mut decision = PolicyDecision::Allow;
    for invocation in &extraction.commands {
        let invocation_decision =
            command_policy_decision(config, &invocation.program, &invocation.args);
        if invocation_decision == PolicyDecision::Deny {
            return PolicyDecision::Deny;
        }
        decision = decision.max(invocation_decision);
    }
    if !extraction.complete
        || extraction.commands.is_empty()
        || extraction
            .commands
            .iter()
            .any(|invocation| !invocation.complete)
    {
        let explicitly_global = config
            .policy
            .command
            .allow
            .iter()
            .any(|pattern| pattern.trim() == "*");
        decision = decision.max(if explicitly_global {
            global
        } else {
            global.max(PolicyDecision::Confirm)
        });
    }
    decision
}

pub(crate) fn path_policy_decision(
    config: &Config,
    path: &Path,
    write: bool,
) -> std::result::Result<PolicyDecision, String> {
    if !config.policy.enabled {
        return Ok(PolicyDecision::Allow);
    }
    let policy = &config.policy.path;
    let in_roots = |roots: &[PathBuf]| -> std::result::Result<bool, String> {
        Ok(
            crate::exec::normalize_roots(roots.iter().map(PathBuf::as_path))
                .map_err(|e| e.to_string())?
                .iter()
                .any(|root| path.starts_with(root)),
        )
    };
    if in_roots(&policy.approved_write)? || (!write && in_roots(&policy.approved_read)?) {
        return Ok(PolicyDecision::Allow);
    }
    if in_roots(&policy.deny)? {
        return Ok(PolicyDecision::Deny);
    }
    if in_roots(&policy.write)? {
        return Ok(PolicyDecision::Allow);
    }
    if in_roots(&policy.read_only)? {
        return Ok(if write {
            PolicyDecision::Deny
        } else {
            PolicyDecision::Allow
        });
    }
    Ok(match policy.default_action {
        crate::config::PathDefaultAction::Allow => PolicyDecision::Allow,
        crate::config::PathDefaultAction::Deny => PolicyDecision::Deny,
    })
}

pub(crate) fn shell_script_risk_level(command: &str) -> &'static str {
    let extraction = shell_parser::extract_literal_commands(command);
    if !extraction.complete
        || extraction
            .commands
            .iter()
            .any(|invocation| risky_file_mutation(&invocation.program))
    {
        "HIGH"
    } else if extraction
        .commands
        .iter()
        .any(|invocation| risk_level(&invocation.program) != "LOW")
    {
        "MEDIUM"
    } else {
        "LOW"
    }
}

pub(crate) fn mutate_rule(
    config_path: PathBuf,
    decision: PolicyDecision,
    command: RuleCommand,
) -> Result<()> {
    let _lock = acquire_config_mutation_lock(&config_path)?;
    let mut config = Config::load_or_default_locked(&config_path)?;
    mutate_patterns(&mut config.policy.command, decision, command)?;
    write_config_with_backup(&config_path, &config)
}

fn mutate_patterns(
    policy: &mut PolicyConfig,
    decision: PolicyDecision,
    command: RuleCommand,
) -> Result<()> {
    let rules = match decision {
        PolicyDecision::Allow => &mut policy.allow,
        PolicyDecision::Confirm => &mut policy.confirm,
        PolicyDecision::Deny => &mut policy.deny,
    };
    match command {
        RuleCommand::Add { pattern } => {
            validate_command_pattern(&pattern)?;
            if !rules.contains(&pattern) {
                rules.push(pattern);
            }
        }
        RuleCommand::Remove { pattern } => {
            if !rules.contains(&pattern) {
                return Err(anyhow!("pattern not found: {pattern}"));
            }
            rules.retain(|existing| existing != &pattern);
        }
    }
    Ok(())
}

pub(crate) fn mutate_path_policy(config_path: PathBuf, command: PathCommand) -> Result<()> {
    let _lock = acquire_config_mutation_lock(&config_path)?;
    let mut config = Config::load_or_default_locked(&config_path)?;
    let policy = &mut config.policy.path;
    let (roots, command) = match command {
        PathCommand::List => {
            println!("{}", serde_json::to_string_pretty(policy)?);
            return Ok(());
        }
        PathCommand::DefaultAction { action } => {
            policy.default_action = action;
            return write_config_with_backup(&config_path, &config);
        }
        PathCommand::Readonly { command } => (&mut policy.read_only, command),
        PathCommand::Write { command } => (&mut policy.write, command),
        PathCommand::Deny { command } => (&mut policy.deny, command),
    };
    match command {
        PathRootCommand::Add { path } => {
            crate::exec::normalize_roots(std::iter::once(path.as_path()))?;
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
        PathRootCommand::Remove { path } => {
            roots.retain(|root| root != &path);
        }
    }
    write_config_with_backup(&config_path, &config)
}
