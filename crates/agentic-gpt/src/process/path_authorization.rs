use super::*;

pub(crate) fn permissive_paths(config: &Config) -> Config {
    let mut config = config.clone();
    config.policy.path = crate::config::PathPolicyConfig {
        default_action: crate::config::PathDefaultAction::Allow,
        ..Default::default()
    };
    config
}

pub(crate) fn argument_path_decision(
    config: &Config,
    cwd: &Path,
    program: &str,
    args: &[String],
) -> std::result::Result<PolicyDecision, String> {
    let mut decision = PolicyDecision::Allow;
    for arg in args {
        if looks_like_path(arg) {
            let path = resolve_argument_path(cwd, arg, classify_program_access(program))?;
            decision = decision.max(crate::policy::path_policy_decision(
                config,
                &path,
                matches!(classify_program_access(program), PathAccessKind::Write),
            )?);
        }
    }
    Ok(decision)
}

fn literal_commands(execution: &ExecutionSpec) -> Vec<(String, Vec<String>)> {
    match execution {
        ExecutionSpec::Argv { program, args } => vec![(program.clone(), args.clone())],
        ExecutionSpec::Shell { command } => {
            crate::policy::shell_parser::extract_literal_commands(command)
                .commands
                .into_iter()
                .filter(|c| c.complete)
                .map(|c| (c.program, c.args))
                .collect()
        }
    }
}

pub(crate) fn execution_path_decision(
    config: &Config,
    cwd: &Path,
    execution: &ExecutionSpec,
) -> std::result::Result<PolicyDecision, String> {
    let mut decision = crate::policy::path_policy_decision(config, cwd, true)?;
    let permissive = permissive_paths(config);
    for (program, args) in literal_commands(execution) {
        preflight(&permissive, cwd, &program, &args)?;
        decision = decision.max(argument_path_decision(config, cwd, &program, &args)?);
    }
    Ok(decision)
}

fn execution_scopes(
    cwd: &Path,
    execution: &ExecutionSpec,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
) -> std::result::Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let mut read = vec![cwd.to_path_buf()];
    let mut write = Vec::new();
    if let Some(a) = acquisition {
        a.validate_scope(true)?;
        for (input, target) in [(&a.read, &mut read), (&a.write, &mut write)] {
            for raw in input {
                let path = expand_path(raw).map_err(|_| "scope_path_invalid".to_string())?;
                let path = if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                };
                target.push(
                    path.canonicalize()
                        .map_err(|_| "scope_path_not_found".to_string())?,
                );
            }
        }
    }
    for (program, args) in literal_commands(execution) {
        for arg in args {
            if looks_like_path(&arg) {
                let path = resolve_argument_path(cwd, &arg, classify_program_access(&program))?;
                // An absent target never grants its existing parent implicitly.
                if path.exists() {
                    if matches!(classify_program_access(&program), PathAccessKind::Read) {
                        read.push(path)
                    } else {
                        write.push(path)
                    }
                }
            }
        }
    }
    read.sort();
    read.dedup();
    write.sort();
    write.dedup();
    Ok((read, write))
}

pub(crate) fn approval_preview(
    cwd: &Path,
    execution: &ExecutionSpec,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
) -> std::result::Result<Vec<String>, String> {
    let (read, write) = execution_scopes(cwd, execution, acquisition)?;
    Ok(vec![
        match execution {
            ExecutionSpec::Shell { command } => {
                format!("risk: {}", crate::policy::shell_script_risk_level(command))
            }
            ExecutionSpec::Argv { program, .. } => {
                format!("risk: {}", crate::utils::risk_level(program))
            }
        },
        format!("cwd: {}", cwd.display()),
        format!(
            "read: {}",
            read.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "write: {}",
            write
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        match execution {
            ExecutionSpec::Shell { command } => command.clone(),
            ExecutionSpec::Argv { program, args } => crate::utils::command_preview(program, args),
        },
    ])
}

pub(crate) fn granted_execution_config(
    config: &Config,
    cwd: &Path,
    execution: &ExecutionSpec,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
) -> std::result::Result<Config, String> {
    let (read, write) = execution_scopes(cwd, execution, acquisition)?;
    let mut granted = config.clone();
    granted.policy.path.approved_read.extend(read);
    granted.policy.path.approved_write.extend(write);
    Ok(granted)
}

pub(crate) fn declared_scope_decision(
    config: &Config,
    cwd: &Path,
    execution: &ExecutionSpec,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
) -> std::result::Result<PolicyDecision, String> {
    let mut decision = PolicyDecision::Allow;
    if let Some(a) = acquisition {
        a.validate_scope(true)?;
        for (paths, writable) in [(&a.read, false), (&a.write, true)] {
            for raw in paths {
                let path = expand_path(raw).map_err(|_| "scope_path_invalid".to_string())?;
                let path = if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                };
                let path = path
                    .canonicalize()
                    .map_err(|_| "scope_path_not_found".to_string())?;
                decision = decision.max(crate::policy::path_policy_decision(
                    config, &path, writable,
                )?);
            }
        }
    }
    let _ = execution;
    Ok(decision)
}

pub(crate) fn scope_identity(
    cwd: &Path,
    execution: &ExecutionSpec,
    acquisition: Option<&agentic_gpt_protocol::AcquireConfirmation>,
) -> std::result::Result<Vec<(PathBuf, u64, u64)>, String> {
    let (read, write) = execution_scopes(cwd, execution, acquisition)?;
    read.into_iter()
        .chain(write)
        .map(|path| {
            let metadata =
                std::fs::metadata(&path).map_err(|_| "scope_path_not_found".to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                Ok((path, metadata.dev(), metadata.ino()))
            }
            #[cfg(not(unix))]
            {
                Ok((path, metadata.len(), u64::from(metadata.is_dir())))
            }
        })
        .collect()
}
