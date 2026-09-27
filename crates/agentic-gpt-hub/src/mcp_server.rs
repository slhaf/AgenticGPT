use agentic_gpt_protocol::{
    normalize_job_group, BatchExecRequest, BootstrapReadRequest, ExecElement, ExecRequest,
    HubCommand, JobCancelRequest, JobGetRequest, JobKind, JobListRequest, JobState, McpBatchCall,
    McpBatchMode, McpBatchRequest, McpCallToolRequest, McpListToolsRequest, NotificationAction,
    RoomDiaryActiveRequest, RoomDiaryLayer, RoomDiaryReadRequest, RoomMaintenanceExecutionMode,
    RoomMaintenanceRequestItem, RoomMaintenanceSlot, RoomMaintenanceStatusRequest,
    RoomMaintenanceSubmitRequest, RoomNotebookReadRequest, RoomNotebookRecentRequest,
    RoomNotebookSearchRequest, RoomStateListRequest, RoomStateReadRequest, SkillActivationRequest,
    SkillInstallCancelRequest, SkillInstallFile, SkillInstallGetRequest, SkillInstallRequest,
    SkillInstallSource, SkillReadRequest, SkillRunRequest, SkillSearchRequest,
    TmuxCapturePaneRequest, TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest,
    TmuxListPanesRequest, TmuxPasteTextRequest, UserNotifySendRequest,
};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ErrorData, Meta, ServerCapabilities, ServerInfo, ToolAnnotations,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::agentic_result::AgenticResult;
use crate::agents::dispatch::{cached_job, mcp_list_servers_all_agents, request_agent};
use crate::notify::{notification_channels, send_user_notification, NotifyRouteError};
use crate::registry::{registry_entries, registry_entry};
use crate::room::{request_active_room, RoomRouteError};
use crate::runs;
use crate::state::{
    projection::{
        add_cache_metadata, build_hub_info_response, filter_cached_jobs, job_list_item,
        live_job_value,
    },
    HubState, McpProfile,
};
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;
const ROOM_TRANSPORT_MARGIN_SECS: u64 = 5;

const MCP_INSTRUCTIONS: &str = "Agentic GPT Hub exposes domain-specific job creation plus one generic lifecycle. Use process.exec for one managed process and process.batch for multiple managed processes; both wait briefly and return Job envelopes. Use mcp.callTool for one downstream MCP Job and mcp.batch for 1..16 atomically admitted child Jobs with one aggregate confirmation, ordered results, global/per-server concurrency bounds, and optional fail-fast scheduling. Use job.get with waitSeconds to inspect or briefly wait, job.list for bounded filtered discovery, and job.cancel for kind-aware cancellation evidence. Use tmux as the persistent shared workspace for stateful development, iterative debugging, TUIs, and user-agent handoff. For tmux work, discover the workspace with tmux.listSessions and tmux.listPanes, inspect it with tmux.capturePane, then use tmux.exec for shell panes or tmux.pasteText for non-shell panes. At Room start, call room.bootstrap, then room.bootstrap.read for relevant guides. Room skills are managed only by the active Room Agent; skills.run returns the same Job envelope and is followed through job.get/job.cancel. Commands remain subject to Agentic local policy, path policy, confirmation, capacity, and audit.";
const COORDINATOR_INSTRUCTIONS: &str = "Agentic GPT Hub coordinator profile. This connector exposes only Hub-native agent status, retained run history, current job snapshots, and notification tools. It never dispatches execution, job-control, tmux, downstream MCP, skills, bootstrap, diary, or notebook commands to an Agent.";
fn default_job_wait_seconds() -> u64 {
    JobGetRequest::DEFAULT_WAIT_SECONDS
}

fn job_get_payload(params: &JobGetArgs) -> JobGetRequest {
    let mut payload = JobGetRequest {
        job_id: params.job_id.clone(),
        wait_only: params.wait_only.unwrap_or(false),
        wait_seconds: params.wait_seconds,
    };
    payload.wait_seconds = Some(payload.effective_wait_seconds());
    payload
}

fn default_standard_wait_seconds() -> u64 {
    5
}
fn default_wait_only() -> bool {
    false
}
fn default_room_wait_seconds() -> u8 {
    0
}
fn default_room_notebook_limit() -> usize {
    20
}
fn default_job_list_limit() -> usize {
    50
}
const COORDINATOR_TOOLS: &[&str] = &[
    "hub.info",
    "agent.list",
    "hub.run.list",
    "hub.run.get",
    "hub.job.list",
    "hub.job.get",
    "user.notify.channels",
    "user.notify.send",
];

#[derive(Clone)]
pub(crate) struct AgenticMcpServer {
    state: HubState,
    profile: McpProfile,
    tool_router: ToolRouter<Self>,
}

impl AgenticMcpServer {
    pub(crate) fn new(state: HubState) -> Self {
        let mut tool_router = Self::tool_router();
        decorate_tool_descriptors(&mut tool_router);
        let profile = state.mcp_profile;
        Self {
            state,
            profile,
            tool_router,
        }
    }

    fn profile_allows_tool(&self, name: &str) -> bool {
        self.profile == McpProfile::Full || COORDINATOR_TOOLS.contains(&name)
    }

    fn generated_tool(&self, name: &str) -> bool {
        self.tool_router.get(name).is_some()
    }

    // Profile filtering narrows the generated router; annotations only describe callable tools.
    fn allows_tool(&self, name: &str) -> bool {
        self.profile_allows_tool(name) && self.generated_tool(name)
    }

    fn instructions(&self) -> &'static str {
        match self.profile {
            McpProfile::Full => MCP_INSTRUCTIONS,
            McpProfile::Coordinator => COORDINATOR_INSTRUCTIONS,
        }
    }
}

fn decorate_tool_descriptors(tool_router: &mut ToolRouter<AgenticMcpServer>) {
    for route in tool_router.map.values_mut() {
        let name = route.attr.name.as_ref();
        let open_world = matches!(
            name,
            "process.exec"
                | "process.batch"
                | "mcp.batch"
                | "mcp.callTool"
                | "tmux.pasteText"
                | "tmux.exec"
                | "tmux.createSession"
                | "tmux.closeSession"
                | "skills.install"
                | "skills.run"
        );
        let read_only = tool_is_read_only(name);
        let destructive = matches!(
            name,
            "job.cancel"
                | "tmux.closeSession"
                | "room.maintenance.submit"
                | "skills.install"
                | "skills.install.cancel"
                | "skills.run"
        );
        route.attr.annotations = Some(
            ToolAnnotations::new()
                .read_only(read_only)
                .destructive(destructive)
                .open_world(open_world),
        );
        route.attr.output_schema = Some(std::sync::Arc::new(object_schema()));
        let mut meta = Map::new();
        meta.insert(
            "securitySchemes".to_string(),
            json!([{ "type": "oauth2", "scopes": ["agentic:mcp"] }]),
        );
        route.attr.meta = Some(Meta(meta));
    }
}

fn tool_is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "process.exec"
            | "process.batch"
            | "job.cancel"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.batch"
            | "mcp.callTool"
            | "user.notify.send"
            | "room.maintenance.submit"
            | "skills.activate"
            | "skills.deactivate"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.run"
    )
}

fn parse_job_kind(value: Option<&str>) -> Result<Option<JobKind>, ErrorData> {
    match value {
        None => Ok(None),
        Some("process") => Ok(Some(JobKind::Process)),
        Some("skill") => Ok(Some(JobKind::Skill)),
        Some("mcp") => Ok(Some(JobKind::Mcp)),
        Some(_) => Err(mcp_invalid_params(
            "job_kind_invalid",
            "kind must be process, skill, or mcp",
        )),
    }
}

fn parse_job_state(value: Option<&str>) -> Result<Option<JobState>, ErrorData> {
    let state = match value {
        None => return Ok(None),
        Some("queued") => JobState::Queued,
        Some("waiting_confirmation") => JobState::WaitingConfirmation,
        Some("starting") => JobState::Starting,
        Some("running") => JobState::Running,
        Some("completed") => JobState::Completed,
        Some("failed") => JobState::Failed,
        Some("rejected") => JobState::Rejected,
        Some("cancel_requested") => JobState::CancelRequested,
        Some("cancelled") => JobState::Cancelled,
        Some("timed_out") => JobState::TimedOut,
        Some("detached") => JobState::Detached,
        Some("unknown_after_restart") => JobState::UnknownAfterRestart,
        Some("skipped") => JobState::Skipped,
        Some(_) => return Err(mcp_invalid_params("job_state_invalid", "unknown Job state")),
    };
    Ok(Some(state))
}

fn object_schema() -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("additionalProperties".to_string(), Value::Bool(true));
    schema
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

pub(crate) async fn mcp_get(State(state): State<HubState>) -> Response {
    let server = AgenticMcpServer::new(state);
    Json(json!({
        "name": "agentic-gpt-hub",
        "profile": server.profile.label(),
        "tools": app_tool_descriptors(&server)
    }))
    .into_response()
}

pub(crate) async fn mcp_post(State(state): State<HubState>, Json(rpc): Json<Value>) -> Response {
    let request = match serde_json::from_value::<JsonRpcRequest>(rpc) {
        Ok(request) => request,
        Err(error) => return rpc_error(None, -32700, format!("Invalid JSON-RPC request: {error}")),
    };
    let id = request.id.clone();
    let server = AgenticMcpServer::new(state);
    match request.method.as_str() {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": negotiated_protocol_version(request.params.as_ref()),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "agentic-gpt-hub",
                    "title": "Agentic GPT Hub",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": server.instructions(),
                "profile": server.profile.label()
            }),
        ),
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": app_tool_descriptors(&server) })),
        "tools/call" => {
            match call_app_tool(&server, request.params.unwrap_or_else(|| json!({}))).await {
                Ok(result) => rpc_result(id, result),
                Err(error) => rpc_error(id, -32602, error),
            }
        }
        _ => rpc_error(id, -32601, "Method not found"),
    }
}

async fn call_app_tool(server: &AgenticMcpServer, params: Value) -> Result<Value, String> {
    let object = params
        .as_object()
        .ok_or_else(|| "tools/call params must be an object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call params.name is required".to_string())?;
    if !server.allows_tool(name) {
        if !server.profile_allows_tool(name) {
            return Err(format!("tool_unavailable_for_profile: {name}"));
        }
        return Err(format!("Unknown tool: {name}"));
    }
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name {
        "agent.list" => server.list_agents().await,
        "process.exec" => server.exec(Parameters(decode_args(arguments)?)).await,
        "process.batch" => server.batch_exec(Parameters(decode_args(arguments)?)).await,
        "job.list" => server.job_list(Parameters(decode_args(arguments)?)).await,
        "job.get" => server.job_get(Parameters(decode_args(arguments)?)).await,
        "job.cancel" => server.job_cancel(Parameters(decode_args(arguments)?)).await,
        "tmux.listSessions" => {
            server
                .tmux_list_sessions(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.listPanes" => {
            server
                .tmux_list_panes(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.capturePane" => {
            server
                .tmux_capture_pane(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.pasteText" => {
            server
                .tmux_paste_text(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.exec" => server.tmux_exec(Parameters(decode_args(arguments)?)).await,
        "tmux.createSession" => {
            server
                .tmux_create_session(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.closeSession" => {
            server
                .tmux_close_session(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.listServers" => {
            server
                .mcp_list_servers(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.listTools" => {
            server
                .mcp_list_tools(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.callTool" => {
            server
                .mcp_call_tool(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.batch" => server.mcp_batch(Parameters(decode_args(arguments)?)).await,
        "hub.info" => server.hub_info().await,
        "user.notify.channels" => server.user_notify_channels().await,
        "hub.run.get" => {
            server
                .hub_run_get(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.run.list" => {
            server
                .hub_run_list(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.job.list" => {
            server
                .hub_job_list(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.job.get" => {
            server
                .hub_job_get(Parameters(decode_args(arguments)?))
                .await
        }
        "user.notify.send" => {
            server
                .user_notify_send(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.active" => {
            server
                .room_diary_active(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.read" => {
            server
                .room_diary_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.recent" => {
            server
                .room_notebook_recent(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.search" => {
            server
                .room_notebook_search(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.read" => {
            server
                .room_notebook_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.state.list" => {
            server
                .room_state_list(Parameters(decode_args(arguments)?))
                .await
        }
        "room.state.read" => {
            server
                .room_state_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.maintenance.status" => {
            server
                .room_maintenance_status(Parameters(decode_args(arguments)?))
                .await
        }
        "room.maintenance.submit" => {
            server
                .room_maintenance_submit(Parameters(decode_args(arguments)?))
                .await
        }
        "room.bootstrap" => server.room_bootstrap().await,
        "room.bootstrap.read" => {
            server
                .room_bootstrap_read(Parameters(decode_args(arguments)?))
                .await
        }
        "bootstrap" => server.bootstrap().await,
        "bootstrap.read" => {
            server
                .bootstrap_read(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.list" => server.skills_list().await,
        "skills.read" => {
            server
                .skills_read(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.search" => {
            server
                .skills_search(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.active" => server.skills_active().await,
        "skills.activate" => {
            server
                .skills_activate(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.deactivate" => {
            server
                .skills_deactivate(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install" => {
            server
                .skills_install(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install.get" => {
            server
                .skills_install_get(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install.cancel" => {
            server
                .skills_install_cancel(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.run" => server.skills_run(Parameters(decode_args(arguments)?)).await,
        _ => return Err(format!("Unknown tool: {name}")),
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_value(result).map_err(|error| error.to_string())
}

fn decode_args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid tool arguments: {error}"))
}

async fn snapshot_job_list(state: &HubState, agent_id: &str) -> Value {
    let snapshots = state.job_cache.snapshots(agent_id).await;
    let mut value = json!({
        "jobs": snapshots
            .iter()
            .cloned()
            .map(|snapshot| snapshot.job)
            .collect::<Vec<_>>()
    });
    add_cache_metadata(&mut value, &snapshots);
    value
}

async fn snapshot_job_list_filtered(
    state: &HubState,
    agent_id: &str,
    request: &JobListRequest,
    unavailable_reason: &str,
) -> Value {
    if request.cursor.is_some() {
        return json!({
            "error": {
                "code": "job_list_cursor_unavailable",
                "message": format!(
                    "Agent is unavailable and Hub cache cannot continue an Agent-issued cursor: {unavailable_reason}"
                )
            },
            "freshness": "unknown"
        });
    }
    let mut snapshots = state.job_cache.snapshots(agent_id).await;
    filter_cached_jobs(&mut snapshots, request);
    let mut value = json!({
        "jobs": snapshots
            .iter()
            .cloned()
            .map(|snapshot| job_list_item(snapshot.job))
            .collect::<Vec<_>>()
    });
    add_cache_metadata(&mut value, &snapshots);
    value
}

async fn cached_job_summary(state: &HubState, agent_id: &str, job_id: &str) -> Option<Value> {
    cached_job(state, agent_id, job_id)
        .await
        .and_then(|snapshot| {
            let mut value = serde_json::to_value(job_list_item(snapshot.job.clone())).ok()?;
            add_cache_metadata(&mut value, std::slice::from_ref(&snapshot));
            Some(value)
        })
}

async fn snapshot_job_get(state: &HubState, agent_id: &str, job_id: &str) -> Value {
    match cached_job(state, agent_id, job_id).await {
        Some(snapshot) => {
            let mut value = json!({
                "job": snapshot.job.clone(),
                "detailAvailable": false,
                "resultTruncated": false
            });
            add_cache_metadata(&mut value, std::slice::from_ref(&snapshot));
            value
        }
        None => {
            json!({ "error": { "code": "job_not_found", "message": "Job was not found" }, "freshness": "unknown" })
        }
    }
}

fn app_tool_descriptors(server: &AgenticMcpServer) -> Vec<Value> {
    server
        .tool_router
        .list_all()
        .into_iter()
        .filter(|tool| server.allows_tool(tool.name.as_ref()))
        .map(|tool| {
            let mut value = serde_json::to_value(tool).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                let security_schemes = json!([{ "type": "oauth2", "scopes": ["agentic:mcp"] }]);
                object.insert("securitySchemes".to_string(), security_schemes.clone());
                object
                    .entry("_meta".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(meta) = object.get_mut("_meta").and_then(Value::as_object_mut) {
                    meta.insert("securitySchemes".to_string(), security_schemes);
                    meta.insert(
                        "openai/toolInvocation/invoking".to_string(),
                        json!("Running…"),
                    );
                    meta.insert("openai/toolInvocation/invoked".to_string(), json!("Done"));
                }
            }
            value
        })
        .collect()
}

fn negotiated_protocol_version(params: Option<&Value>) -> &'static str {
    match params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
    {
        Some("2025-03-26") => "2025-03-26",
        Some("2025-06-18") => "2025-06-18",
        _ => "2025-06-18",
    }
}

fn rpc_result(id: Option<Value>, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result }))
        .into_response()
}

fn rpc_error(id: Option<Value>, code: i64, message: impl ToString) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message.to_string() }
    }))
    .into_response()
}

pub(crate) async fn require_auth_on_mcp_path(
    State(state): State<HubState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/mcp")
        && !crate::oauth::is_valid_mcp_bearer(&state, &headers).await
    {
        return crate::oauth::mcp_unauthorized_response(&state, &headers);
    }
    next.run(request).await
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgenticMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "agentic-gpt-hub",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }
}

#[tool_router(router = tool_router)]
impl AgenticMcpServer {
    #[tool(
        name = "hub.info",
        description = "Inspect Hub runtime health and bounded capacity summaries; read-only."
    )]
    async fn hub_info(&self) -> Result<CallToolResult, ErrorData> {
        let info = build_hub_info_response(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(serde_json::to_value(info).map_err(|error| {
            mcp_internal_error("serialization_error", error.to_string())
        })?))
    }

    #[tool(
        name = "agent.list",
        description = "List registered local agents and availability; read-only discovery."
    )]
    async fn list_agents(&self) -> Result<CallToolResult, ErrorData> {
        let entries = registry_entries(&self.state)
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        let agents = self
            .state
            .agents
            .list_agents(&entries)
            .await
            .into_iter()
            .map(|entry| {
                json!({
                    "agentId": entry.agent_id,
                    "alias": entry.alias,
                    "displayName": entry.display_name,
                    "online": entry.online,
                    "connectionMode": entry.connection_mode.map(|mode| mode.label()),
                    "lastSeenAt": entry.last_seen_at,
                    "capabilities": entry.capabilities,
                    "configSummary": entry.config_summary,
                })
            })
            .collect::<Vec<_>>();
        Ok(ok_json(json!({ "agents": agents })))
    }

    #[tool(
        name = "hub.run.get",
        description = "Read one retained Hub command run by id."
    )]
    async fn hub_run_get(
        &self,
        params: Parameters<HubRunGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        match runs::get_run(&self.state, &params.run_id)
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?
        {
            Some(run) => Ok(ok_json(serde_json::to_value(run).map_err(|error| {
                mcp_internal_error("serialization_error", error.to_string())
            })?)),
            None => Err(mcp_invalid_params("run_not_found", "Run was not found")),
        }
    }

    #[tool(
        name = "hub.run.list",
        description = "List retained Hub and Agent-originated run records; read-only."
    )]
    async fn hub_run_list(
        &self,
        params: Parameters<HubRunListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let limit = params.limit.unwrap_or(20).clamp(1, 100);
        let since = params
            .since_seconds
            .map(|seconds| chrono::Utc::now() - chrono::Duration::seconds(seconds as i64));
        let runs = runs::list_runs(
            &self.state,
            params.agent_id.as_deref(),
            params.source.as_deref(),
            params.status.as_deref(),
            since,
            limit,
        )
        .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "runs": runs, "limit": limit })))
    }

    #[tool(
        name = "hub.job.list",
        description = "List current or cached Job snapshots without dispatching to an Agent."
    )]
    async fn hub_job_list(
        &self,
        params: Parameters<AgentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent_id = params.0.agent_id;
        self.ensure_agent_enabled(&agent_id)?;
        Ok(ok_json(snapshot_job_list(&self.state, &agent_id).await))
    }

    #[tool(
        name = "hub.job.get",
        description = "Get one current or cached Job snapshot without dispatching to an Agent."
    )]
    async fn hub_job_get(
        &self,
        params: Parameters<JobIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        Ok(result_from_value(
            snapshot_job_get(&self.state, &params.agent_id, &params.job_id).await,
        ))
    }

    #[tool(
        name = "process.exec",
        description = "Start one managed process on a local Agent; use job tools for lifecycle follow-up."
    )]
    async fn exec(&self, params: Parameters<ExecArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ExecRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            program: params.program,
            args: params.args.unwrap_or_default(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
            wait_seconds: params.wait_seconds,
        };
        let command = HubCommand::Exec {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(
            |reason| json!({ "error": { "code": "process_exec_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.batch",
        description = "Start multiple managed processes under one admission boundary; started side effects are not rolled back."
    )]
    async fn batch_exec(
        &self,
        params: Parameters<BatchExecArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = BatchExecRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            elements: params
                .elements
                .into_iter()
                .map(|element| ExecElement {
                    program: element.program,
                    args: element.args.unwrap_or_default(),
                    working_directory: element.working_directory,
                })
                .collect(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
            wait_seconds: params.wait_seconds,
        };
        let command = HubCommand::ProcessBatch {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(
            |reason| json!({ "error": { "code": "process_batch_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "job.list",
        description = "List active or retained Jobs for one local Agent; read-only discovery."
    )]
    async fn job_list(&self, params: Parameters<JobListArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = JobListRequest {
            group: parse_job_group(params.group)?,
            kind: parse_job_kind(params.kind.as_deref())?,
            state: parse_job_state(params.state.as_deref())?,
            limit: params.limit,
            cursor: params.cursor,
        };
        let command = HubCommand::JobList {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 2).await {
            Ok(value) => live_job_value(value),
            Err(reason) => {
                snapshot_job_list_filtered(&self.state, &params.agent_id, &payload, &reason).await
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "job.get",
        description = "Inspect or briefly wait for one Job; cached fallback is not proof of a fresh live result."
    )]
    async fn job_get(&self, params: Parameters<JobGetArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = job_get_payload(&params);
        let wait_seconds = payload.effective_wait_seconds();
        let command = HubCommand::JobGet {
            request_id: random_id("req"),
            payload,
        };
        let value =
            match request_agent(&self.state, &params.agent_id, command, wait_seconds + 2).await {
                Ok(value) => live_job_value(value),
                Err(reason) => {
                    let cached =
                        cached_job_summary(&self.state, &params.agent_id, &params.job_id).await;
                    let mut value = json!({
                        "error": {
                            "code": "job_get_unavailable",
                            "message": reason
                        },
                        "freshness": "unknown"
                    });
                    if let Some(cached) = cached {
                        if let Some(freshness) = cached.get("freshness").cloned() {
                            value["freshness"] = freshness;
                        }
                        if let Some(observed_at) = cached.get("observedAt").cloned() {
                            value["observedAt"] = observed_at;
                        }
                        value["cached"] = cached;
                    }
                    value
                }
            };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "job.cancel",
        description = "Request Job cancellation and return observed termination evidence; remote stop is not assumed."
    )]
    async fn job_cancel(&self, params: Parameters<JobIdArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::JobCancel {
            request_id: random_id("req"),
            payload: JobCancelRequest {
                job_id: params.job_id.clone(),
            },
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 5).await {
            Ok(value) => live_job_value(value),
            Err(reason) => {
                let cached = snapshot_job_get(&self.state, &params.agent_id, &params.job_id).await;
                let freshness = cached
                    .get("freshness")
                    .cloned()
                    .unwrap_or_else(|| json!("unknown"));
                let observed_at = cached.get("observedAt").cloned();
                let mut value = json!({
                    "error": {
                        "code": "job_cancel_unavailable",
                        "message": reason
                    },
                    "cached": cached,
                    "freshness": freshness
                });
                if let Some(observed_at) = observed_at {
                    value["observedAt"] = observed_at;
                }
                value
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.listSessions",
        description = "List persistent tmux sessions on a local Agent; read-only."
    )]
    async fn tmux_list_sessions(
        &self,
        params: Parameters<TmuxListSessionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxListSessions {
            request_id: random_id("req"),
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.listPanes",
        description = "List tmux panes and shell/TUI hints; read-only."
    )]
    async fn tmux_list_panes(
        &self,
        params: Parameters<TmuxListPanesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxListPanes {
            request_id: random_id("req"),
            payload: TmuxListPanesRequest {
                session: params.session,
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.capturePane",
        description = "Capture bounded tmux pane history; use it to observe tmux.exec progress."
    )]
    async fn tmux_capture_pane(
        &self,
        params: Parameters<TmuxCapturePaneArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCapturePane {
            request_id: random_id("req"),
            payload: TmuxCapturePaneRequest {
                target: params.target,
                lines: params.lines.unwrap_or(160),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.pasteText",
        description = "Paste text into a non-shell tmux pane or TUI; shell panes are rejected."
    )]
    async fn tmux_paste_text(
        &self,
        params: Parameters<TmuxPasteTextArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxPasteText {
            request_id: random_id("req"),
            payload: TmuxPasteTextRequest {
                target: params.target,
                text: params.text,
                submit: params.submit.unwrap_or(false),
                need_confirm: params.need_confirm.unwrap_or(true),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.exec",
        description = "Submit one structured command to a tmux shell pane; returned snapshot is not proof of completion."
    )]
    async fn tmux_exec(
        &self,
        params: Parameters<TmuxExecArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxExec {
            request_id: random_id("req"),
            payload: TmuxExecRequest {
                target: params.target,
                program: params.program,
                args: params.args,
                need_confirm: params.need_confirm.unwrap_or(false),
                wait_ms: params.wait_ms.unwrap_or(300),
                capture_lines: params.capture_lines.unwrap_or(120),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.createSession",
        description = "Create or reuse one persistent tmux session in an allowed working directory."
    )]
    async fn tmux_create_session(
        &self,
        params: Parameters<TmuxCreateSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCreateSession {
            request_id: random_id("req"),
            payload: TmuxCreateSessionRequest {
                name: params.name,
                cwd: params.cwd,
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.closeSession",
        description = "Close one persistent tmux session; destructive and confirmation-aware."
    )]
    async fn tmux_close_session(
        &self,
        params: Parameters<TmuxCloseSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCloseSession {
            request_id: random_id("req"),
            payload: TmuxCloseSessionRequest {
                name: params.name,
                need_confirm: params.need_confirm.unwrap_or(true),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.listServers",
        description = "List downstream MCP servers, optionally scoped to one Agent; read-only discovery."
    )]
    async fn mcp_list_servers(
        &self,
        params: Parameters<McpListServersArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(agent_id) = params.0.agent_id {
            self.ensure_agent_enabled(&agent_id)?;
            let command = HubCommand::McpListServers {
                request_id: random_id("req"),
            };
            let value = request_agent(&self.state, &agent_id, command, REQUEST_TIMEOUT_SECS)
                .await
                .unwrap_or_else(|reason| json!({ "error": { "code": "mcp_list_servers_timeout", "message": reason } }));
            return Ok(result_from_value(value));
        }

        let value = mcp_list_servers_all_agents(&self.state)
            .await
            .unwrap_or_else(|reason| json!({ "error": { "code": "db_error", "message": reason } }));
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.listTools",
        description = "List tools exposed by one downstream MCP server; read-only discovery."
    )]
    async fn mcp_list_tools(
        &self,
        params: Parameters<McpListToolsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpListToolsRequest {
            agent_id: params.agent_id.clone(),
            server_id: params.server_id,
        };
        let command = HubCommand::McpListTools {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(
            |reason| json!({ "error": { "code": "mcp_list_tools_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.callTool",
        description = "Run one downstream MCP tool as a managed Job; use job tools for lifecycle follow-up."
    )]
    async fn mcp_call_tool(
        &self,
        params: Parameters<McpCallToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpCallToolRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            server_id: params.server_id,
            tool_name: params.tool_name,
            arguments: params.arguments.unwrap_or_else(|| json!({})),
            wait_seconds: params.wait_seconds,
            timeout_seconds: params.timeout_seconds,
        };
        let command = HubCommand::McpCallTool {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let request_timeout = payload.effective_wait_seconds() + 2;
        let value = request_agent(&self.state, &payload.agent_id, command, request_timeout)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "mcp_call_tool_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.batch",
        description = "Run multiple downstream MCP calls as managed Jobs under one admission boundary; downstream side effects are not rolled back."
    )]
    async fn mcp_batch(
        &self,
        params: Parameters<McpBatchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpBatchRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            calls: params
                .calls
                .into_iter()
                .map(|call| McpBatchCall {
                    id: None,
                    server_id: call.server_id,
                    tool_name: call.tool_name,
                    arguments: call.arguments.unwrap_or_else(|| json!({})),
                })
                .collect(),
            mode: params.mode.unwrap_or_default().into(),
            fail_fast: params.fail_fast.unwrap_or(false),
            wait_seconds: params.wait_seconds,
            timeout_seconds: params.timeout_seconds,
        };
        let command = HubCommand::McpBatch {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let request_timeout = payload.effective_wait_seconds() + 2;
        let value = request_agent(&self.state, &payload.agent_id, command, request_timeout)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "mcp_batch_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "user.notify.channels",
        description = "List Hub-native notification channels; read-only."
    )]
    async fn user_notify_channels(&self) -> Result<CallToolResult, ErrorData> {
        let channels = notification_channels(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "channels": channels })))
    }

    #[tool(
        name = "user.notify.send",
        description = "Send one Hub-native user notification."
    )]
    async fn user_notify_send(
        &self,
        params: Parameters<UserNotifySendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let request = UserNotifySendRequest {
            channel_key: params.channel,
            title: params.title,
            body: params.body,
            actions: params
                .actions
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            priority: params.priority,
        };
        let value = send_user_notification(&self.state, request)
            .await
            .map(serde_json::to_value)
            .unwrap_or_else(|error| {
                Ok(json!({
                    "error": {
                        "code": notify_route_error_code(&error),
                        "message": notify_route_error_message(&error)
                    }
                }))
            })
            .unwrap_or_else(|error| json!({ "error": { "code": "serialization_error", "message": error.to_string() } }));
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.active",
        description = "Read the active daily, weekly, and monthly Room diary documents; read-only, semantic, and bounded."
    )]
    async fn room_diary_active(
        &self,
        _params: Parameters<RoomDiaryActiveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryActive {
                request_id: random_id("req"),
                payload: RoomDiaryActiveRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_diary_active_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.read",
        description = "Read one exact Room diary document by validated semantic layer and period; read-only, semantic, and bounded."
    )]
    async fn room_diary_read(
        &self,
        params: Parameters<RoomDiaryReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = RoomDiaryReadRequest {
            layer: params.layer.into(),
            period: params.period,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_diary_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.recent",
        description = "Read bounded recent Room notebook Markdown previews; read-only, semantic, and bounded semantic discovery."
    )]
    async fn room_notebook_recent(
        &self,
        params: Parameters<RoomNotebookRecentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomNotebookRecentRequest {
            limit: params.0.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRecent {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_recent_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.search",
        description = "Search Room notebook Markdown by bounded case-insensitive substring fields; read-only, semantic, and bounded discovery."
    )]
    async fn room_notebook_search(
        &self,
        params: Parameters<RoomNotebookSearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = RoomNotebookSearchRequest {
            query: params.query,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookSearch {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_search_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.read",
        description = "Read one exact Room notebook Markdown document under the validated Notebook root; read-only, semantic, and bounded."
    )]
    async fn room_notebook_read(
        &self,
        params: Parameters<RoomNotebookReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomNotebookReadRequest {
            path: params.0.path,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.state.list",
        description = "List deterministic Room state entity documents; read-only, semantic, and bounded."
    )]
    async fn room_state_list(
        &self,
        _params: Parameters<RoomStateListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomStateList {
                request_id: random_id("req"),
                payload: RoomStateListRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_state_list_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.state.read",
        description = "Read one exact Room state entity Markdown document by validated entity name; read-only, semantic, and bounded."
    )]
    async fn room_state_read(
        &self,
        params: Parameters<RoomStateReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomStateReadRequest {
            entity: params.0.entity,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomStateRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_state_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.maintenance.status",
        description = "Inspect Room maintenance readiness, repository state, schema/scaffold support, executor configuration, workflow/remote availability, synchronization heads, and deterministic occupancy for all five semantic slots; read-only and non-destructive."
    )]
    async fn room_maintenance_status(
        &self,
        _params: Parameters<RoomMaintenanceStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomMaintenanceStatus {
                request_id: random_id("req"),
                payload: RoomMaintenanceStatusRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_maintenance_status_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.maintenance.submit",
        description = "Apply one to five unique Room maintenance slot requests after exact validation; destructive but confined to the validated Room repository, with optional local/workflow mode and bounded workflow wait; not open-world."
    )]
    async fn room_maintenance_submit(
        &self,
        params: Parameters<RoomMaintenanceSubmitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = params.0.into_protocol();
        let timeout_secs = REQUEST_TIMEOUT_SECS
            .max(u64::from(payload.effective_wait_seconds()) + ROOM_TRANSPORT_MARGIN_SECS);
        let value = request_active_room(
            &self.state,
            HubCommand::RoomMaintenanceSubmit {
                request_id: random_id("req"),
                payload,
            },
            timeout_secs,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_maintenance_submit_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.bootstrap",
        description = "Load Room bootstrap guidance; read-only."
    )]
    async fn room_bootstrap(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomBootstrap {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_bootstrap_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.bootstrap.read",
        description = "Read one validated Room bootstrap guide by id."
    )]
    async fn room_bootstrap_read(
        &self,
        params: Parameters<BootstrapReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomBootstrapRead {
                request_id: random_id("req"),
                payload: BootstrapReadRequest { id: params.0.id },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_bootstrap_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "bootstrap",
        description = "Load Room bootstrap guidance; read-only alias."
    )]
    async fn bootstrap(&self) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap().await
    }

    #[tool(
        name = "bootstrap.read",
        description = "Read one Room bootstrap guide by id; read-only alias."
    )]
    async fn bootstrap_read(
        &self,
        params: Parameters<BootstrapReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap_read(params).await
    }

    #[tool(
        name = "skills.list",
        description = "List local Room skills with active state; read-only discovery."
    )]
    async fn skills_list(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsList {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| room_route_error_value_with_timeout(error, "skills_list_timeout"));
        Ok(result_from_value(slim_skills_list_response(value)))
    }

    #[tool(
        name = "skills.read",
        description = "Read one local Room skill package or resource."
    )]
    async fn skills_read(
        &self,
        params: Parameters<SkillReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillReadRequest {
            id: params.0.id,
            path: params.0.path,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.search",
        description = "Search local Room skill metadata and content; read-only."
    )]
    async fn skills_search(
        &self,
        params: Parameters<SkillSearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = SkillSearchRequest {
            query: params.query,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsSearch {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.active",
        description = "List active Room skill state, including stale entries; read-only."
    )]
    async fn skills_active(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsActive {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.activate",
        description = "Mark one valid Room skill active; executes nothing."
    )]
    async fn skills_activate(
        &self,
        params: Parameters<SkillActivationArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillActivationRequest { id: params.0.id };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsActivate {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.deactivate",
        description = "Remove active state for one Room skill; executes nothing."
    )]
    async fn skills_deactivate(
        &self,
        params: Parameters<SkillActivationArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillActivationRequest { id: params.0.id };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsDeactivate {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install",
        description = "Start one asynchronous Room skill installation; use install get/cancel for follow-up."
    )]
    async fn skills_install(
        &self,
        params: Parameters<SkillInstallArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = SkillInstallRequest {
            id: params.id,
            source: params.source.into_protocol(),
            replace_existing: params.replace_existing,
            activate_after_install: params.activate_after_install,
            idempotency_key: params.idempotency_key,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstall {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install.get",
        description = "Inspect or briefly wait for one Room skill installation; read-only lifecycle inspection."
    )]
    async fn skills_install_get(
        &self,
        params: Parameters<SkillInstallGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstallGet {
                request_id: random_id("req"),
                payload: SkillInstallGetRequest {
                    install_id: params.install_id,
                    wait_seconds: params.wait_seconds,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install.cancel",
        description = "Request cooperative cancellation of one Room skill installation before commit."
    )]
    async fn skills_install_cancel(
        &self,
        params: Parameters<SkillInstallCancelArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstallCancel {
                request_id: random_id("req"),
                payload: SkillInstallCancelRequest {
                    install_id: params.0.install_id,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.run",
        description = "Run an executable from an active Room skill as a managed Job."
    )]
    async fn skills_run(
        &self,
        params: Parameters<SkillRunArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsRun {
                request_id: random_id("req"),
                payload: SkillRunRequest {
                    id: params.id,
                    path: params.path,
                    group: params.group,
                    args: params.args,
                    working_directory: params.working_directory,
                    wait_seconds: params.wait_seconds,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }
}

impl AgenticMcpServer {
    fn ensure_agent_enabled(&self, agent_id: &str) -> Result<(), ErrorData> {
        match registry_entry(&self.state, agent_id) {
            Ok(Some(entry)) if entry.enabled => Ok(()),
            Ok(_) => Err(mcp_invalid_params(
                "agent_not_found",
                "Agent is not registered or enabled",
            )),
            Err(error) => Err(mcp_internal_error("db_error", error.to_string())),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AgentIdArgs {
    #[schemars(
        description = "Target local agent id. Room notebook tools do not use agentId; they route to the active Room Agent."
    )]
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HubRunGetArgs {
    #[schemars(description = "Run id returned by a timed-out Hub request.")]
    run_id: String,
}

#[derive(Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HubRunListArgs {
    #[serde(default)]
    #[schemars(description = "Optional agent id filter.")]
    agent_id: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional source filter such as hub or tunnel.")]
    source: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional status filter such as started, completed, failed, or timeout_waiting_result."
    )]
    status: Option<String>,
    #[serde(default)]
    #[schemars(description = "Only include records created within this many seconds.")]
    since_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(description = "Result count, default 20 and capped at 100.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    group: Option<String>,
    #[schemars(
        description = "Executable name or path. For shell syntax, use bash or sh with args such as ['-lc', '...']."
    )]
    program: String,
    #[serde(default)]
    #[schemars(
        description = "Argument vector passed directly to the program; this is not a shell-split string."
    )]
    args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation before execution. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override. Omit or use default to follow local agent config."
    )]
    confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Process working directory. Relative values resolve from the agent workspace root; prefer this over cd in shell commands."
    )]
    working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional human-readable workstream key inherited by every child Job."
    )]
    group: Option<String>,
    #[schemars(
        description = "Commands to run. Each element can override the top-level workingDirectory."
    )]
    elements: Vec<BatchExecElementArgs>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation for the batch. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override for all batch elements."
    )]
    confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Default process working directory for all batch elements. Relative values resolve from the agent workspace root."
    )]
    working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecElementArgs {
    #[schemars(description = "Executable name or path for this batch element.")]
    program: String,
    #[serde(default)]
    #[schemars(description = "Argument vector passed directly to the program.")]
    args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Per-element process working directory. Overrides the batch workingDirectory."
    )]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct JobIdArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(
        description = "Managed Job id returned by process.exec, process.batch, or skills.run."
    )]
    job_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct JobGetArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "Managed Job id.")]
    job_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_job_wait_seconds",
        description = "Bounded wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        default = "default_wait_only",
        description = "While waiting, suppress active intermediate detail; defaults to false; terminal completion still returns normal detail."
    )]
    wait_only: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct JobListArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(description = "Exact human-readable workstream filter.")]
    group: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional Job kind: process, skill, or mcp.")]
    kind: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional Job state filter.")]
    state: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_job_list_limit",
        description = "Maximum retained Jobs; defaults to 50 and is capped at 100."
    )]
    limit: Option<usize>,
    #[serde(default)]
    #[schemars(description = "Opaque cursor returned by a prior job.list response.")]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxListSessionsArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxListPanesArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional tmux session name to scope pane listing.")]
    session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCapturePaneArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    target: String,
    #[serde(default)]
    #[schemars(
        description = "Number of recent tmux history lines to capture. Defaults to 160 and caps at 5000."
    )]
    lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxPasteTextArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    target: String,
    #[schemars(description = "Text to paste into the tmux pane.")]
    text: String,
    #[serde(default)]
    #[schemars(description = "Append Enter after pasting the text. Defaults to false.")]
    submit: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before pasting. Defaults to true.")]
    need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "Shell pane target such as session:window.pane or %0.")]
    target: String,
    #[schemars(description = "Program or shell builtin to execute as one command.")]
    program: String,
    #[serde(default)]
    #[schemars(description = "Structured argument vector; shell operators are not interpreted.")]
    args: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Force local confirmation in addition to configured policy.")]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Milliseconds to wait before returning the post-submit pane snapshot. Defaults to 300 and caps at 5000."
    )]
    wait_ms: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Number of tmux history lines to include in the post-submit snapshot. Defaults to 120, caps at 5000, and 0 disables the snapshot."
    )]
    capture_lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCreateSessionArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux session name.")]
    name: String,
    #[schemars(description = "Session cwd, subject to the local agent path policy.")]
    cwd: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCloseSessionArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux session name.")]
    name: String,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before closing. Defaults to true.")]
    need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpListServersArgs {
    #[serde(default)]
    #[schemars(
        description = "Optional target local agent id. Omit to list MCP servers for all currently connected agents."
    )]
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpListToolsArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    server_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpCallToolArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    group: Option<String>,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    server_id: String,
    #[schemars(description = "Tool name returned by mcp.listTools.")]
    tool_name: String,
    #[serde(default)]
    #[schemars(
        description = "JSON object arguments forwarded to the MCP tool; maximum serialized size 256 KiB."
    )]
    arguments: Option<Value>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Absolute downstream execution deadline in seconds, default 300 and capped at 900."
    )]
    timeout_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum McpBatchModeArgs {
    #[default]
    Parallel,
    Sequential,
}

impl From<McpBatchModeArgs> for McpBatchMode {
    fn from(value: McpBatchModeArgs) -> Self {
        match value {
            McpBatchModeArgs::Parallel => Self::Parallel,
            McpBatchModeArgs::Sequential => Self::Sequential,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpBatchCallArgs {
    #[schemars(description = "Configured MCP server id.")]
    server_id: String,
    #[schemars(description = "Downstream MCP tool name.")]
    tool_name: String,
    #[serde(default)]
    #[schemars(description = "JSON object arguments; maximum serialized size 256 KiB per call.")]
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpBatchArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional human-readable workstream key inherited by every child Job."
    )]
    group: Option<String>,
    #[schemars(
        length(min = 1, max = 16),
        description = "Ordered 1..16 downstream MCP calls; aggregate serialized arguments are capped at 2 MiB."
    )]
    calls: Vec<McpBatchCallArgs>,
    #[serde(default)]
    #[schemars(description = "Execution mode: parallel by default, or sequential.")]
    mode: Option<McpBatchModeArgs>,
    #[serde(default)]
    #[schemars(
        description = "When true, prevent not-yet-started children from starting after a hard child failure; already-started calls are never cancelled."
    )]
    fail_fast: Option<bool>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 900),
        description = "Per-child downstream execution deadline in seconds after scheduling, default 300 and capped at 900."
    )]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UserNotifySendArgs {
    #[schemars(description = "Notification channel key returned by user.notify.channels.")]
    channel: String,
    #[schemars(description = "Notification title.")]
    title: String,
    #[schemars(description = "Notification body.")]
    body: String,
    #[serde(default)]
    #[schemars(description = "Optional notification actions. Phase A does not deliver actions.")]
    actions: Option<Vec<UserNotifyActionArgs>>,
    #[serde(default)]
    #[schemars(description = "Optional priority such as low, normal, high, urgent, or alarm.")]
    priority: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UserNotifyActionArgs {
    #[schemars(description = "Stable action id. Android ack will report this as actionId.")]
    id: String,
    #[schemars(description = "Human-readable action label.")]
    label: String,
}

impl From<UserNotifyActionArgs> for NotificationAction {
    fn from(value: UserNotifyActionArgs) -> Self {
        Self {
            id: value.id,
            label: value.label,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomDiaryActiveArgs {}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RoomDiaryLayerArgs {
    Daily,
    Weekly,
    Monthly,
}

impl From<RoomDiaryLayerArgs> for RoomDiaryLayer {
    fn from(value: RoomDiaryLayerArgs) -> Self {
        match value {
            RoomDiaryLayerArgs::Daily => Self::Daily,
            RoomDiaryLayerArgs::Weekly => Self::Weekly,
            RoomDiaryLayerArgs::Monthly => Self::Monthly,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomDiaryReadArgs {
    #[schemars(description = "Room diary temporal layer.")]
    layer: RoomDiaryLayerArgs,
    #[schemars(
        pattern(r"^(current|\d{4}-\d{2}-\d{2}(--\d{4}-\d{2}-\d{2})?)$"),
        description = "Room-local logical period: daily uses current or YYYY-MM-DD; weekly/monthly use current or YYYY-MM-DD--YYYY-MM-DD."
    )]
    period: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomNotebookRecentArgs {
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_room_notebook_limit",
        description = "Maximum bounded recent Notebook previews returned; defaults to 20."
    )]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomNotebookSearchArgs {
    #[schemars(
        length(min = 1, max = 256),
        description = "Case-insensitive bounded substring query over Notebook paths, H1 titles, and bodies."
    )]
    query: String,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_room_notebook_limit",
        description = "Maximum bounded Notebook previews returned; defaults to 20."
    )]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomNotebookReadArgs {
    #[schemars(
        description = "Exact Notebook-relative Markdown path returned or discovered under Notebook/; arbitrary repository paths are rejected."
    )]
    path: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomStateListArgs {}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomStateReadArgs {
    #[schemars(
        description = "State entity filename stem resolved under State/entities/; arbitrary repository paths are rejected."
    )]
    entity: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomMaintenanceStatusArgs {}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RoomMaintenanceSlotArgs {
    #[serde(rename = "diary.daily")]
    DiaryDaily,
    #[serde(rename = "diary.weekly")]
    DiaryWeekly,
    #[serde(rename = "diary.monthly")]
    DiaryMonthly,
    Notebook,
    Entity,
}

impl From<RoomMaintenanceSlotArgs> for RoomMaintenanceSlot {
    fn from(value: RoomMaintenanceSlotArgs) -> Self {
        match value {
            RoomMaintenanceSlotArgs::DiaryDaily => Self::DiaryDaily,
            RoomMaintenanceSlotArgs::DiaryWeekly => Self::DiaryWeekly,
            RoomMaintenanceSlotArgs::DiaryMonthly => Self::DiaryMonthly,
            RoomMaintenanceSlotArgs::Notebook => Self::Notebook,
            RoomMaintenanceSlotArgs::Entity => Self::Entity,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RoomMaintenanceModeArgs {
    Local,
    Workflow,
}

impl From<RoomMaintenanceModeArgs> for RoomMaintenanceExecutionMode {
    fn from(value: RoomMaintenanceModeArgs) -> Self {
        match value {
            RoomMaintenanceModeArgs::Local => Self::Local,
            RoomMaintenanceModeArgs::Workflow => Self::Workflow,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomMaintenanceItemArgs {
    #[schemars(description = "Unique Room semantic slot to maintain.")]
    slot: RoomMaintenanceSlotArgs,
    #[schemars(
        description = "Slot-specific maintenance payload; validated by the Room maintenance executor."
    )]
    payload: Value,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomMaintenanceSubmitArgs {
    #[schemars(
        length(min = 1, max = 5),
        description = "One to five maintenance requests; each slot may appear at most once. The set is validated against the Room repository before any mutation."
    )]
    items: Vec<RoomMaintenanceItemArgs>,
    #[serde(default)]
    #[schemars(
        description = "Optional execution mode override; local applies in the validated Room repository, workflow submits through the configured Room workflow."
    )]
    mode: Option<RoomMaintenanceModeArgs>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_room_wait_seconds",
        description = "Optional bounded wait for workflow consumption and local fast-forward, from 0 through 30 seconds."
    )]
    wait_seconds: Option<u8>,
}

impl RoomMaintenanceSubmitArgs {
    fn into_protocol(self) -> RoomMaintenanceSubmitRequest {
        RoomMaintenanceSubmitRequest {
            items: self
                .items
                .into_iter()
                .map(|item| RoomMaintenanceRequestItem {
                    slot: item.slot.into(),
                    payload: item.payload,
                })
                .collect(),
            mode: self.mode.map(RoomMaintenanceExecutionMode::from),
            wait_seconds: self.wait_seconds,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BootstrapReadArgs {
    #[schemars(description = "Guide id returned by room.bootstrap.")]
    id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillReadArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional package-relative file path. Omit to read the legacy SKILL.md response."
    )]
    path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillSearchArgs {
    #[schemars(
        description = "Case-insensitive substring query over id, frontmatter, tags, and SKILL.md content."
    )]
    query: String,
    #[serde(default)]
    #[schemars(description = "Maximum skills returned. Defaults to 20 and caps at 100.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillActivationArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallArgs {
    #[schemars(description = "Target skill id. One installation job targets exactly one id.")]
    id: String,
    #[schemars(description = "GitHub, HTTPS-file, or inline-content source descriptor.")]
    source: SkillInstallSourceArgs,
    #[serde(default)]
    #[schemars(
        description = "Archive an existing workspace skill before replacement. Defaults to false."
    )]
    replace_existing: bool,
    #[serde(default)]
    #[schemars(
        description = "Optional explicit activation choice; new skills default active and replacement preserves its prior state."
    )]
    activate_after_install: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional idempotency key for safe retries of the same install request."
    )]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum SkillInstallSourceArgs {
    Github {
        #[serde(default)]
        repository: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(rename = "ref", default)]
        ref_name: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
    Files {
        files: Vec<SkillInstallFileArgs>,
    },
}

impl SkillInstallSourceArgs {
    fn into_protocol(self) -> SkillInstallSource {
        match self {
            Self::Github {
                repository,
                url,
                ref_name,
                path,
            } => SkillInstallSource::Github {
                repository,
                url,
                ref_name,
                path,
            },
            Self::Files { files } => SkillInstallSource::Files {
                files: files
                    .into_iter()
                    .map(SkillInstallFileArgs::into_protocol)
                    .collect(),
            },
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallFileArgs {
    path: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    executable: Option<bool>,
}

impl SkillInstallFileArgs {
    fn into_protocol(self) -> SkillInstallFile {
        SkillInstallFile {
            path: self.path,
            url: self.url,
            content: self.content,
            content_base64: self.content_base64,
            sha256: self.sha256,
            executable: self.executable,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallGetArgs {
    install_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded status wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallCancelArgs {
    install_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillRunArgs {
    id: String,
    #[schemars(description = "Package-relative executable path under scripts/.")]
    path: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    group: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    wait_seconds: Option<u64>,
}

fn ok_json(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
}

fn remove_empty_warnings(value: &mut Value) {
    if value
        .get("warnings")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        if let Some(object) = value.as_object_mut() {
            object.remove("warnings");
        }
    }
}

fn slim_skills_list_response(mut value: Value) -> Value {
    remove_empty_warnings(&mut value);
    if let Some(skills) = value.get_mut("skills").and_then(Value::as_array_mut) {
        for skill in skills {
            remove_empty_warnings(skill);
        }
    }
    value
}

fn result_from_value(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
}

fn room_route_error_value(error: RoomRouteError) -> Value {
    room_route_error_value_with_timeout(error, "room_notebook_timeout")
}

fn room_route_error_value_with_timeout(error: RoomRouteError, timeout_code: &'static str) -> Value {
    match error {
        RoomRouteError::NotActive => json!({
            "error": { "code": "room_not_active", "message": "no active room agent" }
        }),
        RoomRouteError::StateConflict => json!({
            "error": { "code": "room_state_conflict", "message": "active room state is inconsistent" }
        }),
        RoomRouteError::Timeout(reason) => json!({
            "error": { "code": timeout_code, "message": reason }
        }),
    }
}

fn parse_job_group(value: Option<String>) -> Result<Option<String>, ErrorData> {
    normalize_job_group(value.as_deref())
        .map_err(|error| mcp_invalid_params(error.code(), error.message()))
}

fn mcp_invalid_params(code: &'static str, message: impl ToString) -> ErrorData {
    ErrorData::invalid_params(message.to_string(), Some(json!({ "code": code })))
}

fn mcp_internal_error(code: &'static str, message: String) -> ErrorData {
    ErrorData::internal_error(message, Some(json!({ "code": code })))
}

fn notify_route_error_code(error: &NotifyRouteError) -> &'static str {
    match error {
        NotifyRouteError::InvalidChannel(_) => "invalid_notify_channel",
        NotifyRouteError::AgentNotFound(_) => "agent_alias_not_found",
        NotifyRouteError::ChannelUnavailable { reason, .. } => reason,
        NotifyRouteError::DeliveryFailed { .. } => "notify_delivery_failed",
        NotifyRouteError::Db(_) => "db_error",
    }
}

fn notify_route_error_message(error: &NotifyRouteError) -> String {
    match error {
        NotifyRouteError::InvalidChannel(channel) => {
            format!("Invalid notification channel: {channel}")
        }
        NotifyRouteError::AgentNotFound(alias) => {
            format!("No enabled agent found for alias: {alias}")
        }
        NotifyRouteError::ChannelUnavailable {
            channel_key,
            reason,
        } => format!("Notification channel {channel_key} is unavailable: {reason}"),
        NotifyRouteError::DeliveryFailed {
            channel_key,
            reason,
        } => format!("Notification delivery failed for {channel_key}: {reason}"),
        NotifyRouteError::Db(reason) => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::{HubConfig, McpProfile, RemoteConfirmationConfig};
    use axum::body::to_bytes;
    use axum::extract::State;
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_hub_config() -> HubConfig {
        HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: true,
                provider: "ntfy".to_string(),
                timeout_seconds: 45,
                ntfy: crate::NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "secret-topic-for-test".to_string(),
                    callback_base_url: "https://callback.example.invalid".to_string(),
                },
            },
        }
    }

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(test_hub_config()),
            mcp_profile: McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            job_cache: Arc::new(crate::state::JobCache::new()),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(crate::notify::NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: chrono::Utc::now(),
                result: crate::notify::NtfyHealthStatus::Healthy,
            }))),
        }
    }

    #[test]
    fn tool_read_only_hints_match_side_effect_semantics() {
        for name in [
            "agent.list",
            "job.list",
            "tmux.listSessions",
            "tmux.listPanes",
            "tmux.capturePane",
            "hub.run.get",
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
            "room.bootstrap",
            "room.bootstrap.read",
            "skills.list",
            "skills.read",
            "skills.search",
            "skills.active",
            "skills.install.get",
        ] {
            assert!(tool_is_read_only(name), "{name} should be read-only");
        }
        for name in [
            "process.exec",
            "process.exec",
            "job.cancel",
            "tmux.exec",
            "tmux.pasteText",
            "tmux.createSession",
            "tmux.closeSession",
            "mcp.batch",
            "mcp.callTool",
            "user.notify.send",
            "room.maintenance.submit",
            "skills.activate",
            "skills.deactivate",
            "skills.install",
            "skills.install.cancel",
            "skills.run",
        ] {
            assert!(!tool_is_read_only(name), "{name} should not be read-only");
        }
    }

    #[test]
    fn coordinator_profile_exposes_only_native_tools() {
        let mut state = test_state();
        state.mcp_profile = McpProfile::Coordinator;
        let server = AgenticMcpServer::new(state);
        let mut names = app_tool_descriptors(&server)
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "agent.list",
                "hub.info",
                "hub.job.get",
                "hub.job.list",
                "hub.run.get",
                "hub.run.list",
                "user.notify.channels",
                "user.notify.send",
            ]
        );
    }

    #[tokio::test]
    async fn coordinator_rejects_hidden_execution_tools_before_dispatch() {
        let mut state = test_state();
        state.mcp_profile = McpProfile::Coordinator;
        let server = AgenticMcpServer::new(state);
        let error = call_app_tool(
            &server,
            json!({ "name": "process.exec", "arguments": { "agentId": "agent" } }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("tool_unavailable_for_profile"));
        for name in [
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
            "room.maintenance.submit",
        ] {
            let error = call_app_tool(&server, json!({ "name": name, "arguments": {} }))
                .await
                .unwrap_err();
            assert!(
                error.contains("tool_unavailable_for_profile"),
                "{name}: {error}"
            );
        }
        let run_count: i64 = server
            .state
            .db
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 0);
    }

    #[test]
    fn full_profile_keeps_bootstrap_aliases_and_execution_surface() {
        let server = AgenticMcpServer::new(test_state());
        let names = app_tool_descriptors(&server)
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "bootstrap"));
        assert!(names.iter().any(|name| name == "bootstrap.read"));
        assert!(names.iter().any(|name| name == "process.exec"));
        assert!(names.iter().any(|name| name == "mcp.batch"));
        assert!(names.iter().any(|name| name == "hub.job.list"));
        for name in [
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
            "room.maintenance.submit",
        ] {
            assert!(
                names.iter().any(|candidate| candidate == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations() {
        let server = AgenticMcpServer::new(test_state());
        let tools = app_tool_descriptors(&server);
        let batch = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("mcp.batch"))
            .expect("mcp.batch descriptor missing");
        assert_eq!(batch["annotations"]["readOnlyHint"], false);
        assert_eq!(batch["annotations"]["destructiveHint"], false);
        assert_eq!(batch["annotations"]["openWorldHint"], true);
        let calls = &batch["inputSchema"]["properties"]["calls"];
        assert_eq!(calls["type"], "array");
        assert_eq!(calls["minItems"], 1);
        assert_eq!(calls["maxItems"], 16);
        assert_eq!(
            batch["inputSchema"]["properties"]["waitSeconds"]["minimum"],
            0
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["waitSeconds"]["maximum"],
            30
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["timeoutSeconds"]["minimum"],
            1
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["timeoutSeconds"]["maximum"],
            900
        );
        assert!(batch["inputSchema"]["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("agentId"))
                && required.contains(&json!("calls"))));
    }

    #[test]
    fn skill_install_and_run_tools_are_exposed_with_stable_annotations() {
        let server = AgenticMcpServer::new(test_state());
        let tools = app_tool_descriptors(&server);
        let mut names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        names.sort_unstable();
        for name in [
            "room.bootstrap",
            "room.bootstrap.read",
            "skills.install",
            "skills.install.get",
            "skills.install.cancel",
            "skills.run",
        ] {
            assert!(names.contains(&name), "missing MCP tool {name}");
        }
        let install = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("skills.install"))
            .unwrap();
        assert_eq!(install["annotations"]["readOnlyHint"], false);
        assert_eq!(install["annotations"]["destructiveHint"], true);
        let get = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("skills.install.get"))
            .unwrap();
        assert_eq!(get["annotations"]["readOnlyHint"], true);

        for name in ["room.bootstrap", "room.bootstrap.read"] {
            let tool = tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                .unwrap_or_else(|| panic!("missing MCP tool {name}"));
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
            assert_eq!(tool["annotations"]["openWorldHint"], false);
        }
    }

    #[tokio::test]
    async fn every_advertised_tool_is_accepted_by_apps_dispatcher() {
        let server = AgenticMcpServer::new(test_state());
        for tool in app_tool_descriptors(&server) {
            let name = tool["name"].as_str().unwrap();
            let result = call_app_tool(&server, json!({ "name": name, "arguments": {} })).await;
            if let Err(error) = result {
                assert!(
                    !error.starts_with("Unknown tool:"),
                    "advertised tool {name} is not accepted by tools/call: {error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn apps_bootstrap_tools_are_callable_through_tools_call() {
        for (name, arguments) in [
            ("room.bootstrap", json!({})),
            ("room.bootstrap.read", json!({ "id": "missing" })),
        ] {
            let response = mcp_post(
                State(test_state()),
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": name, "arguments": arguments }
                })),
            )
            .await;
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert!(
                value.get("error").is_none(),
                "{name} was not dispatched: {value}"
            );
            assert_eq!(value["result"]["isError"], true);
            assert_eq!(
                value["result"]["structuredContent"]["error"]["code"],
                "room_not_active"
            );
        }
    }

    #[test]
    fn bootstrap_timeout_values_preserve_operation_specific_codes() {
        for (code, expected) in [
            ("room_bootstrap_timeout", "room_bootstrap_timeout"),
            ("room_bootstrap_read_timeout", "room_bootstrap_read_timeout"),
        ] {
            let value = room_route_error_value_with_timeout(
                RoomRouteError::Timeout("timed out".to_string()),
                code,
            );
            let result = serde_json::to_value(result_from_value(value)).unwrap();
            assert_eq!(result["isError"], true);
            assert_eq!(result["structuredContent"]["error"]["code"], expected);
        }
    }

    #[test]
    fn tmux_paste_schema_exposes_confirmation_default_field() {
        let schema =
            serde_json::to_string(&rmcp::schemars::schema_for!(TmuxPasteTextArgs)).unwrap();
        assert!(schema.contains("needConfirm"));
        assert!(schema.contains("submit"));
    }

    #[test]
    fn tmux_exec_schema_exposes_snapshot_fields() {
        let schema = serde_json::to_string(&rmcp::schemars::schema_for!(TmuxExecArgs)).unwrap();
        assert!(schema.contains("waitMs"));
        assert!(schema.contains("captureLines"));
    }

    #[test]
    fn room_mcp_input_schemas_do_not_include_agent_id() {
        let schemas = [
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryActiveArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookRecentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomStateListArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomStateReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomMaintenanceStatusArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomMaintenanceSubmitArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(BootstrapReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillActivationArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallGetArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallCancelArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillRunArgs)).unwrap(),
        ];
        for schema in schemas {
            assert!(!schema.contains("agentId"));
            assert!(!schema.contains("agent_id"));
        }
    }

    #[test]
    fn native_tool_values_use_agentic_result_shape() {
        let value = json!({ "jobs": [] });

        let result = result_from_value(value.clone());
        let serialized = serde_json::to_value(result).unwrap();

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["isError"], false);
        assert_eq!(serialized["content"][0]["type"], "text");
    }

    #[tokio::test]
    async fn mcp_tools_call_wire_response_uses_agentic_result_shape() {
        let response = mcp_post(
            State(test_state()),
            Json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "agent.list",
                    "arguments": {}
                }
            })),
        )
        .await;

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
        assert_eq!(value["result"]["content"][0]["type"], "text");
        assert_eq!(value["result"]["structuredContent"]["agents"], json!([]));
        assert_eq!(value["result"]["isError"], false);
    }

    #[test]
    fn job_get_descriptor_and_wait_normalization_match_protocol_contract() {
        assert_eq!(
            default_job_wait_seconds(),
            JobGetRequest::DEFAULT_WAIT_SECONDS
        );
        let schema = serde_json::to_string(&rmcp::schemars::schema_for!(JobGetArgs)).unwrap();
        assert!(schema.contains("\"default\":5"));
        assert!(schema.contains("\"maximum\":30"));

        for (wait_seconds, expected) in [(None, 5), (Some(0), 0), (Some(31), 30)] {
            let params = JobGetArgs {
                agent_id: "agent".to_string(),
                job_id: "job".to_string(),
                wait_seconds,
                wait_only: None,
            };
            let payload = job_get_payload(&params);
            assert_eq!(payload.wait_seconds, Some(expected));
            assert_eq!(payload.effective_wait_seconds(), expected);
        }
    }
}
