use agentic_gpt_protocol::{
    JobBatchResponse, JobCancelResponse, JobDetail, JobError, JobInfo, JobKind, JobListItem,
    JobListResponse, JobResponse, JobState, JobToolResponse, JobWaitResponse, McpBatchResponse,
    McpBatchToolChildResponse, McpBatchToolResponse,
};
use anyhow::Result;
use serde_json::Value;

fn elapsed_ms(info: &JobInfo) -> u64 {
    info.started_at
        .map(|started_at| (chrono::Utc::now() - started_at).num_milliseconds().max(0) as u64)
        .unwrap_or(0)
}

fn duration_ms(info: &JobInfo) -> Option<u64> {
    info.started_at.map(|started_at| {
        let finished_at = info.finished_at.unwrap_or(info.updated_at);
        (finished_at - started_at).num_milliseconds().max(0) as u64
    })
}

pub(crate) fn rejection_error(reason: &str) -> JobError {
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
        .unwrap_or("job_rejected")
        .chars()
        .take(64)
        .collect();
    JobError {
        code,
        message: reason.to_string(),
    }
}

fn process_error(info: &JobInfo, detail: &JobDetail) -> Option<JobError> {
    detail.error.clone().or_else(|| {
        (info.state == JobState::Rejected)
            .then(|| info.reject_reason.as_deref().map(rejection_error))
            .flatten()
    })
}

fn job_tool_response(
    detail: &JobDetail,
    include_identity: bool,
    aggregate_result_omitted: bool,
) -> JobToolResponse {
    let info = &detail.job;
    let process_like = matches!(info.kind, JobKind::Process | JobKind::Skill);
    let terminal = info.state.is_terminal();
    let mut response = JobToolResponse {
        job_id: info.job_id.clone(),
        group: include_identity.then(|| info.group.clone()).flatten(),
        kind: include_identity.then_some(info.kind),
        state: info.state,
        elapsed_ms: (!terminal).then(|| elapsed_ms(info)),
        duration_ms: terminal.then(|| duration_ms(info)).flatten(),
        exit_code: (terminal && process_like)
            .then_some(info.exit_code)
            .flatten()
            .filter(|code| *code != 0),
        stdout_tail: if process_like {
            info.stdout_tail.clone()
        } else {
            String::new()
        },
        stderr_tail: if process_like {
            info.stderr_tail.clone()
        } else {
            String::new()
        },
        truncated: (terminal || !info.state.is_terminal()) && process_like && info.truncated,
        result: (!process_like && terminal)
            .then(|| detail.result.clone())
            .flatten(),
        error: if terminal {
            if process_like {
                process_error(info, detail)
            } else {
                detail.error.clone()
            }
        } else {
            None
        },
        result_truncated: (!process_like && terminal) && detail.result_truncated,
        result_bytes: (!process_like && terminal && detail.result_truncated)
            .then_some(detail.result_bytes)
            .flatten(),
        result_sha256: (!process_like && terminal && detail.result_truncated)
            .then(|| detail.result_sha256.clone())
            .flatten(),
        result_preview: (!process_like && terminal && detail.result_truncated)
            .then(|| detail.result_preview.clone())
            .flatten(),
        result_omitted: (!process_like && terminal) && aggregate_result_omitted,
    };
    if !process_like {
        response.stdout_tail.clear();
        response.stderr_tail.clear();
        response.truncated = false;
    }
    response
}

fn slim_job_detail_value(detail: &JobDetail, include_identity: bool) -> Result<Value> {
    Ok(serde_json::to_value(job_tool_response(
        detail,
        include_identity,
        false,
    ))?)
}

fn push_job_snapshot(snapshots: Option<&mut Vec<JobInfo>>, job: JobInfo) {
    if let Some(snapshots) = snapshots {
        snapshots.push(job);
    }
}

pub(crate) fn slim_job_get_response(
    detail: JobDetail,
    wait_only: bool,
    wait_seconds: u64,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    if wait_only && wait_seconds > 0 && !detail.job.state.is_terminal() {
        let value = serde_json::to_value(JobWaitResponse {
            job_id: detail.job.job_id.clone(),
            state: detail.job.state,
            elapsed_ms: elapsed_ms(&detail.job),
        })?;
        push_job_snapshot(snapshots, detail.job);
        return Ok(value);
    }
    let value = slim_job_detail_value(&detail, true)?;
    push_job_snapshot(snapshots, detail.job);
    Ok(value)
}

pub(crate) fn slim_process_response(
    response: JobResponse,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    slim_job_response(response, false, snapshots)
}

pub(crate) fn slim_mcp_response(
    response: JobResponse,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    slim_job_response(response, false, snapshots)
}

fn slim_job_response(
    response: JobResponse,
    include_identity: bool,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    let value = slim_job_detail_value(&response.detail, include_identity)?;
    push_job_snapshot(snapshots, response.detail.job);
    Ok(value)
}

pub(crate) fn slim_process_batch_response(
    response: JobBatchResponse,
    mut snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    let jobs = response
        .jobs
        .into_iter()
        .map(|job| {
            let detail = JobDetail {
                job,
                detail_available: true,
                result: None,
                error: None,
                result_truncated: false,
                result_bytes: None,
                result_sha256: None,
                result_preview: None,
            };
            let response = job_tool_response(&detail, false, false);
            if let Some(snapshots) = snapshots.as_deref_mut() {
                snapshots.push(detail.job);
            }
            response
        })
        .collect();
    Ok(serde_json::to_value(
        agentic_gpt_protocol::JobBatchToolResponse {
            batch_id: response.batch_id,
            status: response.status,
            jobs,
        },
    )?)
}

pub(crate) fn slim_mcp_batch_response(
    response: McpBatchResponse,
    mut snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    let mut slim = McpBatchToolResponse {
        status: response.status,
        error: response.error,
        results: response
            .results
            .into_iter()
            .map(|child| {
                let result_omitted = child.result_omitted;
                let job = job_tool_response(&child.detail, false, result_omitted);
                if let Some(snapshots) = snapshots.as_deref_mut() {
                    snapshots.push(child.detail.job);
                }
                McpBatchToolChildResponse { job }
            })
            .collect(),
    };
    apply_slim_mcp_batch_budget(&mut slim)?;
    Ok(serde_json::to_value(slim)?)
}

pub(crate) fn slim_job_list_response(page: crate::job_history::JobHistoryPage) -> Result<Value> {
    let jobs = page
        .jobs
        .into_iter()
        .map(|job| JobListItem {
            job_id: job.job_id,
            group: job.group,
            kind: job.kind,
            state: job.state,
            created_at: job.created_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        })
        .collect();
    Ok(serde_json::to_value(JobListResponse {
        jobs,
        next_cursor: page.next_cursor,
    })?)
}

pub(crate) fn slim_cancel_response(
    detail: JobDetail,
    snapshots: Option<&mut Vec<JobInfo>>,
) -> Result<Value> {
    let cancel_outcome = detail
        .job
        .cancel_outcome
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let error = if matches!(
        cancel_outcome.as_str(),
        "cancel_failed" | "notification_failed" | "notification_timeout"
    ) {
        detail.error.or_else(|| {
            Some(JobError {
                code: cancel_outcome.clone(),
                message: format!(
                    "Cancellation did not complete; termination evidence: {}",
                    detail
                        .job
                        .termination_evidence
                        .as_deref()
                        .unwrap_or("unknown")
                ),
            })
        })
    } else {
        None
    };
    let value = serde_json::to_value(JobCancelResponse {
        job_id: detail.job.job_id.clone(),
        state: detail.job.state,
        cancel_outcome,
        termination_evidence: detail
            .job
            .termination_evidence
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        error,
    })?;
    push_job_snapshot(snapshots, detail.job);
    Ok(value)
}

fn apply_slim_mcp_batch_budget(response: &mut McpBatchToolResponse) -> Result<()> {
    let limit = agentic_gpt_protocol::McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES;
    let mut bytes = serde_json::to_vec(response)?.len();
    if bytes > limit {
        for index in (0..response.results.len()).rev() {
            if response.results[index].job.result.take().is_some() {
                response.results[index].job.result_omitted = true;
                bytes = serde_json::to_vec(response)?.len();
                if bytes <= limit {
                    break;
                }
            }
        }
    }
    if bytes > limit {
        return Err(anyhow::anyhow!(
            "mcp_batch_result_too_large_after_clipping: bytes={bytes}; max={limit}"
        ));
    }
    Ok(())
}
