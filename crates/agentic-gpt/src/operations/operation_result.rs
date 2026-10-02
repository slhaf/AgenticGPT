use agentic_gpt_protocol::{
    EventPanel, McpBatchToolChildResponse, McpBatchToolResponse, ProcessCancelResponse,
    ProcessCaptureStatus, ProcessDetail, ProcessError, ProcessInfo, ProcessKind,
    ProcessListResponse, ProcessMcpResult, ProcessMcpResultStatus, ProcessResponse, ProcessState,
    MAX_PROCESS_RESPONSE_BYTES, MIN_PROCESS_RESPONSE_BYTES,
};
use anyhow::{anyhow, Result};

use serde_json::Value;

pub(crate) fn attach_event_panel_to_tool_result(
    result: &mut rmcp::model::CallToolResult,
    panel: &EventPanel,
) -> Result<()> {
    {
        let structured = result
            .structured_content
            .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        attach_event_panel(structured, panel)?;
    }

    #[derive(serde::Serialize)]
    struct EventReminder<'a> {
        events: &'a EventPanel,
    }

    let reminder = serde_json::to_string(&EventReminder { events: panel })?;
    let already_present = result.content.iter().any(|content| {
        content
            .as_text()
            .is_some_and(|text| text.text.as_str() == reminder.as_str())
    });
    if !already_present {
        result.content.push(rmcp::model::Content::text(reminder));
    }
    Ok(())
}

pub(crate) fn attach_event_panel(value: &mut Value, panel: &EventPanel) -> Result<()> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("event_panel_result_must_be_object"))?;
    object.insert("events".to_string(), serde_json::to_value(panel)?);
    Ok(())
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

fn bounded_error_code(mut code: String) -> String {
    if code.len() > crate::process::MAX_PROCESS_ERROR_CODE_BYTES {
        let mut end = crate::process::MAX_PROCESS_ERROR_CODE_BYTES;
        while !code.is_char_boundary(end) {
            end -= 1;
        }
        code.truncate(end);
    }
    code
}

fn process_error(process: &ProcessInfo, detail: &ProcessDetail) -> Option<ProcessError> {
    detail
        .error
        .clone()
        .or_else(|| process_rejection_error(process))
        .map(|mut error| {
            error.code = bounded_error_code(error.code);
            error.message = crate::process::bounded_response_text(&error.message);
            error
        })
}

fn mcp_batch_process_response(
    detail: &ProcessDetail,
    result_omitted: bool,
    include_result: bool,
) -> ProcessResponse {
    let process = &detail.process;
    let mcp_result_status = if process.state.is_active() {
        ProcessMcpResultStatus::Pending
    } else if detail
        .result_bytes
        .is_some_and(|bytes| bytes > crate::process::MAX_MCP_RESULT_BYTES)
    {
        ProcessMcpResultStatus::NotRetained
    } else if !detail.detail_available || !detail.result_available || detail.result.is_none() {
        ProcessMcpResultStatus::Unavailable
    } else if result_omitted || !include_result {
        ProcessMcpResultStatus::Deferred
    } else {
        ProcessMcpResultStatus::Included
    };
    let mcp_result = (process.kind == ProcessKind::Mcp).then(|| ProcessMcpResult {
        status: mcp_result_status,
        bytes: detail.result_bytes,
        sha256: detail.result_sha256.clone(),
        value: None,
        preview: (mcp_result_status != ProcessMcpResultStatus::Included)
            .then(|| {
                detail
                    .result_preview
                    .as_deref()
                    .map(crate::process::bounded_response_text)
            })
            .flatten(),
    });
    ProcessResponse {
        agent_id: process.agent_id.clone(),
        process_id: process.process_id.clone(),
        kind: process.kind,
        state: process.state,
        capture_status: process.capture_status,
        group: process.group.clone(),
        batch_id: process.batch_id.clone(),
        batch_index: process.batch_index,
        exit_code: process.exit_code,
        wait_elapsed_ms: None,
        error: process_error(process, detail),
        cancel_outcome: process.cancel_outcome.clone(),
        termination_evidence: process.termination_evidence.clone(),
        capture_error: process
            .capture_error
            .as_deref()
            .map(crate::process::bounded_response_text),
        output: None,
        mcp_result,
    }
}

fn mcp_batch_preflight_child(
    index: usize,
    id: Option<String>,
    agent_id: &str,
    group: Option<&str>,
    batch_id: &str,
    process_id: &str,
) -> McpBatchToolChildResponse {
    // Bound each optional field by its longest controlled response value. These
    // maxima come from different lifecycle branches, so the synthetic child is
    // intentionally more verbose than any one reachable child response.
    McpBatchToolChildResponse {
        index,
        id,
        process: ProcessResponse {
            agent_id: agent_id.to_string(),
            process_id: process_id.to_string(),
            kind: ProcessKind::Mcp,
            state: ProcessState::WaitingConfirmation,
            capture_status: ProcessCaptureStatus::NotApplicable,
            group: group.map(str::to_string),
            batch_id: Some(batch_id.to_string()),
            batch_index: Some(index),
            exit_code: None,
            wait_elapsed_ms: None,
            error: Some(ProcessError {
                code: "mcp_batch_fail_fast_skipped".to_string(),
                message: crate::process::PROCESS_RESPONSE_TRUNCATION_MARKER.to_string(),
            }),
            cancel_outcome: Some("cancelled_before_request".to_string()),
            termination_evidence: Some(
                "mcp_cancel_notification_sent_no_terminal_response".to_string(),
            ),
            capture_error: None,
            output: None,
            mcp_result: Some(ProcessMcpResult {
                status: ProcessMcpResultStatus::NotRetained,
                bytes: Some(usize::MAX),
                sha256: Some(format!("sha256:{}", "x".repeat(64))),
                value: None,
                preview: None,
            }),
        },
    }
}

pub(crate) fn ensure_mcp_batch_response_fits(
    batch_id: &str,
    agent_id: &str,
    group: Option<&str>,
    calls: &[(usize, Option<String>)],
    boot_generation: &str,
    response_budget: usize,
) -> Result<()> {
    if !(MIN_PROCESS_RESPONSE_BYTES..=MAX_PROCESS_RESPONSE_BYTES).contains(&response_budget) {
        return Err(anyhow!("process_response_config_invalid"));
    }

    let process_id = format!("process_{boot_generation}_{}", uuid::Uuid::nil().simple());
    let mut normal_children = Vec::with_capacity(calls.len());
    let mut rejected_children = Vec::with_capacity(calls.len());
    for (index, id) in calls {
        let child =
            mcp_batch_preflight_child(*index, id.clone(), agent_id, group, batch_id, &process_id);
        normal_children.push(child.clone());
        rejected_children.push(child);
    }
    let normal = McpBatchToolResponse {
        batch_id: batch_id.to_string(),
        status: agentic_gpt_protocol::McpBatchStatus::CompletedWithErrors,
        error: None,
        results: normal_children,
    };
    let rejected = McpBatchToolResponse {
        batch_id: batch_id.to_string(),
        status: agentic_gpt_protocol::McpBatchStatus::Rejected,
        error: Some(ProcessError {
            code: "mcp_batch_rejected".to_string(),
            message: crate::process::PROCESS_RESPONSE_TRUNCATION_MARKER.to_string(),
        }),
        results: rejected_children,
    };
    let required_bytes = crate::process::serialized_json_size(&normal)
        .max(crate::process::serialized_json_size(&rejected));
    if required_bytes > response_budget {
        return Err(anyhow!(
            "mcp_batch_response_too_large: bytes={required_bytes}; max={response_budget}"
        ));
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SerializedMcpResult<'a> {
    status: ProcessMcpResultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<&'a str>,
}

fn serialized_value_delta(value: &Value, previous: &ProcessMcpResult) -> isize {
    let previous_bytes = crate::process::serialized_json_size(&SerializedMcpResult {
        status: previous.status,
        bytes: previous.bytes,
        sha256: previous.sha256.as_deref(),
        value: previous.value.as_ref(),
        preview: previous.preview.as_deref(),
    });
    let included_bytes = crate::process::serialized_json_size(&SerializedMcpResult {
        status: ProcessMcpResultStatus::Included,
        bytes: previous.bytes,
        sha256: previous.sha256.as_deref(),
        value: Some(value),
        preview: None,
    });
    included_bytes as isize - previous_bytes as isize
}

fn trim_mcp_batch_descriptions(response: &mut McpBatchToolResponse) {
    if let Some(error) = response.error.as_mut() {
        if !error.message.is_empty() {
            error.message = crate::process::PROCESS_RESPONSE_TRUNCATION_MARKER.to_string();
        }
    }
    for child in &mut response.results {
        if let Some(error) = child.process.error.as_mut() {
            if !error.message.is_empty() {
                error.message = crate::process::PROCESS_RESPONSE_TRUNCATION_MARKER.to_string();
            }
        }
        if let Some(capture_error) = child.process.capture_error.as_mut() {
            if !capture_error.is_empty() {
                *capture_error = crate::process::PROCESS_RESPONSE_TRUNCATION_MARKER.to_string();
            }
        }
        if let Some(result) = child.process.mcp_result.as_mut() {
            result.preview = None;
        }
    }
}
fn apply_serialized_delta(size: usize, delta: isize) -> usize {
    if delta >= 0 {
        size.saturating_add(delta as usize)
    } else {
        size.saturating_sub(delta.unsigned_abs())
    }
}

pub(crate) fn slim_mcp_batch_response(
    batch: crate::mcp::batch::ManagedMcpBatchResponse,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    let response_budget = batch.response_budget;
    let response = batch.response;
    let mut slim = McpBatchToolResponse {
        batch_id: response.batch_id.clone(),
        status: response.status,
        error: response.error.clone().map(|mut error| {
            error.code = bounded_error_code(error.code);
            error.message = crate::process::bounded_response_text(&error.message);
            error
        }),
        results: response
            .results
            .iter()
            .map(|child| McpBatchToolChildResponse {
                index: child.index,
                id: child.id.clone(),
                process: mcp_batch_process_response(&child.process, child.result_omitted, false),
            })
            .collect(),
    };

    let mut base_bytes = crate::process::serialized_json_size(&slim);
    if base_bytes > response_budget {
        trim_mcp_batch_descriptions(&mut slim);
        base_bytes = crate::process::serialized_json_size(&slim);
    }
    if base_bytes > response_budget {
        return Err(anyhow!(
            "mcp_batch_response_too_large: bytes={base_bytes}; max={response_budget}"
        ));
    }

    let deltas = response
        .results
        .iter()
        .enumerate()
        .map(|(index, child)| -> Result<Option<isize>> {
            if child.result_omitted
                || !child.process.detail_available
                || !child.process.result_available
                || child.process.result.is_none()
                || child
                    .process
                    .result_bytes
                    .is_some_and(|bytes| bytes > crate::process::MAX_MCP_RESULT_BYTES)
            {
                return Ok(None);
            }
            let result = slim.results[index]
                .process
                .mcp_result
                .as_ref()
                .ok_or_else(|| anyhow!("mcp_batch_result_projection_missing"))?;
            Ok(Some(serialized_value_delta(
                child.process.result.as_ref().expect("checked above"),
                result,
            )))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut include_results = deltas.iter().map(Option::is_some).collect::<Vec<_>>();
    let mut final_bytes = deltas.iter().flatten().fold(base_bytes, |size, delta| {
        apply_serialized_delta(size, *delta)
    });
    if final_bytes > response_budget {
        for index in (0..deltas.len()).rev() {
            let Some(delta) = deltas[index] else {
                continue;
            };
            if delta <= 0 {
                continue;
            }
            include_results[index] = false;
            final_bytes = apply_serialized_delta(final_bytes, -delta);
            if final_bytes <= response_budget {
                break;
            }
        }
    }
    if final_bytes > response_budget {
        return Err(anyhow!(
            "mcp_batch_response_too_large: bytes={final_bytes}; max={response_budget}"
        ));
    }

    for (index, child) in response.results.into_iter().enumerate() {
        if let Some(snapshots) = snapshots.as_deref_mut() {
            snapshots.push(child.process.process.clone());
        }
        if include_results[index] {
            let value = child
                .process
                .result
                .ok_or_else(|| anyhow!("mcp_batch_retained_result_missing"))?;
            let result = slim.results[index]
                .process
                .mcp_result
                .as_mut()
                .ok_or_else(|| anyhow!("mcp_batch_result_projection_missing"))?;
            result.status = ProcessMcpResultStatus::Included;
            result.value = Some(value);
            result.preview = None;
        }
    }

    let serialized_bytes = crate::process::serialized_json_size(&slim);
    if serialized_bytes > response_budget {
        return Err(anyhow!(
            "mcp_batch_response_too_large: bytes={serialized_bytes}; max={response_budget}"
        ));
    }
    Ok(serde_json::to_value(slim)?)
}

fn push_process_snapshot(snapshots: Option<&mut Vec<ProcessInfo>>, process: ProcessInfo) {
    if let Some(snapshots) = snapshots {
        snapshots.push(process);
    }
}

pub(crate) fn slim_process_response(
    response: crate::process::ManagedProcessResponse,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    push_process_snapshot(snapshots, response.process);
    Ok(serde_json::to_value(response.response)?)
}

pub(crate) fn slim_mcp_response(
    response: crate::process::ManagedProcessResponse,
    snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    slim_process_response(response, snapshots)
}

pub(crate) fn slim_process_batch_response(
    response: crate::process::ManagedProcessBatchResponse,
    mut snapshots: Option<&mut Vec<ProcessInfo>>,
) -> Result<Value> {
    for process in response.processes {
        if let Some(snapshots) = snapshots.as_deref_mut() {
            snapshots.push(process);
        }
    }
    Ok(serde_json::to_value(response.response)?)
}

pub(crate) fn slim_process_list_response(response: ProcessListResponse) -> Result<Value> {
    Ok(serde_json::to_value(response)?)
}

pub(crate) fn slim_process_cancel_response(response: ProcessCancelResponse) -> Result<Value> {
    Ok(serde_json::to_value(response)?)
}
