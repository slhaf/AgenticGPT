use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::config::{write_config_with_backup, Config, PathPolicyConfig, Rule};
use crate::exec;
use crate::state::{CapabilityProfile, RunMode};
use crate::utils::command_preview;
use crate::{PathCommand, PathRootCommand, PathRootKind, RuleCommand};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
}
pub(crate) fn policy_decision(
    config: &Config,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    policy_decision_for_mode(config, RunMode::Normal, program, args, need_confirm)
}

pub(crate) fn policy_decision_for_mode(
    config: &Config,
    run_mode: RunMode,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    policy_decision_for_profile(config, run_mode.profile(), program, args, need_confirm)
}

pub(crate) fn policy_decision_for_profile(
    config: &Config,
    profile: CapabilityProfile,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    let mut decision = if need_confirm {
        PolicyDecision::Confirm
    } else {
        PolicyDecision::Allow
    };
    for rule in builtin_rules(profile, PolicyDecision::Confirm) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Confirm);
        }
    }
    for rule in builtin_rules(profile, PolicyDecision::Deny) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Deny);
        }
    }

    let mut configured_decision = None;
    for rule in &config.policy.allow {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Allow);
        }
    }
    for rule in &config.policy.confirm {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Confirm);
        }
    }
    for rule in &config.policy.deny {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Deny);
        }
    }

    configured_decision.unwrap_or(decision)
}

impl Rule {
    pub(crate) fn matches(&self, program: &str, args: &[String]) -> bool {
        self.program == program
            && args.len() >= self.args_prefix.len()
            && self
                .args_prefix
                .iter()
                .zip(args.iter())
                .all(|(expected, actual)| expected == actual)
    }
}

pub(crate) fn builtin_rules(profile: CapabilityProfile, decision: PolicyDecision) -> Vec<Rule> {
    let programs = match decision {
        PolicyDecision::Deny => vec!["su", "mkfs", "dd", "ssh"],
        PolicyDecision::Confirm if profile == CapabilityProfile::Room => {
            vec!["sudo", "mount", "systemctl", "service", "scp"]
        }
        PolicyDecision::Confirm => vec![
            "sudo",
            "rm",
            "mv",
            "chmod",
            "chown",
            "mount",
            "systemctl",
            "service",
            "docker",
            "scp",
            "curl",
            "wget",
            "bash",
            "sh",
            "zsh",
            "fish",
            "perl",
            "ruby",
        ],
        PolicyDecision::Allow => vec![],
    };
    let mut rules = programs
        .into_iter()
        .map(|program| Rule {
            program: program.to_string(),
            args_prefix: vec![],
        })
        .collect::<Vec<_>>();
    if decision == PolicyDecision::Confirm && profile == CapabilityProfile::Normal {
        rules.push(Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        });
        rules.push(Rule {
            program: "node".to_string(),
            args_prefix: vec!["-e".to_string()],
        });
    }
    rules
}
pub(crate) fn mutate_rule(
    config_path: PathBuf,
    decision: PolicyDecision,
    command: RuleCommand,
) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    let rules = match decision {
        PolicyDecision::Allow => &mut config.policy.allow,
        PolicyDecision::Confirm => &mut config.policy.confirm,
        PolicyDecision::Deny => &mut config.policy.deny,
    };
    match command {
        RuleCommand::Add {
            program,
            args_prefix,
        } => {
            let rule = Rule {
                program,
                args_prefix,
            };
            println!("added {}", rule_display(&rule));
            rules.push(rule);
        }
        RuleCommand::Remove {
            program,
            args_prefix,
        } => {
            remove_rule(rules, &program, &args_prefix)?;
        }
    }
    write_config_with_backup(&config_path, &config)
}

pub(crate) fn remove_rule(
    rules: &mut Vec<Rule>,
    program: &str,
    args_prefix: &[String],
) -> Result<()> {
    remove_rule_with_interactive(rules, program, args_prefix, io::stdin().is_terminal())
}

pub(crate) fn remove_rule_with_interactive(
    rules: &mut Vec<Rule>,
    program: &str,
    args_prefix: &[String],
    interactive: bool,
) -> Result<()> {
    let matches = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.program == program && rule.args_prefix == args_prefix)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(anyhow!(
            "rule not found: {}",
            command_preview(program, args_prefix)
        )),
        1 => {
            let removed = rules.remove(matches[0]);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ if interactive => {
            let selected = choose_rule_interactively(rules, &matches)?;
            let removed = rules.remove(selected);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ => {
            eprintln!(
                "multiple rules match {}; rerun in an interactive terminal or provide a more specific args prefix:",
                command_preview(program, args_prefix)
            );
            for index in matches {
                eprintln!("  {}", rule_display(&rules[index]));
            }
            Err(anyhow!("multiple_matching_rules"))
        }
    }
}

fn choose_rule_interactively(rules: &[Rule], matches: &[usize]) -> Result<usize> {
    println!("multiple matching rules:");
    for (ordinal, index) in matches.iter().enumerate() {
        println!("  {}) {}", ordinal + 1, rule_display(&rules[*index]));
    }
    print!("select rule to remove [1-{}]: ", matches.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selected = input
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid selection"))?;
    if selected == 0 || selected > matches.len() {
        return Err(anyhow!("selection out of range"));
    }
    Ok(matches[selected - 1])
}

fn rule_display(rule: &Rule) -> String {
    command_preview(&rule.program, &rule.args_prefix)
}

pub(crate) fn mutate_path_policy(config_path: PathBuf, command: PathCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    match command {
        PathCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.path_policy)?);
            return Ok(());
        }
        PathCommand::Write { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Write, command)
        }
        PathCommand::Readonly { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Readonly, command)
        }
        PathCommand::Deny { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Deny, command)
        }
    }
    write_config_with_backup(&config_path, &config)
}

pub(crate) fn mutate_path_roots(
    policy: &mut PathPolicyConfig,
    kind: PathRootKind,
    command: PathRootCommand,
) {
    match command {
        PathRootCommand::Add { path } => {
            let roots = roots_for_kind(policy, kind);
            if !roots.iter().any(|existing| paths_match(existing, &path)) {
                roots.push(path);
            }
        }
        PathRootCommand::Remove { path } => {
            let roots = roots_for_kind(policy, kind);
            roots.retain(|existing| !paths_match(existing, &path));
        }
    }
}

fn roots_for_kind(policy: &mut PathPolicyConfig, kind: PathRootKind) -> &mut Vec<PathBuf> {
    match kind {
        PathRootKind::Write => &mut policy.write_roots,
        PathRootKind::Readonly => &mut policy.read_only_roots,
        PathRootKind::Deny => &mut policy.deny_roots,
    }
}

pub(crate) fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (
        exec::expand_pathbuf(left).and_then(|path| exec::canonicalize_existing_or_parent(&path)),
        exec::expand_pathbuf(right).and_then(|path| exec::canonicalize_existing_or_parent(&path)),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
