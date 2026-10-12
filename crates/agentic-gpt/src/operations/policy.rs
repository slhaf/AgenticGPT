use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::config::{acquire_config_mutation_lock, write_config_with_backup, Config, PolicyConfig};
use crate::config_cli::{PathCommand, PathPolicyCommand, RuleCommand};
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

pub(crate) fn validate_path_pattern(pattern: &str) -> Result<()> {
    patterns::path_pattern(pattern)
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
    decide(&config.policy, |pattern| {
        patterns::command_matches(pattern, program, args)
    })
}

pub(crate) fn shell_policy_decision(config: &Config, command: &str) -> PolicyDecision {
    let extraction = shell_parser::extract_literal_commands(command);
    // The sole wildcard applies even when no literal invocation can be recovered.
    let global = decide(&config.policy, |pattern| pattern.trim() == "*");
    if config
        .policy
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
    let policy = if write {
        &config.path_policy.write
    } else {
        &config.path_policy.read
    };
    let mut normalized = Vec::new();
    for (action, patterns) in [
        (PolicyDecision::Deny, &policy.deny),
        (PolicyDecision::Confirm, &policy.confirm),
        (PolicyDecision::Allow, &policy.allow),
    ] {
        for pattern in patterns {
            normalized.push((action, patterns::path_pattern(pattern)?));
        }
    }
    let target = path
        .to_str()
        .ok_or_else(|| "path_policy_target_not_utf8".to_string())?;
    for (action, pattern) in normalized {
        if patterns::path_matches(&pattern, target) {
            return Ok(action);
        }
    }
    Ok(policy.default_action)
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
    mutate_patterns(&mut config.policy, decision, command, false)?;
    write_config_with_backup(&config_path, &config)
}

fn mutate_patterns(
    policy: &mut PolicyConfig,
    decision: PolicyDecision,
    command: RuleCommand,
    path: bool,
) -> Result<()> {
    let rules = match decision {
        PolicyDecision::Allow => &mut policy.allow,
        PolicyDecision::Confirm => &mut policy.confirm,
        PolicyDecision::Deny => &mut policy.deny,
    };
    match command {
        RuleCommand::Add { pattern } => {
            if path {
                validate_path_pattern(&pattern)?;
            } else {
                validate_command_pattern(&pattern)?;
            }
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
    let (policy, command) = match command {
        PathCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.path_policy)?);
            return Ok(());
        }
        PathCommand::Read { command } => (&mut config.path_policy.read, command),
        PathCommand::Write { command } => (&mut config.path_policy.write, command),
    };
    match command {
        PathPolicyCommand::DefaultAction { action } => policy.default_action = action,
        PathPolicyCommand::Allow { command } => {
            mutate_patterns(policy, PolicyDecision::Allow, command, true)?
        }
        PathPolicyCommand::Confirm { command } => {
            mutate_patterns(policy, PolicyDecision::Confirm, command, true)?
        }
        PathPolicyCommand::Deny { command } => {
            mutate_patterns(policy, PolicyDecision::Deny, command, true)?
        }
    }
    write_config_with_backup(&config_path, &config)
}
