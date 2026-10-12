use super::*;

fn confirmation_error(reason: String) -> FileError {
    let code = match reason.as_str() {
        "provider_unavailable" => "file_confirmation_unavailable",
        "policy_denied" => "path_denied",
        _ => "file_confirmation_denied",
    };
    FileError::new(
        code,
        &format!("File operation was not authorized: {reason}"),
    )
}

pub(super) async fn authorize_file_targets(
    state: &AppState,
    config: &Config,
    acquisition: Option<&AcquireConfirmation>,
    operation: &str,
    targets: &[(PathBuf, Access)],
) -> std::result::Result<(Config, crate::confirmation::OperationAuthorization), FileError> {
    if let Some(a) = acquisition {
        a.validate_scope(false)
            .map_err(|r| FileError::new("file_invalid_acquisition", &r))?;
    }
    let mut decision = PolicyDecision::Allow;
    let mut preview = Vec::new();
    let ancestors = targets
        .iter()
        .filter(|(p, _)| !p.exists())
        .filter_map(|(p, _)| {
            p.ancestors()
                .skip(1)
                .find(|a| a.exists())
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();
    let ancestor_identities = ancestors
        .iter()
        .map(|p| fs::metadata(p).ok().map(|m| identity(&m)))
        .collect::<Vec<_>>();
    let identities = targets
        .iter()
        .map(|(p, _)| std::fs::metadata(p).ok().map(|m| identity(&m)))
        .collect::<Vec<_>>();
    for (path, access) in targets {
        decision = decision.max(
            crate::policy::path_policy_decision(config, path, *access == Access::Write)
                .map_err(|r| FileError::new("path_policy_error", &r))?,
        );
        preview.push(format!(
            "{} {}",
            if *access == Access::Write {
                "write"
            } else {
                "read"
            },
            path.display()
        ));
    }
    let authorization = crate::confirmation::authorize_operation(
        state,
        config,
        acquisition,
        false,
        decision,
        operation,
        &preview,
        false,
    )
    .await
    .map_err(confirmation_error)?;
    for ((path, _), before) in targets.iter().zip(identities) {
        if std::fs::metadata(path).ok().map(|m| identity(&m)) != before {
            return Err(FileError::new(
                "file_revision_conflict",
                "target identity changed while awaiting approval",
            ));
        }
    }
    for (path, before) in ancestors.iter().zip(ancestor_identities) {
        if fs::metadata(path).ok().map(|m| identity(&m)) != before {
            return Err(FileError::new(
                "file_revision_conflict",
                "target ancestor changed while awaiting approval",
            ));
        }
    }
    let mut effective = config.clone();
    if authorization.approved {
        if acquisition.is_some_and(|a| a.effective_acquire(false)) {
            effective = exec::permissive_paths(config);
        } else {
            // Policy confirmation approves Confirm paths, not unrelated Deny paths.
            for policy in [
                &mut effective.path_policy.read,
                &mut effective.path_policy.write,
            ] {
                policy.allow.append(&mut policy.confirm);
                if policy.default_action == PolicyDecision::Confirm {
                    policy.default_action = PolicyDecision::Allow;
                }
            }
        }
    }
    Ok((effective, authorization))
}

#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}
#[cfg(not(unix))]
fn identity(metadata: &fs::Metadata) -> (u64, bool) {
    (metadata.len(), metadata.is_dir())
}

pub(super) async fn authorize_read_requests(
    state: &AppState,
    config: &Config,
    requests: &[ReadRequest],
    acquisition: Option<&AcquireConfirmation>,
    recursive: bool,
) -> std::result::Result<Config, FileError> {
    if let Some(a) = acquisition {
        a.validate_scope(false)
            .map_err(|r| FileError::new("file_invalid_acquisition", &r))?;
    }
    let unrestricted = exec::permissive_paths(config);
    let proactive = acquisition.is_some_and(|a| a.effective_acquire(false));
    let mut targets = Vec::new();
    let mut roots = Vec::new();
    for request in requests {
        let resolved = match resolve_path(&unrestricted, &request.path, Access::Read) {
            Ok(p) => p,
            Err(_) => continue,
        };
        roots.push((request.path.clone(), resolved.path.clone()));
        let action = crate::policy::path_policy_decision(config, &resolved.path, false)
            .map_err(|r| FileError::new("path_policy_error", &r))?;
        if !proactive && action == PolicyDecision::Deny {
            continue;
        }
        targets.push((resolved.path.clone(), Access::Read));
        if recursive && !proactive && action == PolicyDecision::Allow {
            let filter_config = config.clone();
            let walker = WalkBuilder::new(&resolved.path)
                .follow_links(false)
                .filter_entry(move |entry| {
                    let Ok(path) = entry.path().canonicalize() else {
                        return false;
                    };
                    !is_reserved_path(&filter_config, &path)
                        && crate::policy::path_policy_decision(&filter_config, &path, false).ok()
                            != Some(PolicyDecision::Deny)
                })
                .build();
            for entry in walker.flatten().take(MAX_SEARCH_FILES + 1) {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    let Ok(path) = entry.path().canonicalize() else {
                        continue;
                    };
                    if crate::policy::path_policy_decision(config, &path, false)
                        .map_err(|r| FileError::new("path_policy_error", &r))?
                        == PolicyDecision::Confirm
                    {
                        targets.push((path, Access::Read));
                    }
                }
            }
        }
    }
    let effective = if targets.is_empty() {
        config.clone()
    } else {
        authorize_file_targets(
            state,
            config,
            acquisition,
            if recursive {
                "file.search (recursive search scope)"
            } else {
                "file.read"
            },
            &targets,
        )
        .await?
        .0
    };
    for (input, path) in roots {
        if resolve_path(&unrestricted, &input, Access::Read)?.path != path {
            return Err(FileError::new(
                "file_symlink_rejected",
                "request target changed while awaiting approval",
            ));
        }
    }
    Ok(effective)
}

pub(crate) async fn read_request_with_images(
    state: &AppState,
    request: &ReadRequest,
    acquisition: Option<AcquireConfirmation>,
) -> std::result::Result<ReadOutput, FileError> {
    let base = state.config.read().await.clone();
    let config = authorize_read_requests(
        state,
        &base,
        std::slice::from_ref(request),
        acquisition.as_ref(),
        false,
    )
    .await?;
    let resolved = resolve_path(&config, &request.path, Access::Read)?;
    read_with_images(
        &resolved,
        request.metadata,
        request.start_line,
        request.end_line,
        MAX_IMAGE_RESPONSE_BYTES,
    )
}

pub(crate) async fn search_request(
    state: &AppState,
    request: &SearchRequest,
    acquisition: Option<AcquireConfirmation>,
) -> Value {
    let batch = search_batch(state, std::slice::from_ref(request), acquisition).await;
    match batch
        .get("results")
        .and_then(Value::as_array)
        .and_then(|r| r.first())
    {
        Some(item) => {
            if let Some(error) = item.get("error") {
                json!({"error":error})
            } else {
                item.get("result").cloned().unwrap_or(batch.clone())
            }
        }
        None => batch,
    }
}
