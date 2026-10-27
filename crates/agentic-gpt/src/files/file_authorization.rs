use super::*;

fn confirmation_error(reason: String) -> FileError {
    let code = match reason.as_str() {
        "provider_unavailable" => "file_confirmation_unavailable",
        "policy_denied" => "path_denied",
        "deny" => "file_confirmation_denied",
        "timeout" | "expired" => "file_confirmation_timeout",
        "cancelled" => "file_confirmation_cancelled",
        "callback_token_invalid" => "file_confirmation_callback_invalid",
        _ => "file_confirmation_failed",
    };
    FileError::new(
        code,
        &format!("File operation was not authorized: {reason}"),
    )
}

fn authorization_metadata(path: &Path) -> std::result::Result<Option<fs::Metadata>, FileError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(path_io_error(error, path, false)),
    }
}

pub(super) async fn authorize_file_targets(
    state: &AppState,
    config: &Config,
    acquisition: Option<&AcquireConfirmation>,
    operation: &str,
    targets: &[(PathBuf, Access)],
) -> std::result::Result<(Config, crate::confirmation::OperationAuthorization), FileError> {
    let mut decision = PolicyDecision::Allow;
    let mut preview = Vec::new();
    let identities = targets
        .iter()
        .map(|(path, _)| authorization_metadata(path).map(|m| m.as_ref().map(identity)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut ancestors = Vec::new();
    let mut ancestor_identities = Vec::new();
    for ((path, _), before) in targets.iter().zip(&identities) {
        if before.is_some() {
            continue;
        }
        for ancestor in path.ancestors().skip(1) {
            if let Some(metadata) = authorization_metadata(ancestor)? {
                ancestors.push(ancestor.to_path_buf());
                ancestor_identities.push(identity(&metadata));
                break;
            }
        }
    }
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
        decision,
        operation,
        &preview,
        "文件访问需要确认",
    )
    .await
    .map_err(confirmation_error)?;
    for ((path, _), before) in targets.iter().zip(identities) {
        if authorization_metadata(path)?.as_ref().map(identity) != before {
            return Err(FileError::new(
                "file_path_changed",
                "target identity changed while awaiting approval",
            ));
        }
    }
    for (path, before) in ancestors.iter().zip(ancestor_identities) {
        if authorization_metadata(path)?.as_ref().map(identity) != Some(before) {
            return Err(FileError::new(
                "file_path_changed",
                "target ancestor changed while awaiting approval",
            ));
        }
    }
    let effective = if authorization.approved {
        exec::permissive_paths(config)
    } else {
        config.clone()
    };
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
    paths: &[&str],
    acquisition: Option<&AcquireConfirmation>,
    operation: &str,
) -> std::result::Result<Config, FileError> {
    let unrestricted = exec::permissive_paths(config);
    let proactive = acquisition.is_some();
    let mut targets = Vec::new();
    let mut roots = Vec::new();
    for input in paths {
        let resolved = match resolve_path(&unrestricted, input, Access::Read) {
            Ok(p) => p,
            Err(_) => continue,
        };
        roots.push((*input, resolved.path.clone()));
        let action = crate::policy::path_policy_decision(config, &resolved.path, false)
            .map_err(|r| FileError::new("path_policy_error", &r))?;
        if !proactive && action == PolicyDecision::Deny {
            continue;
        }
        targets.push((resolved.path.clone(), Access::Read));
    }
    let effective = if targets.is_empty() {
        config.clone()
    } else {
        authorize_file_targets(state, config, acquisition, operation, &targets)
            .await?
            .0
    };
    for (input, path) in roots {
        if resolve_path(&unrestricted, input, Access::Read)?.path != path {
            return Err(FileError::new(
                "file_path_changed",
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
    let paths = [request.path.as_str()];
    let config =
        authorize_read_requests(state, &base, &paths, acquisition.as_ref(), "file.read").await?;
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
    match super::search_results(state, std::slice::from_ref(request), acquisition)
        .await
        .pop()
        .expect("single file.search request produces one result")
    {
        Ok(output) => output.into_slim_value(),
        Err(error) => error.value(),
    }
}
