use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::config::{
    acquire_config_mutation_lock, write_config_with_backup, Config, PathPolicyConfig, Rule,
};
use crate::config_cli::{PathCommand, PathRootCommand, PathRootKind, RuleCommand};
use crate::exec;
use crate::state::CapabilityProfile;
use crate::utils::{command_preview, risk_level, risky_file_mutation};

#[path = "shell_parser.rs"]
pub(crate) mod shell_parser;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
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

pub(crate) fn shell_policy_decision_for_profile(
    config: &Config,
    profile: CapabilityProfile,
    command: &str,
    need_confirm: bool,
) -> PolicyDecision {
    let extraction = shell_parser::extract_literal_commands(command);
    let mut decision = PolicyDecision::Allow;
    let mut complete = extraction.complete;

    for invocation in &extraction.commands {
        let invocation_decision = policy_decision_for_profile(
            config,
            profile,
            &invocation.program,
            &invocation.args,
            false,
        );
        if invocation_decision == PolicyDecision::Deny {
            return PolicyDecision::Deny;
        }
        if !invocation.complete {
            complete = false;
            continue;
        }
        let invocation_decision = if invocation_decision == PolicyDecision::Allow
            && !config
                .policy
                .allow
                .iter()
                .any(|rule| rule.matches(&invocation.program, &invocation.args))
        {
            PolicyDecision::Confirm
        } else {
            invocation_decision
        };
        decision = decision.max(invocation_decision);
    }

    if need_confirm || !complete || extraction.commands.is_empty() {
        decision.max(PolicyDecision::Confirm)
    } else {
        decision
    }
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
    let _lock = acquire_config_mutation_lock(&config_path)?;
    let mut config = Config::load_or_default_locked(&config_path)?;
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
    let _lock = acquire_config_mutation_lock(&config_path)?;
    let mut config = Config::load_or_default_locked(&config_path)?;
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
    match exec::normalize_roots([left, right].into_iter()) {
        Ok(normalized) => normalized.len() == 1,
        Err(_) => false,
    }
}

#[cfg(test)]
mod shell_policy_tests {
    use super::{shell_policy_decision_for_profile, PolicyDecision};
    use crate::{
        config::{Config, Rule},
        state::CapabilityProfile,
    };

    fn config() -> Config {
        Config::default_config().unwrap()
    }

    fn rule(program: &str, args_prefix: &[&str]) -> Rule {
        Rule {
            program: program.to_string(),
            args_prefix: args_prefix.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    #[test]
    fn shell_requires_allow_for_every_literal_command() {
        let mut config = config();
        config.policy.allow.push(rule("printf", &[]));
        assert_eq!(
            shell_policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "printf ok && printf 'two words'",
                false,
            ),
            PolicyDecision::Allow
        );
        assert_eq!(
            shell_policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "printf ok && git status",
                false,
            ),
            PolicyDecision::Confirm
        );
    }

    #[test]
    fn shell_supports_literal_quotes_concatenation_and_supported_operators() {
        let mut config = config();
        config
            .policy
            .allow
            .extend([rule("printf", &[]), rule("cat", &[])]);
        let script = r#"printf 'two words' && printf "escaped \"quote" || cat 'pre'fix | printf end; printf final"#;
        assert_eq!(
            shell_policy_decision_for_profile(&config, CapabilityProfile::Normal, script, false,),
            PolicyDecision::Allow
        );

        config.policy.confirm.push(rule("cat", &["prefix"]));
        assert_eq!(
            shell_policy_decision_for_profile(&config, CapabilityProfile::Normal, script, false,),
            PolicyDecision::Confirm
        );
    }

    #[test]
    fn shell_confirmation_cannot_be_removed_by_an_allow_rule() {
        let mut config = config();
        config.policy.allow.push(rule("printf", &[]));
        assert_eq!(
            shell_policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                "printf ok",
                true,
            ),
            PolicyDecision::Confirm
        );
    }

    #[test]
    fn dynamic_words_redirects_and_complex_scripts_never_use_partial_allows() {
        let mut config = config();
        config
            .policy
            .allow
            .extend([rule("printf", &[]), rule("true", &[])]);
        for script in [
            "",
            r#"printf "$HOME""#,
            "printf ok > output.txt",
            "if true; then printf safe; fi",
        ] {
            assert_eq!(
                shell_policy_decision_for_profile(
                    &config,
                    CapabilityProfile::Normal,
                    script,
                    false,
                ),
                PolicyDecision::Confirm,
                "{script}"
            );
        }
    }

    #[test]
    fn known_deny_wins_inside_unsupported_or_incomplete_scripts() {
        let mut config = config();
        config.policy.allow.push(rule("printf", &[]));
        config.policy.deny.push(rule("touch", &["blocked"]));
        for script in [
            "printf ok; if true; then ssh host; fi",
            r#"touch blocked "$EXTRA"; printf ok"#,
        ] {
            assert_eq!(
                shell_policy_decision_for_profile(
                    &config,
                    CapabilityProfile::Normal,
                    script,
                    false,
                ),
                PolicyDecision::Deny,
                "{script}"
            );
        }
    }
    #[test]
    fn terminal_dollar_in_a_quoted_argument_is_not_truncated() {
        let mut config = config();
        config.policy.allow.push(rule("touch", &["blocked"]));
        config.policy.deny.push(rule("touch", &["blocked$"]));

        assert_eq!(
            shell_policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                r#"touch "blocked$""#,
                false,
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn escaped_crlf_cannot_hide_a_default_denied_command() {
        let mut config = config();
        config.policy.allow.push(rule("printf", &[]));
        let script = concat!("printf safe \\", "\r\nssh host");

        assert_eq!(
            shell_policy_decision_for_profile(&config, CapabilityProfile::Normal, script, false,),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn escaped_word_separator_requires_confirmation_instead_of_partial_matching() {
        let mut config = config();
        config.policy.allow.push(rule("touch", &["blocked"]));
        config.policy.deny.push(rule("touch", &[" blocked"]));

        assert_eq!(
            shell_policy_decision_for_profile(
                &config,
                CapabilityProfile::Normal,
                r"touch \ blocked",
                false,
            ),
            PolicyDecision::Confirm
        );
    }
}
