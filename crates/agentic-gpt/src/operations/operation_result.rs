use agentic_gpt_protocol::{
    McpBatchRequest, McpBatchResponse, McpBatchToolChildResponse, McpBatchToolResponse,
    ProcessBatchResponse, ProcessCancelResponse, ProcessDetail, ProcessError, ProcessInfo,
    ProcessListResponse, ProcessResponse, ProcessResultStatus, ProcessState, ProcessStatusResponse,
    ProcessToolResponse,
};
use anyhow::Result;
use serde_json::Value;

fn elapsed_ms(process: &ProcessInfo) -> u64 {
    process
        .started_at
        .map(|started_at| (chrono::Utc::now() - started_at).num_milliseconds().max(0) as u64)
        .unwrap_or(0)
}

fn duration_ms(process: &ProcessInfo) -> Option<u64> {
    process.started_at.map(|started_at| {
        let finished_at = process.finished_at.unwrap_or(process.updated_at);
        (finished_at - started_at).num_milliseconds().max(0) as u64
    })
}

pub(crate) fn rejection_error(reason: &str) -> ProcessError {
    let code = reason
        .split([':', ';'])
        .next()
        .map(str::trim)
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .unwrap_or("process_rejected")
        .chars()
        .take(64)
        .collect();
    ProcessError {
        code,
        message: reason.to_string(),
    }
}

fn process_rejection_error(process: &ProcessInfo) -> Option<ProcessError> {
    (process.state == ProcessState::Rejected)
        .then(|| process.reject_reason.as_deref().map(rejection_error))
        .flatten()
}

fn attach_process_rejection_error(response: &mut ProcessResponse) {
    if response.error.is_none() {
        response.error = process_rejection_error(&response.process);
    }
}

fn process_error(process: &ProcessInfo, detail: &ProcessDetail) -> Option<ProcessError> {
    detail
        .error
        .clone()
        .or_else(|| process_rejection_error(process))
}

fn process_tool_response(detail: &ProcessDetail, include_identity: bool) -> ProcessToolResponse {
    let process = &detail.process;
    let terminal = process.state.is_terminal();
    ProcessToolResponse {
        process_id: process.process_id.clone(),
        group: include_identity.then(|| process.group.clone()).flatten(),
        kind: include_identity.then_some(process.kind),
        state: process.state,
        elapsed_ms: (!terminal).then(|| elapsed_ms(process)),
        duration_ms: terminal.then(|| duration_ms(process)).flatten(),
        exit_code: terminal.then_some(process.exit_code).flatten(),
        error: terminal.then(|| process_error(process, detail)).flatten(),
        result_status: if terminal && detail.result_available {
            Some(ProcessResultStatus::Complete)
        } else if terminal && process.kind == agentic_gpt_protocol::ProcessKind::Mcp {
            Some(
                if detail
                    .result_bytes
                    .map_or(false, |bytes| bytes > crate::process::MAX_MCP_RESULT_BYTES)
                {
                    ProcessResultStatus::TooLarge
                } else {
                    ProcessResultStatus::Unavailable
                },
            )
        } else {
            None
        },
        result_available: detail.result_available,
        result_bytes: detail.result_bytes,
        result_sha256: detail.result_sha256.clone(),
        result_preview: detail.result_preview.clone(),
    }
}

fn push_process_snapshot(snapshots: Option<&mut Vec<ProcessInfo>>, process: ProcessInfo) {
    if let Some(snapshots) = snapshots {
        snapshots.push(process);
    }
}

pub(crate) fn slim_process_response(
    mut response: ProcessResponse,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    attach_process_rejection_error(&mut response);
    push_process_snapshot(snapshots, response.process.clone());
    if response.process.state == ProcessState::Rejected {
        response.process.reject_reason = None;
    }
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_mcp_response(
    response: ProcessResponse,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    slim_process_response(response, snapshots)
}

pub(crate) fn slim_process_batch_response(
    mut response: ProcessBatchResponse,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    for child in &mut response.processes {
        attach_process_rejection_error(child);
        if let Some(snapshots) = snapshots.as_deref_mut() {
            snapshots.push(child.process.clone());
        }
        if child.process.state == ProcessState::Rejected {
            child.process.reject_reason = None;
        }
    }
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_process_status_response(
    response: ProcessStatusResponse,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    push_process_snapshot(snapshots, response.process.clone());
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_process_list_response(response: ProcessListResponse) -> Result<Value> {
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_process_cancel_response(response: ProcessCancelResponse) -> Result<Value> {
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_mcp_batch_response(
    response: McpBatchResponse,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    let mut slim = McpBatchToolResponse {
        status: response.status,
        error: response.error,
        results: response
            .results
            .into_iter()
            .map(|child| {
                if let Some(snapshots) = snapshots.as_deref_mut() {
                    snapshots.push(child.process.process.clone());
                }
                McpBatchToolChildResponse {
                    process: process_tool_response(&child.process, false),
                }
            })
            .collect(),
    };
    apply_slim_mcp_batch_budget(&mut slim)?;
    Ok(serde_json::to_value(slim)?)
}

fn apply_slim_mcp_batch_budget(response: &mut McpBatchToolResponse) -> Result<()> {
    let limit = McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES;
    let mut bytes = serde_json::to_vec(response)?.len();
    if bytes > limit {
        for index in (0..response.results.len()).rev() {
            if response.results[index]
                .process
                .result_preview
                .take()
                .is_some()
            {
                bytes = serde_json::to_vec(response)?.len();
                if bytes <= limit {
                    break;
                }
            }
        }
    }
    if bytes > limit {
        return Err(anyhow::anyhow!(
            "mcp_batch_response_too_large: bytes={bytes}; max={limit}"
        ));
    }
    Ok(())
}
