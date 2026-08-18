use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use agentic_gpt_protocol::{
    normalize_job_group, BatchExecRequest, ExecElement, ExecRequest, HubCommand, JobBatchResponse,
    JobCancelResponse, JobDetail, JobError, JobInfo, JobKind, JobListItem, JobListRequest,
    JobListResponse, JobResponse, JobState, JobToolResponse, JobWaitResponse, McpBatchResponse,
    McpBatchToolChildResponse, McpBatchToolResponse,
};
#[cfg(test)]
use agentic_gpt_protocol::{McpBatchChildResponse, McpBatchStatus};
use anyhow::Result;
use chrono::Utc;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientJsonRpcMessage,
        ClientRequest, ErrorData, Implementation, InitializeRequest, InitializeRequestParams,
        ListToolsResult, Meta, PaginatedRequestParams, ProtocolVersion, RequestId,
        ServerCapabilities, ServerInfo, ServerJsonRpcMessage, Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
    transport::{async_rw::AsyncRwTransport, stdio, Transport},
    ServerHandler, ServiceExt,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{
    local_service,
    state::{AppState, CapabilityProfile},
};

const INSTRUCTIONS: &str = "Agentic GPT local Tunnel worker. Start with agent.info to inspect the active profile, exact workspace/path policy, capacity, confirmation channels, MCP scheduler state, and connection state. Use file.read/search for bounded UTF-8 workspace work and file.edit for Codex apply-patch edits, process.exec/process.batch for process Jobs, mcp.callTool for one downstream MCP Job, mcp.batch for 1..16 atomically admitted child Jobs with one aggregate confirmation and bounded 8/2 concurrency, job.get/list/cancel for lifecycle control, tmux for persistent workspaces, skills for the local skills workspace, and bootstrap for Room startup guidance. All calls remain subject to path policy, configured confirmation, audit, and bounded waits.";
const PATCH_SCHEMA_DESCRIPTION: &str = "Codex apply_patch text beginning with *** Begin Patch and ending with *** End Patch; supports Add File, Delete File, Update File, and Move to across multiple files.";

const NORMAL_TOOLS: &[&str] = &[
    "agent.info",
    "file.read",
    "file.search",
    "file.edit",
    "mcp.batch",
    "mcp.callTool",
    "mcp.list",
    "process.batch",
    "process.exec",
    "job.cancel",
    "job.get",
    "job.list",
    "skills.install",
    "skills.install.cancel",
    "skills.install.get",
    "skills.list",
    "skills.read",
    "skills.run",
    "skills.setActive",
    "tmux.exec",
    "tmux.pasteText",
    "tmux.panes",
    "tmux.sessions",
];

const ROOM_BOOTSTRAP_TOOLS: &[&str] = &["bootstrap", "bootstrap.read"];

const ROOM_ONLY_TOOLS: &[&str] = &[
    "room.diary.append",
    "room.diary.recent",
    "room.diary.selectExact",
    "room.notebook.append",
    "room.notebook.current",
    "room.notebook.recent",
    "room.notebook.remove",
    "room.notebook.search",
    "room.notebook.selectExact",
    "room.notebook.update",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestIngress {
    TunnelStdio,
    LocalUnix,
}

impl RequestIngress {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::TunnelStdio => "tunnel:stdio",
            Self::LocalUnix => "local:unix",
        }
    }

    fn source(self, tool: &str) -> String {
        let prefix = match self {
            Self::TunnelStdio => "tunnel",
            Self::LocalUnix => "local",
        };
        format!("{prefix}:{tool}")
    }
}

pub(crate) async fn serve_stdio(state: AppState) -> Result<()> {
    let server = AgentMcpServer::new(state);
    let (stdin, stdout) = stdio();
    let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(stdin, stdout);
    let running = server
        .serve(ResumableStdioTransport::new(transport))
        .await?;
    let _ = running.waiting().await?;
    Ok(())
}

const INTERNAL_STDIO_CLIENT_NAME: &str = "agentic-gpt-stdio-resume";

#[derive(Debug)]
enum StdioInitializationState {
    AwaitingClientInitialize,
    SyntheticInitializePending { id: RequestId },
    Initialized,
}

/// Restores rmcp's private server state when the tunnel control plane resumes
/// an initialized logical connection against a freshly restarted stdio worker.
/// The synthetic handshake stays inside this transport so its private request
/// id can never leak into the tunnel request/response stream.
struct ResumableStdioTransport<T> {
    inner: T,
    state: StdioInitializationState,
    pending: Option<ClientJsonRpcMessage>,
}

impl<T> ResumableStdioTransport<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            state: StdioInitializationState::AwaitingClientInitialize,
            pending: None,
        }
    }

    fn synthetic_initialize(id: RequestId) -> ClientJsonRpcMessage {
        let params = InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(INTERNAL_STDIO_CLIENT_NAME, env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2025_06_18);
        ClientJsonRpcMessage::request(
            ClientRequest::InitializeRequest(InitializeRequest::new(params)),
            id,
        )
    }
}

impl<T> Transport<RoleServer> for ResumableStdioTransport<T>
where
    T: Transport<RoleServer> + 'static,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let synthetic_result = match (&self.state, &item) {
            (
                StdioInitializationState::SyntheticInitializePending { id },
                ServerJsonRpcMessage::Response(response),
            ) if &response.id == id => Some(true),
            (
                StdioInitializationState::SyntheticInitializePending { id },
                ServerJsonRpcMessage::Error(error),
            ) if error.id.as_ref() == Some(id) => Some(false),
            _ => None,
        };

        if let Some(success) = synthetic_result {
            if success {
                self.state = StdioInitializationState::Initialized;
                crate::utils::log_info(
                    "mcp_stdio_session_resumed; ingress=tunnel:stdio; workerContinues=true"
                        .to_string(),
                );
            } else {
                crate::utils::log_warn(
                    "mcp_stdio_session_resume_failed; ingress=tunnel:stdio".to_string(),
                );
            }
        }

        let delegated = if synthetic_result.is_some() {
            None
        } else {
            Some(self.inner.send(item))
        };
        async move {
            match delegated {
                Some(send) => send.await,
                None => Ok(()),
            }
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        if matches!(self.state, StdioInitializationState::Initialized) {
            if let Some(pending) = self.pending.take() {
                return Some(pending);
            }
            return self.inner.receive().await;
        }

        if matches!(
            self.state,
            StdioInitializationState::SyntheticInitializePending { .. }
        ) {
            crate::utils::log_warn(
                "mcp_stdio_session_resume_invariant_failed; ingress=tunnel:stdio".to_string(),
            );
            return None;
        }

        loop {
            let message = self.inner.receive().await?;
            match message {
                ClientJsonRpcMessage::Request(request)
                    if matches!(&request.request, ClientRequest::InitializeRequest(_)) =>
                {
                    self.state = StdioInitializationState::Initialized;
                    return Some(ClientJsonRpcMessage::Request(request));
                }
                ClientJsonRpcMessage::Request(request)
                    if matches!(&request.request, ClientRequest::PingRequest(_)) =>
                {
                    return Some(ClientJsonRpcMessage::Request(request));
                }
                ClientJsonRpcMessage::Request(request) => {
                    let trigger_method = request.request.method().to_string();
                    let id = RequestId::String(
                        format!("agentic-gpt-internal-init-{}", Uuid::new_v4()).into(),
                    );
                    self.pending = Some(ClientJsonRpcMessage::Request(request));
                    self.state =
                        StdioInitializationState::SyntheticInitializePending { id: id.clone() };
                    crate::utils::log_warn(format!(
                        "mcp_stdio_session_resume; ingress=tunnel:stdio; triggerMethod={trigger_method}; action=synthetic_initialize"
                    ));
                    return Some(Self::synthetic_initialize(id));
                }
                ClientJsonRpcMessage::Notification(_) => {
                    crate::utils::log_warn(
                        "mcp_message_before_initialize; ingress=tunnel:stdio; messageKind=notification; action=ignored"
                            .to_string(),
                    );
                }
                ClientJsonRpcMessage::Response(_) => {
                    crate::utils::log_warn(
                        "mcp_message_before_initialize; ingress=tunnel:stdio; messageKind=response; action=ignored"
                            .to_string(),
                    );
                }
                ClientJsonRpcMessage::Error(_) => {
                    crate::utils::log_warn(
                        "mcp_message_before_initialize; ingress=tunnel:stdio; messageKind=error; action=ignored"
                            .to_string(),
                    );
                }
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

#[derive(Clone)]
pub(crate) struct AgentMcpServer {
    state: AppState,
    tools: Arc<Vec<Tool>>,
    ingress: RequestIngress,
}

#[derive(Default)]
enum HumanResponseState {
    #[default]
    Awaiting,
    Inline,
    Active,
}

#[derive(Default)]
struct HumanTerminalState {
    response: HumanResponseState,
    pending: Vec<String>,
}

struct HumanTerminalTracker {
    state: Mutex<HumanTerminalState>,
    emitter: Arc<dyn Fn(String) + Send + Sync>,
}

impl Default for HumanTerminalTracker {
    fn default() -> Self {
        Self::with_emitter(crate::utils::log_info)
    }
}

impl HumanTerminalTracker {
    fn with_emitter(emitter: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            state: Mutex::new(HumanTerminalState::default()),
            emitter: Arc::new(emitter),
        }
    }

    fn record(&self, profile: &str, source: &str, job: &JobInfo) {
        let message = managed_terminal_event_message(profile, source, job);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.response {
            HumanResponseState::Awaiting => state.pending.push(message),
            HumanResponseState::Inline => {}
            HumanResponseState::Active => (self.emitter)(message),
        }
    }

    fn finish_response(&self, inline: bool, lifecycle_message: Option<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(state.response, HumanResponseState::Awaiting) {
            return;
        }
        if inline {
            state.response = HumanResponseState::Inline;
            state.pending.clear();
            if let Some(message) = lifecycle_message {
                (self.emitter)(message);
            }
            return;
        }

        state.response = HumanResponseState::Active;
        if let Some(message) = lifecycle_message {
            (self.emitter)(message);
        }
        for message in std::mem::take(&mut state.pending) {
            (self.emitter)(message);
        }
    }
}

impl AgentMcpServer {
    pub(crate) fn new(state: AppState) -> Self {
        Self::with_ingress(state, RequestIngress::TunnelStdio)
    }

    pub(crate) fn with_ingress(state: AppState, ingress: RequestIngress) -> Self {
        let profile = state.runtime.profile;
        let mut names = NORMAL_TOOLS.to_vec();
        if profile == CapabilityProfile::Room {
            names.extend_from_slice(ROOM_BOOTSTRAP_TOOLS);
            names.extend_from_slice(ROOM_ONLY_TOOLS);
        }
        names.sort_unstable();
        let tools = names.into_iter().map(tool_descriptor).collect::<Vec<_>>();
        Self {
            state,
            tools: Arc::new(tools),
            ingress,
        }
    }

    async fn call(&self, request: CallToolRequestParams) -> Result<Value, ErrorData> {
        let name = request.name.to_string();
        if !self.tools.iter().any(|tool| tool.name == name) {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("Tool is not available: {name}"),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let run_id = task_id("run");
        let report_request_id = task_id("req");
        let started_at = Utc::now();
        let terminal_tracker = Arc::new(HumanTerminalTracker::default());
        crate::hub::report_tool_arguments(
            &self.state,
            &run_id,
            &report_request_id,
            &name,
            arguments.clone(),
            started_at,
        );
        let value = match self
            .dispatch_with_lifecycle(&name, arguments, terminal_tracker.clone())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                let lifecycle = format!(
                    "mcp_tool; ingress={}; run={}; tool={name}; profile={}; status=failed; durationMs={}; errorCode={}",
                    self.ingress.label(),
                    crate::utils::compact_id(&run_id),
                    self.state.runtime.profile.label(),
                    (Utc::now() - started_at).num_milliseconds().max(0),
                    bounded_error_code(&error.to_string())
                );
                terminal_tracker.finish_response(true, Some(lifecycle));
                crate::hub::report_run_event(
                    &self.state,
                    &run_id,
                    &report_request_id,
                    &name,
                    "failed",
                    started_at,
                    None,
                    Some(error.to_string()),
                    None,
                );
                return Err(ErrorData::invalid_params(error.to_string(), None));
            }
        };
        let job: Option<agentic_gpt_protocol::JobInfo> = value
            .get("job")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .or_else(|| serde_json::from_value(value.clone()).ok());
        if let Some(job) = job.as_ref() {
            crate::hub::report_job(&self.state, job.clone());
        }
        let is_error = value.get("error").is_some();
        let reason = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let job_id = job.as_ref().map(|job| job.job_id.as_str());
        let exit_code = job.as_ref().and_then(|job| job.exit_code);
        let active = value_has_active_job(&value);
        let terminal_failure = value_has_terminal_failure(&value);
        let human_reason = reason
            .clone()
            .or_else(|| human_failure_reason(&value, job.as_ref()));
        let mut lifecycle = format!(
            "mcp_tool; ingress={}; run={}; tool={name}; profile={}; status={}; durationMs={}",
            self.ingress.label(),
            crate::utils::compact_id(&run_id),
            self.state.runtime.profile.label(),
            if is_error || terminal_failure {
                "failed"
            } else if active {
                "active"
            } else {
                "completed"
            },
            (Utc::now() - started_at).num_milliseconds().max(0),
        );
        if let Some(job_id) = job_id {
            lifecycle.push_str(&format!("; job={}", crate::utils::compact_id(job_id)));
        }
        if let Some(exit_code) = exit_code {
            lifecycle.push_str(&format!("; exitCode={exit_code}"));
        }
        if let Some(reason) = human_reason.as_deref() {
            lifecycle.push_str(&format!("; errorCode={}", bounded_error_code(reason)));
        }
        terminal_tracker.finish_response(!active, Some(lifecycle));
        crate::hub::report_run_event(
            &self.state,
            &run_id,
            &report_request_id,
            &name,
            if is_error { "failed" } else { "completed" },
            started_at,
            Some(value.clone()),
            reason,
            job,
        );
        Ok(value)
    }

    #[cfg(test)]
    async fn dispatch(&self, name: &str, arguments: Value) -> Result<Value> {
        let terminal_tracker = Arc::new(HumanTerminalTracker::default());
        let result = self
            .dispatch_with_lifecycle(name, arguments, terminal_tracker.clone())
            .await;
        terminal_tracker.finish_response(
            result
                .as_ref()
                .map(|value| !value_has_active_job(value))
                .unwrap_or(true),
            None,
        );
        result
    }

    async fn dispatch_with_lifecycle(
        &self,
        name: &str,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        if self.tools.iter().any(|tool| tool.name == name) {
            validate_stdio_arguments(name, &arguments)?;
        }
        let request_id = request_id();
        match name {
            "agent.info" => {
                let _: EmptyArgs = from_value(arguments)?;
                Ok(crate::agent_info::collect(&self.state).await)
            }
            "file.read" => {
                let args: FileReadArgs = from_value(arguments)?;
                validate_file_read_args(&args)?;
                if let Some(requests) = args.requests {
                    return Ok(crate::file_ops::read_batch(
                        &self.state,
                        &requests.into_iter().map(Into::into).collect::<Vec<_>>(),
                    )
                    .await);
                }
                let config = self.state.config.read().await.clone();
                let path = args.path.expect("validated single file.read path");
                match crate::file_ops::resolve_path(&config, &path, crate::file_ops::Access::Read) {
                    Ok(resolved) => crate::file_ops::to_result(crate::file_ops::read(
                        &resolved,
                        args.metadata.unwrap_or(false),
                        args.start_line,
                        args.end_line,
                    )),
                    Err(error) => Ok(error.value()),
                }
            }
            "file.search" => {
                let args: FileSearchArgs = from_value(arguments)?;
                validate_file_search_args(&args)?;
                if let Some(requests) = args.requests {
                    return Ok(crate::file_ops::search_batch(
                        &self.state,
                        &requests.into_iter().map(Into::into).collect::<Vec<_>>(),
                    )
                    .await);
                }
                let config = self.state.config.read().await.clone();
                let resolved = crate::file_ops::resolve_path(
                    &config,
                    args.path
                        .as_deref()
                        .expect("validated single file.search path"),
                    crate::file_ops::Access::Read,
                );
                match resolved {
                    Ok(resolved) => crate::file_ops::to_result(
                        crate::file_ops::search_with_context_limit(
                            crate::file_ops::SearchOptions {
                                root: &resolved,
                                query: args.query.as_deref().expect("validated search query"),
                                mode: if args.mode.as_deref() == Some("regex") {
                                    crate::file_ops::SearchMode::Regex
                                } else {
                                    crate::file_ops::SearchMode::Literal
                                },
                                case_sensitive: args.case_sensitive.unwrap_or(true),
                                include: args.include.as_deref().unwrap_or(&[]),
                                exclude: args.exclude.as_deref().unwrap_or(&[]),
                                context_lines: args.context_lines.unwrap_or(0),
                                max_results: args.max_results.unwrap_or(50),
                                hidden: args.hidden.unwrap_or(false),
                                respect_gitignore: args.respect_gitignore.unwrap_or(true),
                                scan_file_limit: crate::file_ops::MAX_SEARCH_FILES,
                                scan_byte_limit: crate::file_ops::MAX_SEARCH_BYTES,
                            },
                            config.limits.max_file_search_context_lines,
                        )
                        .map(crate::file_ops::slim_search_response),
                    ),
                    Err(error) if error.code == "file_not_found" => {
                        Ok(crate::file_ops::FileError::new(
                            "file_search_path_not_found",
                            "search path was not found",
                        )
                        .value())
                    }
                    Err(error) => Ok(error.value()),
                }
            }
            "file.edit" => {
                let args: FileEditArgs = from_value(arguments)?;
                Ok(crate::file_ops::edit(
                    &self.state,
                    crate::file_ops::EditRequest {
                        patch: args.patch,
                        need_confirm: args.need_confirm,
                    },
                )
                .await)
            }
            "process.exec" => {
                self.dispatch_process_exec(arguments, terminal_tracker)
                    .await
            }
            "process.batch" => {
                self.dispatch_process_batch(arguments, terminal_tracker)
                    .await
            }
            "job.get" => self.dispatch_job_get(arguments).await,
            "job.cancel" => self.dispatch_job_cancel(arguments).await,
            "job.list" => self.dispatch_job_list(arguments).await,
            "tmux.sessions" => {
                let args: TmuxSessionsArgs = from_value(arguments)?;
                validate_tmux_sessions_args(&args)?;
                match args.action.as_str() {
                    "list" if args.name.is_none() && args.cwd.is_none() => {
                        Ok(crate::tmux::list_sessions().await)
                    }
                    "create" => {
                        let name = args
                            .name
                            .ok_or_else(|| anyhow::anyhow!("name is required for create"))?;
                        let cwd = args
                            .cwd
                            .ok_or_else(|| anyhow::anyhow!("cwd is required for create"))?;
                        Ok(crate::tmux::create_session(
                            &self.state,
                            agentic_gpt_protocol::TmuxCreateSessionRequest { name, cwd },
                        )
                        .await)
                    }
                    "close" => {
                        let name = args
                            .name
                            .ok_or_else(|| anyhow::anyhow!("name is required for close"))?;
                        Ok(crate::tmux::close_session(
                            &self.state,
                            agentic_gpt_protocol::TmuxCloseSessionRequest {
                                name,
                                need_confirm: args.need_confirm.unwrap_or(true),
                            },
                        )
                        .await)
                    }
                    _ => Err(anyhow::anyhow!("invalid tmux.sessions action")),
                }
            }
            "tmux.panes" => {
                let args: TmuxPanesArgs = from_value(arguments)?;
                validate_tmux_panes_args(&args)?;
                match args.action.as_str() {
                    "list" => Ok(crate::tmux::list_panes(
                        agentic_gpt_protocol::TmuxListPanesRequest {
                            session: args.session,
                        },
                    )
                    .await),
                    "capture" => Ok(crate::tmux::capture_pane(
                        agentic_gpt_protocol::TmuxCapturePaneRequest {
                            target: args
                                .target
                                .ok_or_else(|| anyhow::anyhow!("target is required for capture"))?,
                            lines: args.lines.unwrap_or(160),
                        },
                    )
                    .await),
                    _ => Err(anyhow::anyhow!("invalid tmux.panes action")),
                }
            }
            "tmux.listSessions" => {
                self.require_agent(&arguments).await?;
                dispatch(self, HubCommand::TmuxListSessions { request_id }).await
            }
            "tmux.listPanes" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxListPanes {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.capturePane" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCapturePane {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.pasteText" => {
                let args: TmuxPasteArgs = from_value(arguments)?;
                Ok(crate::tmux::paste_text(
                    &self.state,
                    agentic_gpt_protocol::TmuxPasteTextRequest {
                        target: args.target,
                        text: args.text,
                        submit: args.submit,
                        need_confirm: args.need_confirm.unwrap_or(true),
                    },
                )
                .await)
            }
            "tmux.exec" => {
                let args: TmuxExecArgs = from_value(arguments)?;
                Ok(crate::tmux::exec(
                    &self.state,
                    agentic_gpt_protocol::TmuxExecRequest {
                        target: args.target,
                        program: args.program,
                        args: args.args,
                        need_confirm: args.need_confirm,
                        wait_ms: args.wait_ms.unwrap_or(300),
                        capture_lines: args.capture_lines.unwrap_or(120),
                    },
                )
                .await)
            }
            "tmux.createSession" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCreateSession {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.closeSession" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCloseSession {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "mcp.list" => {
                let args: McpListArgs = from_value(arguments)?;
                if let Some(server_id) = args.server_id {
                    let config = self.state.config.read().await.clone();
                    match crate::mcp::list_tools(
                        &self.state,
                        agentic_gpt_protocol::McpListToolsRequest {
                            agent_id: config.agent_id,
                            server_id,
                        },
                    )
                    .await
                    {
                        Ok(value) => Ok(value),
                        Err(error) => Ok(json!({
                            "error": { "code": "mcp_list_tools_failed", "message": error.to_string() }
                        })),
                    }
                } else {
                    Ok(crate::mcp::list_servers(&self.state).await)
                }
            }
            "mcp.listServers" => {
                self.require_optional_agent(&arguments).await?;
                dispatch(self, HubCommand::McpListServers { request_id }).await
            }
            "mcp.listTools" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::McpListTools {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "mcp.callTool" => {
                let args: McpCallArgs = from_value(arguments)?;
                let group = match normalize_stdio_group(args.group) {
                    Ok(group) => group,
                    Err(error) => return Ok(error),
                };
                let config = self.state.config.read().await.clone();
                let request_source = self.ingress.source("mcp.callTool");
                match crate::mcp::call_tool(
                    &self.state,
                    agentic_gpt_protocol::McpCallToolRequest {
                        agent_id: config.agent_id,
                        group,
                        server_id: args.server_id,
                        tool_name: args.tool_name,
                        arguments: args.arguments,
                        wait_seconds: args.wait_seconds,
                        timeout_seconds: args.timeout_seconds,
                    },
                    &request_source,
                    Some(managed_terminal_event_hook(
                        self.state.runtime.profile,
                        request_source.clone(),
                        terminal_tracker,
                    )),
                )
                .await
                {
                    Ok(value) => slim_mcp_response(value),
                    Err(error) => Ok(structured_error_from_reason(
                        "mcp_call_tool_failed",
                        error.to_string(),
                    )),
                }
            }
            "mcp.batch" => {
                let args: McpBatchArgs = from_value(arguments)?;
                let group = match normalize_stdio_group(args.group) {
                    Ok(group) => group,
                    Err(error) => return Ok(error),
                };
                let config = self.state.config.read().await.clone();
                let request_source = self.ingress.source("mcp.batch");
                match crate::mcp::batch_slim(
                    &self.state,
                    agentic_gpt_protocol::McpBatchRequest {
                        agent_id: config.agent_id,
                        group,
                        calls: args
                            .calls
                            .into_iter()
                            .map(|call| agentic_gpt_protocol::McpBatchCall {
                                id: None,
                                server_id: call.server_id,
                                tool_name: call.tool_name,
                                arguments: call.arguments,
                            })
                            .collect(),
                        mode: args.mode.unwrap_or_default(),
                        fail_fast: args.fail_fast,
                        wait_seconds: args.wait_seconds,
                        timeout_seconds: args.timeout_seconds,
                    },
                    &request_source,
                    Some(managed_terminal_event_hook(
                        self.state.runtime.profile,
                        request_source.clone(),
                        terminal_tracker,
                    )),
                )
                .await
                {
                    Ok(value) => slim_mcp_batch_response(value),
                    Err(error) => Ok(structured_error_from_reason(
                        "mcp_batch_failed",
                        error.to_string(),
                    )),
                }
            }
            "bootstrap" => {
                let _: EmptyArgs = from_value(arguments)?;
                dispatch(self, HubCommand::Bootstrap { request_id }).await
            }
            "bootstrap.read" => {
                let args: BootstrapReadArgs = from_value(arguments)?;
                dispatch(
                    self,
                    HubCommand::BootstrapRead {
                        request_id,
                        payload: agentic_gpt_protocol::BootstrapReadRequest { id: args.id },
                    },
                )
                .await
            }
            "skills.list" => self.dispatch_skills_list(arguments).await,
            "skills.read" => {
                let args: SkillReadArgs = from_value(arguments)?;
                dispatch(
                    self,
                    HubCommand::SkillsRead {
                        request_id,
                        payload: agentic_gpt_protocol::SkillReadRequest {
                            id: args.id,
                            path: args.path,
                        },
                    },
                )
                .await
            }
            "skills.search" => {
                dispatch(
                    self,
                    HubCommand::SkillsSearch {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.active" => dispatch(self, HubCommand::SkillsActive { request_id }).await,
            "skills.setActive" => {
                let args: SkillSetActiveArgs = from_value(arguments)?;
                let request = agentic_gpt_protocol::SkillActivationRequest { id: args.id };
                let result = if args.active {
                    crate::skills::activate(&self.state, request).await
                } else {
                    crate::skills::deactivate(&self.state, request).await
                };
                map_result_value(result, "skills_set_active_failed")
            }
            "skills.activate" => {
                dispatch(
                    self,
                    HubCommand::SkillsActivate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.deactivate" => {
                dispatch(
                    self,
                    HubCommand::SkillsDeactivate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.install" => {
                let args: SkillInstallArgs = from_value(arguments)?;
                dispatch(
                    self,
                    HubCommand::SkillsInstall {
                        request_id,
                        payload: agentic_gpt_protocol::SkillInstallRequest {
                            id: args.id,
                            source: args.source,
                            replace_existing: args.replace_existing,
                            activate_after_install: args.activate_after_install,
                            idempotency_key: args.idempotency_key,
                        },
                    },
                )
                .await
            }
            "skills.install.get" => {
                let args: SkillInstallGetArgs = from_value(arguments)?;
                dispatch(
                    self,
                    HubCommand::SkillsInstallGet {
                        request_id,
                        payload: agentic_gpt_protocol::SkillInstallGetRequest {
                            install_id: args.install_id,
                            wait_seconds: args.wait_seconds,
                        },
                    },
                )
                .await
            }
            "skills.install.cancel" => {
                let args: SkillInstallCancelArgs = from_value(arguments)?;
                dispatch(
                    self,
                    HubCommand::SkillsInstallCancel {
                        request_id,
                        payload: agentic_gpt_protocol::SkillInstallCancelRequest {
                            install_id: args.install_id,
                        },
                    },
                )
                .await
            }
            "skills.run" => self.dispatch_skill_run(arguments, terminal_tracker).await,
            "room.notebook.append" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookAppend {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.recent" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookRecent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.selectExact" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookSelectExact {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.search" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookSearch {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.current" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookCurrent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.update" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookUpdate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.remove" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookRemove {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.append" => {
                dispatch(
                    self,
                    HubCommand::RoomDiaryAppend {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.recent" => {
                dispatch(
                    self,
                    HubCommand::RoomDiaryRecent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.selectExact" => {
                dispatch(
                    self,
                    HubCommand::RoomDiarySelectExact {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            _ => Err(anyhow::anyhow!("unknown agent tool: {name}")),
        }
    }

    async fn dispatch_process_exec(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: ProcessExecArgs = from_value(arguments)?;
        let group = match normalize_stdio_group(args.group) {
            Ok(group) => group,
            Err(error) => return Ok(error),
        };
        let config = self.state.config.read().await.clone();
        let request_source = self.ingress.source("process.exec");
        let terminal_event_hook = managed_terminal_event_hook(
            self.state.runtime.profile,
            request_source.clone(),
            terminal_tracker,
        );
        let response = crate::jobs::start_and_wait_process(
            self.state.clone(),
            ExecRequest {
                agent_id: config.agent_id,
                group,
                program: args.program,
                args: args.args,
                need_confirm: args.need_confirm,
                confirm_method: None,
                working_directory: args.working_directory,
                wait_seconds: args.wait_seconds,
            },
            crate::jobs::ManagedJobOptions {
                terminal_event_hook: Some(terminal_event_hook),
                ..crate::jobs::ManagedJobOptions::for_source(request_source)
            },
        )
        .await;
        slim_process_response(serde_json::to_value(response)?)
    }

    async fn dispatch_job_get(&self, arguments: Value) -> Result<Value> {
        let args: JobGetArgs = from_value(arguments)?;
        let wait_seconds = args.wait_seconds.unwrap_or(0).min(30);
        match crate::jobs::get_job_detail(&self.state, &args.job_id, wait_seconds).await {
            Ok(job) if args.wait_only && wait_seconds > 0 && !job.job.state.is_terminal() => {
                let elapsed_ms = elapsed_ms(&job.job);
                Ok(serde_json::to_value(JobWaitResponse {
                    job_id: job.job.job_id.clone(),
                    state: job.job.state,
                    elapsed_ms,
                })?)
            }
            Ok(job) => slim_job_detail_response(job, true),
            Err(reason) => Ok(job_error(reason)),
        }
    }

    async fn dispatch_process_batch(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: ProcessBatchArgs = from_value(arguments)?;
        let group = match normalize_stdio_group(args.group) {
            Ok(group) => group,
            Err(error) => return Ok(error),
        };
        let config = self.state.config.read().await.clone();
        let request_source = self.ingress.source("process.batch");
        let terminal_event_hook = managed_terminal_event_hook(
            self.state.runtime.profile,
            request_source.clone(),
            terminal_tracker,
        );
        let request = BatchExecRequest {
            agent_id: config.agent_id,
            group,
            elements: args
                .elements
                .into_iter()
                .map(|element| ExecElement {
                    program: element.program,
                    args: element.args,
                    working_directory: element.working_directory,
                })
                .collect(),
            need_confirm: args.need_confirm,
            confirm_method: None,
            working_directory: args.working_directory,
            wait_seconds: args.wait_seconds,
        };
        match crate::jobs::start_process_batch(
            self.state.clone(),
            request,
            request_source,
            Some(terminal_event_hook),
        )
        .await
        {
            Ok(response) => slim_process_batch_response(response),
            Err(reason) => Ok(structured_error_value("process_batch_rejected", reason)),
        }
    }

    async fn dispatch_job_cancel(&self, arguments: Value) -> Result<Value> {
        let args: JobCancelArgs = from_value(arguments)?;
        match crate::jobs::cancel_job(&self.state, &args.job_id).await {
            Ok(job) => slim_cancel_response(job),
            Err(reason) => Ok(job_error(reason)),
        }
    }

    async fn dispatch_job_list(&self, arguments: Value) -> Result<Value> {
        let args: JobListArgs = from_value(arguments)?;
        let group = match normalize_stdio_group(args.group) {
            Ok(group) => group,
            Err(error) => return Ok(error),
        };
        match crate::jobs::list_jobs_page(
            &self.state,
            JobListRequest {
                group,
                kind: args.kind,
                state: args.state,
                limit: args.limit,
                cursor: args.cursor,
            },
        )
        .await
        {
            Ok(page) => slim_job_list_response(page),
            Err(reason) => Ok(structured_error_from_reason("job_list_failed", reason)),
        }
    }

    async fn dispatch_skill_run(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: SkillRunArgs = from_value(arguments)?;
        let group = match normalize_stdio_group(args.group) {
            Ok(group) => group,
            Err(error) => return Ok(error),
        };
        let request = agentic_gpt_protocol::SkillRunRequest {
            id: args.id,
            path: args.path,
            group,
            args: args.args,
            working_directory: args.working_directory,
            wait_seconds: args.wait_seconds,
        };
        let program = match crate::skills::resolve_run_program(&self.state, &request).await {
            Ok(program) => program,
            Err(error) => return Ok(crate::hub::skill_run_command_error(error)),
        };
        let config = self.state.config.read().await.clone();
        if let Some(working_directory) = request.working_directory.as_deref() {
            if let Err(reason) =
                crate::exec::resolve_working_directory(&config, Some(working_directory))
            {
                return Ok(json!({
                    "error": { "code": "invalid_working_directory", "message": reason }
                }));
            }
        }
        let wait_seconds = request.effective_wait_seconds();
        let request_source = self.ingress.source("skills.run");
        let info = crate::jobs::start_skill_job_with_hook_and_source(
            self.state.clone(),
            ExecRequest {
                agent_id: config.agent_id,
                group: request.group.clone(),
                program: program.to_string_lossy().to_string(),
                args: request.args.unwrap_or_default(),
                need_confirm: false,
                confirm_method: None,
                working_directory: request.working_directory,
                wait_seconds: Some(wait_seconds),
            },
            &request.id,
            &request.path,
            &request_source,
            Some(managed_terminal_event_hook(
                self.state.runtime.profile,
                request_source.clone(),
                terminal_tracker,
            )),
        )
        .await;
        let info = crate::jobs::wait_for_job(&self.state, info, wait_seconds).await;
        slim_process_response(serde_json::to_value(crate::jobs::response(
            info.clone(),
            info.state.is_terminal(),
        ))?)
    }

    async fn dispatch_skills_list(&self, arguments: Value) -> Result<Value> {
        let args: SkillListArgs = from_value(arguments)?;
        let active = match crate::skills::active(&self.state).await {
            Ok(active) => active,
            Err(error) => {
                return Ok(json!({
                    "error": { "code": "skills_list_failed", "message": error.to_string() }
                }))
            }
        };
        let mut warnings = active.warnings.clone();
        warnings.extend(
            active
                .active_skills
                .iter()
                .filter(|skill| skill.stale)
                .map(|skill| format!("active_skill_missing:{}", skill.id)),
        );
        let mut skills = if args
            .query
            .as_deref()
            .is_some_and(|query| !query.trim().is_empty())
        {
            match crate::skills::search(
                &self.state,
                agentic_gpt_protocol::SkillSearchRequest {
                    query: args.query.clone().unwrap_or_default(),
                    limit: args.limit,
                },
            )
            .await
            {
                Ok(response) => {
                    warnings.extend(response.warnings);
                    response.skills
                }
                Err(error) => {
                    return Ok(json!({
                        "error": { "code": "skills_list_failed", "message": error.to_string() }
                    }));
                }
            }
        } else {
            match crate::skills::list(&self.state).await {
                Ok(response) => {
                    warnings.extend(response.warnings);
                    response.skills
                }
                Err(error) => {
                    return Ok(json!({
                        "error": { "code": "skills_list_failed", "message": error.to_string() }
                    }));
                }
            }
        };
        if args.active_only {
            skills.retain(|skill| skill.active);
        }
        if args
            .query
            .as_deref()
            .is_none_or(|query| query.trim().is_empty())
        {
            if let Some(limit) = args.limit {
                skills.truncate(limit.clamp(1, 100));
            }
        }
        let mut response = json!({"skills": skills});
        if !warnings.is_empty() {
            response["warnings"] = json!(warnings);
        }
        Ok(response)
    }

    async fn require_agent(&self, arguments: &Value) -> Result<()> {
        let supplied = arguments
            .get("agentId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("agentId is required"))?;
        let expected = self.state.config.read().await.agent_id.clone();
        if supplied != expected {
            return Err(anyhow::anyhow!("agentId does not identify this worker"));
        }
        Ok(())
    }

    async fn require_optional_agent(&self, arguments: &Value) -> Result<()> {
        if arguments.get("agentId").is_some() {
            self.require_agent(arguments).await?;
        }
        Ok(())
    }
}

impl ServerHandler for AgentMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = self.call(request).await?;
        let is_error = value.get("error").is_some();
        let result = if is_error {
            CallToolResult::structured_error(value)
        } else {
            CallToolResult::structured(value)
        };
        Ok(result)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: (*self.tools).clone(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "agentic-gpt-local-agent",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessExecArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobGetArgs {
    job_id: String,
    #[serde(default)]
    wait_seconds: Option<u64>,
    #[serde(default)]
    wait_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobCancelArgs {
    job_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobListArgs {
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    kind: Option<JobKind>,
    #[serde(default)]
    state: Option<JobState>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchElementArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessBatchArgs {
    elements: Vec<BatchElementArgs>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileReadArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    metadata: Option<bool>,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
    #[serde(default)]
    requests: Option<Vec<FileReadRequestArgs>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileReadRequestArgs {
    path: String,
    #[serde(default)]
    metadata: bool,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileSearchArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    include: Option<Vec<String>>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
    #[serde(default)]
    context_lines: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    respect_gitignore: Option<bool>,
    #[serde(default)]
    requests: Option<Vec<FileSearchRequestArgs>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileSearchRequestArgs {
    path: String,
    query: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    context_lines: usize,
    #[serde(default = "default_max_search_results")]
    max_results: usize,
    #[serde(default)]
    hidden: bool,
    #[serde(default = "default_true")]
    respect_gitignore: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileEditArgs {
    patch: String,
    #[serde(default)]
    need_confirm: bool,
}

fn default_max_search_results() -> usize {
    50
}

fn default_true() -> bool {
    true
}

impl From<FileReadRequestArgs> for crate::file_ops::ReadRequest {
    fn from(value: FileReadRequestArgs) -> Self {
        Self {
            path: value.path,
            metadata: value.metadata,
            start_line: value.start_line,
            end_line: value.end_line,
        }
    }
}

impl From<FileSearchRequestArgs> for crate::file_ops::SearchRequest {
    fn from(value: FileSearchRequestArgs) -> Self {
        Self {
            path: value.path,
            query: value.query,
            mode: value.mode,
            case_sensitive: value.case_sensitive,
            include: value.include,
            exclude: value.exclude,
            context_lines: value.context_lines,
            max_results: value.max_results,
            hidden: value.hidden,
            respect_gitignore: value.respect_gitignore,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct McpListArgs {
    #[serde(default)]
    server_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct McpCallArgs {
    #[serde(default)]
    group: Option<String>,
    server_id: String,
    tool_name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    wait_seconds: Option<u64>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct McpBatchArgs {
    calls: Vec<McpBatchCallArgs>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    mode: Option<agentic_gpt_protocol::McpBatchMode>,
    #[serde(default)]
    fail_fast: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct McpBatchCallArgs {
    server_id: String,
    tool_name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillListArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    active_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillReadArgs {
    id: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillSetActiveArgs {
    id: String,
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillRunArgs {
    id: String,
    path: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillInstallArgs {
    id: String,
    source: agentic_gpt_protocol::SkillInstallSource,
    #[serde(default)]
    replace_existing: bool,
    #[serde(default)]
    activate_after_install: Option<bool>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillInstallGetArgs {
    install_id: String,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillInstallCancelArgs {
    install_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapReadArgs {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TmuxSessionsArgs {
    action: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TmuxPanesArgs {
    action: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    lines: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TmuxExecArgs {
    target: String,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_ms: Option<u64>,
    #[serde(default)]
    capture_lines: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TmuxPasteArgs {
    target: String,
    text: String,
    #[serde(default)]
    submit: bool,
    #[serde(default)]
    need_confirm: Option<bool>,
}

async fn dispatch(server: &AgentMcpServer, command: HubCommand) -> Result<Value> {
    local_service::dispatch(server.state.clone(), command).await
}

fn job_error(reason: String) -> Value {
    json!({"error": {"code": reason, "message": reason}})
}

fn structured_error_value(default_code: &str, message: impl Into<String>) -> Value {
    json!({
        "error": {
            "code": default_code,
            "message": message.into()
        }
    })
}

fn structured_error_from_reason(default_code: &str, message: impl Into<String>) -> Value {
    let message = message.into();
    let mut error = rejection_error(&message);
    if error.code == "job_rejected" {
        error.code = default_code.to_string();
    }
    json!({"error": error})
}

fn normalize_stdio_group(group: Option<String>) -> std::result::Result<Option<String>, Value> {
    normalize_job_group(group.as_deref()).map_err(|error| {
        json!({
            "error": {
                "code": error.code(),
                "message": error.message()
            }
        })
    })
}

fn elapsed_ms(info: &JobInfo) -> u64 {
    info.started_at
        .map(|started_at| (Utc::now() - started_at).num_milliseconds().max(0) as u64)
        .unwrap_or(0)
}

fn duration_ms(info: &JobInfo) -> Option<u64> {
    info.started_at.map(|started_at| {
        let finished_at = info.finished_at.unwrap_or(info.updated_at);
        (finished_at - started_at).num_milliseconds().max(0) as u64
    })
}

fn rejection_error(reason: &str) -> JobError {
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

fn slim_job_detail_response(detail: JobDetail, include_identity: bool) -> Result<Value> {
    Ok(serde_json::to_value(job_tool_response(
        &detail,
        include_identity,
        false,
    ))?)
}

fn slim_process_response(value: Value) -> Result<Value> {
    let response: JobResponse = serde_json::from_value(value)?;
    slim_job_detail_response(response.detail, false)
}

fn slim_mcp_response(value: Value) -> Result<Value> {
    let response: JobResponse = serde_json::from_value(value)?;
    slim_job_detail_response(response.detail, false)
}

fn slim_process_batch_response(response: JobBatchResponse) -> Result<Value> {
    let jobs = response
        .jobs
        .into_iter()
        .map(|job| {
            job_tool_response(
                &JobDetail {
                    job,
                    detail_available: true,
                    result: None,
                    error: None,
                    result_truncated: false,
                    result_bytes: None,
                    result_sha256: None,
                    result_preview: None,
                },
                false,
                false,
            )
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

fn slim_job_list_response(page: crate::job_history::JobHistoryPage) -> Result<Value> {
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

fn slim_cancel_response(detail: JobDetail) -> Result<Value> {
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
    Ok(serde_json::to_value(JobCancelResponse {
        job_id: detail.job.job_id,
        state: detail.job.state,
        cancel_outcome,
        termination_evidence: detail
            .job
            .termination_evidence
            .unwrap_or_else(|| "unknown".to_string()),
        error,
    })?)
}

fn slim_mcp_batch_response(value: Value) -> Result<Value> {
    let response: McpBatchResponse = serde_json::from_value(value)?;
    let mut slim = McpBatchToolResponse {
        status: response.status,
        error: response.error,
        results: response
            .results
            .into_iter()
            .map(|child| McpBatchToolChildResponse {
                job: job_tool_response(&child.detail, false, child.result_omitted),
            })
            .collect(),
    };
    apply_slim_mcp_batch_budget(&mut slim)?;
    Ok(serde_json::to_value(slim)?)
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

fn from_value<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn validate_stdio_arguments(name: &str, arguments: &Value) -> Result<()> {
    let object = arguments
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("tool arguments must be an object"))?;
    let allowed = properties_for(name);
    if let Some(unknown) = object.keys().find(|key| !allowed.contains_key(*key)) {
        return Err(anyhow::anyhow!("unknown tool argument: {unknown}"));
    }
    match name {
        "file.read" => {
            let args: FileReadArgs = from_value(arguments.clone())?;
            validate_file_read_args(&args)?;
        }
        "file.search" => {
            let args: FileSearchArgs = from_value(arguments.clone())?;
            validate_file_search_args(&args)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_file_read_args(args: &FileReadArgs) -> Result<()> {
    match (&args.path, &args.requests) {
        (Some(_), None) => Ok(()),
        (None, Some(requests))
            if !requests.is_empty()
                && requests.len() <= crate::file_ops::MAX_BATCH_OPERATIONS
                && args.metadata.is_none()
                && args.start_line.is_none()
                && args.end_line.is_none() =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "file.read single fields and requests are mutually exclusive"
        )),
        (None, Some(_)) => Err(anyhow::anyhow!(
            "file.read requests must contain 1..32 items"
        )),
        (None, None) => Err(anyhow::anyhow!("file.read requires path or requests")),
    }
}

fn validate_file_search_args(args: &FileSearchArgs) -> Result<()> {
    let batch = args.requests.is_some();
    let has_flat = args.path.is_some()
        || args.query.is_some()
        || args.mode.is_some()
        || args.case_sensitive.is_some()
        || args.include.is_some()
        || args.exclude.is_some()
        || args.context_lines.is_some()
        || args.max_results.is_some()
        || args.hidden.is_some()
        || args.respect_gitignore.is_some();
    if batch {
        let requests = args.requests.as_ref().expect("batch request present");
        if requests.is_empty() || requests.len() > crate::file_ops::MAX_BATCH_OPERATIONS {
            return Err(anyhow::anyhow!(
                "file.search requests must contain 1..32 items"
            ));
        }
        if has_flat {
            return Err(anyhow::anyhow!(
                "file.search single fields and requests are mutually exclusive"
            ));
        }
        return Ok(());
    }
    if args.path.is_none() || args.query.is_none() {
        return Err(anyhow::anyhow!(
            "file.search requires path and query or requests"
        ));
    }
    if args
        .mode
        .as_deref()
        .is_some_and(|mode| !matches!(mode, "literal" | "regex"))
    {
        return Err(anyhow::anyhow!("file search mode must be literal or regex"));
    }
    Ok(())
}

fn validate_tmux_sessions_args(args: &TmuxSessionsArgs) -> Result<()> {
    match args.action.as_str() {
        "list" if args.name.is_none() && args.cwd.is_none() && args.need_confirm.is_none() => {
            Ok(())
        }
        "create" if args.name.is_some() && args.cwd.is_some() && args.need_confirm.is_none() => {
            Ok(())
        }
        "close" if args.name.is_some() && args.cwd.is_none() => Ok(()),
        "list" => Err(anyhow::anyhow!("tmux.sessions list accepts only action")),
        "create" => Err(anyhow::anyhow!(
            "tmux.sessions create accepts action, name, and cwd"
        )),
        "close" => Err(anyhow::anyhow!(
            "tmux.sessions close accepts action, name, and needConfirm"
        )),
        _ => Err(anyhow::anyhow!("invalid tmux.sessions action")),
    }
}

fn validate_tmux_panes_args(args: &TmuxPanesArgs) -> Result<()> {
    match args.action.as_str() {
        "list" if args.target.is_none() && args.lines.is_none() => Ok(()),
        "capture" if args.session.is_none() && args.target.is_some() => Ok(()),
        "list" => Err(anyhow::anyhow!(
            "tmux.panes list accepts action and session"
        )),
        "capture" => Err(anyhow::anyhow!(
            "tmux.panes capture accepts action, target, and lines"
        )),
        _ => Err(anyhow::anyhow!("invalid tmux.panes action")),
    }
}

fn map_result_value<T: serde::Serialize>(result: Result<T>, code: &str) -> Result<Value> {
    match result {
        Ok(value) => Ok(serde_json::to_value(value)?),
        Err(error) => Ok(json!({
            "error": { "code": code, "message": error.to_string() }
        })),
    }
}

fn request_id() -> String {
    task_id("req")
}

fn task_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn value_has_active_job(value: &Value) -> bool {
    job_values(value).any(|job| {
        job.get("state")
            .and_then(Value::as_str)
            .is_some_and(is_active_job_state)
    })
}

fn value_has_terminal_failure(value: &Value) -> bool {
    value.get("error").is_some()
        || job_values(value).any(|job| {
            job.get("state")
                .and_then(Value::as_str)
                .is_some_and(is_failure_job_state)
        })
}

fn human_failure_reason(value: &Value, job: Option<&JobInfo>) -> Option<String> {
    job.and_then(|job| job.reject_reason.clone()).or_else(|| {
        job_values(value).find_map(|job| {
            job.get("rejectReason")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    })
}

fn job_values(value: &Value) -> impl Iterator<Item = &Value> {
    let wrapped = value.get("job").into_iter();
    let direct = value
        .get("jobId")
        .and_then(|_| value.get("state"))
        .map(|_| value)
        .into_iter();
    let batch = value
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    wrapped.chain(direct).chain(batch)
}

fn is_active_job_state(state: &str) -> bool {
    matches!(
        state,
        "queued" | "waiting_confirmation" | "starting" | "running" | "cancel_requested"
    )
}

fn is_failure_job_state(state: &str) -> bool {
    matches!(
        state,
        "failed" | "rejected" | "cancelled" | "timed_out" | "unknown_after_restart"
    )
}

fn bounded_error_code(value: &str) -> String {
    let candidate = value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .find(|part| !part.is_empty())
        .unwrap_or("tool_error");
    candidate.chars().take(64).collect()
}

fn managed_terminal_event_hook(
    profile: CapabilityProfile,
    source: impl Into<String>,
    tracker: Arc<HumanTerminalTracker>,
) -> crate::jobs::TerminalEventHook {
    let source = source.into();
    let profile = profile.label();
    Arc::new(move |job| {
        tracker.record(profile, &source, job);
    })
}

fn managed_terminal_event_message(profile: &str, source: &str, job: &JobInfo) -> String {
    let duration_ms = job
        .started_at
        .map(|started_at| (job.updated_at - started_at).num_milliseconds().max(0))
        .unwrap_or(0);
    let mut message = format!(
        "managed_job; source={source}; profile={profile}; status={}; job={}; durationMs={duration_ms}",
        job.state,
        crate::utils::compact_id(&job.job_id)
    );
    if let Some(exit_code) = job.exit_code {
        message.push_str(&format!("; exitCode={exit_code}"));
    }
    if let Some(reason) = job.reject_reason.as_deref() {
        message.push_str(&format!("; errorCode={}", bounded_error_code(reason)));
    }
    message
}

fn tool_descriptor(name: &str) -> Tool {
    let input_schema = tool_input_schema(name);
    let annotations = ToolAnnotations::new()
        .read_only(tool_is_read_only(name))
        .destructive(tool_is_destructive(name))
        .open_world(tool_is_open_world(name));
    Tool::new(name.to_string(), tool_description(name), input_schema)
        .with_annotations(annotations)
        .with_raw_output_schema(Arc::new(output_schema()))
        .with_meta(Meta(Map::from_iter([(
            "surface".to_string(),
            Value::String("agent-local".to_string()),
        )])))
}

fn tool_input_schema(name: &str) -> Map<String, Value> {
    let (properties, required) = tool_schema(name);
    let mut result = schema(properties, required);
    if matches!(name, "file.read" | "file.search") {
        let (flat_required, flat_fields) = if name == "file.search" {
            (
                vec!["path", "query"],
                vec![
                    "path",
                    "query",
                    "mode",
                    "caseSensitive",
                    "include",
                    "exclude",
                    "contextLines",
                    "maxResults",
                    "hidden",
                    "respectGitignore",
                ],
            )
        } else {
            (
                vec!["path"],
                vec!["path", "metadata", "startLine", "endLine"],
            )
        };
        let flat_absent_requests = json!({
            "required": flat_required,
            "not": {"required": ["requests"]}
        });
        let batch_absent_flat = json!({
            "required": ["requests"],
            "not": {"anyOf": flat_fields.iter().map(|field| json!({"required": [field]})).collect::<Vec<_>>()}
        });
        result.insert(
            "oneOf".to_string(),
            json!([flat_absent_requests, batch_absent_flat]),
        );
    }
    result
}

fn tool_schema(name: &str) -> (Map<String, Value>, &'static [&'static str]) {
    let required: &'static [&'static str] = match name {
        "process.exec" => &["program"],
        "process.batch" => &["elements"],
        "job.get" | "job.cancel" => &["jobId"],
        "file.read" | "file.search" => &[],
        "file.edit" => &["patch"],
        "mcp.callTool" => &["serverId", "toolName"],
        "mcp.batch" => &["calls"],
        "skills.setActive" => &["id", "active"],
        "tmux.sessions" | "tmux.panes" => &["action"],
        "tmux.exec" => &["target", "program"],
        "tmux.pasteText" => &["target", "text"],
        "tmux.listPanes" => &["agentId"],
        "tmux.capturePane" => &["agentId", "target"],
        "tmux.createSession" => &["agentId", "name", "cwd"],
        "tmux.closeSession" => &["agentId", "name"],
        "mcp.listTools" => &["agentId", "serverId"],
        "bootstrap.read" => &["id"],
        "skills.read" => &["id"],
        "skills.search" => &["query"],
        "skills.activate" | "skills.deactivate" => &["id"],
        "skills.install" => &["id", "source"],
        "skills.install.get" | "skills.install.cancel" => &["installId"],
        "skills.run" => &["id", "path"],
        "room.notebook.append" => &["scope", "content"],
        "room.notebook.selectExact" => &["date"],
        "room.notebook.search" => &["query"],
        "room.notebook.current" => &["scope"],
        "room.notebook.update" | "room.notebook.remove" => &["id"],
        "room.diary.append" => &["entry"],
        "room.diary.selectExact" => &["date"],
        _ => &[],
    };
    (properties_for(name), required)
}

fn properties_for(name: &str) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut add = |key: &str, value: Value| {
        properties.insert(key.to_string(), value);
    };
    let string = |description: &str| json!({"type": "string", "description": description});
    let number = |description: &str| json!({"type": "integer", "description": description});
    let boolean = |description: &str| json!({"type": "boolean", "description": description});
    let strings = |description: &str| json!({"type": "array", "items": {"type": "string"}, "description": description});
    if matches!(
        name,
        "tmux.listPanes"
            | "tmux.capturePane"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.listTools"
    ) {
        add("agentId", string("Target local agent id."));
    }
    match name {
        "file.read" => {
            add(
                "path",
                string("File or directory path; resolved and checked by path policy."),
            );
            add(
                "metadata",
                boolean("Include file metadata alongside content; default false."),
            );
            add("startLine", number("Inclusive start line."));
            add("endLine", number("Inclusive end line."));
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("File or directory path; resolved and checked by path policy."),
                        "metadata":boolean("Include file metadata alongside content; default false."),
                        "startLine":number("Inclusive start line."), "endLine":number("Inclusive end line.")
                    },"required":["path"]},
                    "description":"Ordered batch reads; mutually exclusive with flat single-read fields. Maximum 32."
                }),
            );
        }
        "file.search" => {
            add(
                "path",
                string("File or directory root; resolved and checked by path policy."),
            );
            add("query", string("Literal or regex query."));
            add(
                "mode",
                json!({"type":"string","enum":["literal","regex"],"default":"literal"}),
            );
            add("caseSensitive", boolean("Case-sensitive; default true."));
            add("include", strings("Include globs; max 16."));
            add("exclude", strings("Exclude globs; max 16."));
            add(
                "contextLines",
                json!({
                    "type":"integer",
                    "minimum":0,
                    "description":"Requested context lines; values above the live configured maximum are clipped and reported.",
                    "default":0
                }),
            );
            add("maxResults", number("Maximum matches, max 200."));
            add("hidden", boolean("Include hidden files; default false."));
            add(
                "respectGitignore",
                boolean("Honor Git ignore rules inside repositories; default true."),
            );
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("File or directory root; resolved and checked by path policy."),
                        "query":string("Literal or regex query."),
                        "mode":{"type":"string","enum":["literal","regex"],"default":"literal"},
                        "caseSensitive":{"type":"boolean","default":true},
                        "include":strings("Include globs; max 16."), "exclude":strings("Exclude globs; max 16."),
                        "contextLines":{"type":"integer","minimum":0,"default":0},
                        "maxResults":{"type":"integer","maximum":200,"default":50},
                        "hidden":{"type":"boolean","default":false},
                        "respectGitignore":{"type":"boolean","default":true}
                    },"required":["path","query"]},
                    "description":"Ordered batch searches; mutually exclusive with flat single-search fields. Maximum 32."
                }),
            );
        }
        "file.edit" => {
            add("patch", string(PATCH_SCHEMA_DESCRIPTION));
            add(
                "needConfirm",
                boolean("Request one confirmation before an effective mutation; default false."),
            );
        }
        "mcp.list" => add("serverId", string("Optional configured MCP server id.")),
        "process.exec" => {
            add("program", string("Executable name or path."));
            add("args", strings("Direct argument vector."));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional human-readable workstream key; trimmed, control-free, and at most 32 Unicode characters."
                }),
            );
            add(
                "needConfirm",
                boolean("Request confirmation before execution."),
            );
            add("workingDirectory", string("Process working directory."));
            add(
                "waitSeconds",
                number("Bounded inline wait in seconds, capped at 30."),
            );
        }
        "process.batch" => {
            add(
                "elements",
                json!({
                    "type": "array",
                    "items": {"type": "object", "properties": {
                        "program": string("Executable name or path."),
                        "args": strings("Direct argument vector."),
                        "workingDirectory": string("Per-element working directory.")
                    }, "required": ["program"], "additionalProperties": false}
                }),
            );
            add(
                "needConfirm",
                boolean("Request confirmation for the batch."),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional human-readable workstream key inherited by every child Job."
                }),
            );
            add(
                "workingDirectory",
                string("Default process working directory."),
            );
            add(
                "waitSeconds",
                number("Bounded inline wait in seconds, capped at 30."),
            );
        }
        "job.get" => {
            add("jobId", string("Managed Job id."));
            add(
                "waitSeconds",
                number("Bounded wait in seconds, capped at 30."),
            );
            add(
                "waitOnly",
                json!({
                    "type": "boolean",
                    "default": false,
                    "description": "While waiting, return only jobId/state/elapsedMs if the Job remains active; terminal completion returns normal detail."
                }),
            );
        }
        "job.cancel" => {
            add("jobId", string("Managed Job id."));
        }
        "job.list" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Exact human-readable workstream filter."
                }),
            );
            add(
                "kind",
                json!({"type":"string","enum":["process","skill","mcp"]}),
            );
            add(
                "state",
                json!({"type":"string","enum":["queued","waiting_confirmation","starting","running","completed","failed","rejected","cancel_requested","cancelled","timed_out","detached","unknown_after_restart","skipped"]}),
            );
            add("limit", number("Maximum Jobs to return, capped at 100."));
            add(
                "cursor",
                string("Opaque cursor returned by a prior job.list response."),
            );
        }
        "tmux.sessions" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "create", "close"]}),
            );
            add("name", string("tmux session name for create or close."));
            add("cwd", string("Working directory for create."));
            add(
                "needConfirm",
                boolean("Request confirmation before closing."),
            );
        }
        "tmux.panes" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "capture"]}),
            );
            add("session", string("Optional tmux session name for list."));
            add("target", string("tmux pane target for capture."));
            add("lines", number("History lines for capture, default 160."));
        }
        "tmux.listPanes" => add("session", string("Optional tmux session name.")),
        "tmux.capturePane" => {
            add("target", string("tmux pane target."));
            add("lines", number("History lines, default 160."));
        }
        "tmux.pasteText" => {
            add("target", string("tmux pane target."));
            add("text", string("Text to paste."));
            add("submit", boolean("Append Enter after pasting."));
            add(
                "needConfirm",
                boolean("Request confirmation before writing."),
            );
        }
        "tmux.exec" => {
            add("target", string("Shell pane target."));
            add("program", string("Program or shell builtin."));
            add("args", strings("Structured argument vector."));
            add("needConfirm", boolean("Request confirmation."));
            add("waitMs", number("Post-submit wait in milliseconds."));
            add("captureLines", number("Post-submit history lines."));
        }
        "tmux.createSession" => {
            add("name", string("tmux session name."));
            add("cwd", string("Session working directory."));
        }
        "tmux.closeSession" => {
            add("name", string("tmux session name."));
            add(
                "needConfirm",
                boolean("Request confirmation before closing."),
            );
        }
        "mcp.listServers" => add("agentId", string("Optional local agent id.")),
        "mcp.listTools" => add("serverId", string("Configured MCP server id.")),
        "mcp.batch" => {
            add(
                "calls",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 16,
                    "description": "Ordered downstream MCP calls; every call is validated before capacity admission/confirmation, aggregate serialized arguments are capped at 2 MiB, and downstream side effects are not rolled back.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serverId", "toolName"],
                        "properties": {
                            "serverId": {"type": "string", "description": "Configured downstream MCP server id."},
                            "toolName": {"type": "string", "description": "Tool name returned by mcp.listTools for this server."},
                            "arguments": {
                                "type": "object",
                                "default": {},
                                "description": "Per-call arguments capped at 256 KiB serialized."
                            }
                        }
                    }
                }),
            );
            add(
                "mode",
                json!({"type":"string","enum":["parallel","sequential"],"default":"parallel","description":"Scheduling mode; sequential waits for each child to become terminal."}),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional human-readable workstream key inherited by every child Job."
                }),
            );
            add(
                "failFast",
                json!({"type":"boolean","default":false,"description":"After a hard failure, skip only children that have not started; already-started calls are never cancelled."}),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"Bounded inline wait before returning child Job envelopes; maximum 30 seconds."}),
            );
            add(
                "timeoutSeconds",
                json!({"type":"integer","minimum":1,"maximum":900,"default":300,"description":"Absolute downstream confirmation/connect/request deadline; maximum 900 seconds."}),
            );
        }
        "mcp.callTool" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional human-readable workstream key for this downstream Job."
                }),
            );
            add(
                "serverId",
                string("Configured downstream MCP server id; discover valid values with mcp.list."),
            );
            add(
                "toolName",
                string("Downstream MCP tool name returned by mcp.list for serverId."),
            );
            add(
                "arguments",
                json!({
                    "type": "object",
                    "description": "Downstream tool arguments as a JSON object; maximum serialized size 256 KiB.",
                    "default": {}
                }),
            );
            add(
                "waitSeconds",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 30,
                    "default": 5,
                    "description": "Bounded inline wait before returning the Job envelope."
                }),
            );
            add(
                "timeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 900,
                    "default": 300,
                    "description": "Absolute downstream execution deadline."
                }),
            );
        }
        "bootstrap.read" => add("id", string("Guide id returned by bootstrap.")),
        "skills.list" => {
            add("query", string("Optional case-insensitive skill query."));
            add("limit", number("Maximum skills returned."));
            add(
                "activeOnly",
                boolean("Return only valid active skill summaries."),
            );
        }
        "skills.setActive" => {
            add("id", string("Skill id."));
            add("active", boolean("Whether the skill should be active."));
        }
        "skills.read" => {
            add("id", string("Skill id."));
            add("path", string("Optional package-relative resource path."));
        }
        "skills.search" => {
            add("query", string("Case-insensitive search query."));
            add("limit", number("Maximum skills returned."));
        }
        "skills.activate" | "skills.deactivate" => add("id", string("Skill id.")),
        "skills.install" => {
            add("id", string("Target skill id."));
            add(
                "source",
                json!({"type": "object", "description": "GitHub, HTTPS-file, or inline source descriptor."}),
            );
            add(
                "replaceExisting",
                boolean("Archive an existing skill before replacement."),
            );
            add(
                "activateAfterInstall",
                boolean("Optional activation choice."),
            );
            add("idempotencyKey", string("Optional retry key."));
        }
        "skills.install.get" => {
            add("installId", string("Installation job id."));
            add("waitSeconds", number("Bounded status wait, capped at 30."));
        }
        "skills.install.cancel" => add("installId", string("Installation job id.")),
        "skills.run" => {
            add("id", string("Skill id."));
            add("path", string("Package-relative executable path."));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional human-readable workstream key for this skill Job."
                }),
            );
            add("args", strings("Script argument vector."));
            add("workingDirectory", string("Optional working directory."));
            add("waitSeconds", number("Bounded inline wait, capped at 30."));
        }
        "room.notebook.append" => {
            add("datetime", string("Optional ISO-8601 timestamp."));
            add("scope", string("Path-safe notebook namespace."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"], "default": "NORMAL"}),
            );
            add("abstract", string("Short summary."));
            add("content", string("Full passage content."));
            add("tags", strings("Optional labels."));
        }
        "room.notebook.recent" => {
            add("scope", string("Optional scope filter."));
            add("days", number("Calendar days to scan."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
            );
            add("limit", number("Maximum passages."));
        }
        "room.notebook.selectExact" => {
            add(
                "date",
                json!({
                    "type": "string",
                    "pattern": "^\\d{4}-\\d{2}-\\d{2}$",
                    "description": "Room-local calendar date in YYYY-MM-DD format."
                }),
            );
            add("scope", string("Optional scope filter."));
            add("limit", number("Maximum passages."));
        }
        "room.notebook.search" => {
            add("query", string("Search query."));
            add("scope", string("Optional scope filter."));
            add("limit", number("Maximum passages."));
        }
        "room.notebook.current" => add("scope", string("Notebook scope.")),
        "room.notebook.update" => {
            add("id", string("Passage id."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
            );
            add("abstract", string("Optional replacement summary."));
            add("content", string("Optional replacement content."));
            add("tags", strings("Optional replacement labels."));
        }
        "room.notebook.remove" => add("id", string("Passage id.")),
        "room.diary.append" => {
            add("tags", strings("Optional labels."));
            add("entry", string("Diary entry text."));
        }
        "room.diary.recent" => {
            add("days", number("Logical diary days to scan."));
            add("limit", number("Maximum entries."));
        }
        "room.diary.selectExact" => {
            add(
                "date",
                json!({
                    "type": "string",
                    "pattern": "^\\d{4}-\\d{2}-\\d{2}$",
                    "description": "Room-local logical diary date in YYYY-MM-DD format."
                }),
            );
            add("limit", number("Maximum entries."));
        }
        _ => {}
    }
    properties
}

fn schema(properties: Map<String, Value>, required: &[&str]) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("type".to_string(), Value::String("object".to_string()));
    result.insert("properties".to_string(), Value::Object(properties));
    result.insert(
        "required".to_string(),
        Value::Array(
            required
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
    result.insert("additionalProperties".to_string(), Value::Bool(false));
    result
}

fn output_schema() -> Map<String, Value> {
    Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("additionalProperties".to_string(), Value::Bool(true)),
    ])
}

fn tool_description(name: &str) -> String {
    match name {
        "agent.info" => "Inspect local Agent runtime, workspace policy, connectivity, capacity, and health; read-only diagnostics.".to_string(),
        "file.read" => "Read bounded UTF-8 file content and optionally metadata; read-only.".to_string(),
        "file.search" => "Search bounded workspace text in-process; read-only.".to_string(),
        "file.edit" => "Apply a Codex apply_patch patch to workspace files; mutations remain policy and confirmation controlled.".to_string(),
        "process.exec" => "Start one managed local process; use job tools for lifecycle follow-up.".to_string(),
        "process.batch" => "Start multiple managed local processes under one admission boundary; started side effects are not rolled back.".to_string(),
        "job.get" => "Inspect or briefly wait for one managed Job; waitOnly performs status-only waiting.".to_string(),
        "job.list" => "List active or retained managed Jobs; read-only discovery.".to_string(),
        "job.cancel" => "Request cancellation of one managed Job; returned state is observed evidence, not a termination guarantee.".to_string(),
        "tmux.listSessions" => "List persistent tmux sessions; read-only.".to_string(),
        "tmux.sessions" => "List, create, or close persistent tmux sessions; close is destructive.".to_string(),
        "tmux.listPanes" => "List tmux panes; read-only.".to_string(),
        "tmux.panes" => "List panes or capture bounded pane history; read-only.".to_string(),
        "tmux.capturePane" => "Capture bounded history from one tmux pane; read-only.".to_string(),
        "tmux.pasteText" => "Paste text into a non-shell tmux pane or TUI; shell panes are rejected.".to_string(),
        "tmux.exec" => "Submit one structured command to a tmux shell pane; submission does not prove command completion.".to_string(),
        "tmux.createSession" => "Create or reuse one persistent tmux session in an allowed working directory.".to_string(),
        "tmux.closeSession" => "Close one persistent tmux session; destructive and confirmation-aware.".to_string(),
        "mcp.listServers" => "List configured downstream MCP servers; read-only discovery.".to_string(),
        "mcp.listTools" => "List tools exposed by one downstream MCP server; read-only discovery.".to_string(),
        "mcp.list" => "List downstream MCP servers or one server's tools; read-only discovery.".to_string(),
        "mcp.batch" => "Run multiple downstream MCP calls as managed Jobs under one admission boundary; downstream side effects are not rolled back.".to_string(),
        "mcp.callTool" => "Run one downstream MCP tool as a managed Job; use job tools for lifecycle follow-up.".to_string(),
        "bootstrap" => "Load Room bootstrap guidance; read-only and not a generic file reader.".to_string(),
        "bootstrap.read" => "Read one validated Room bootstrap guide; not an arbitrary path reader.".to_string(),
        "skills.list" => "List local skills with optional filtering; read-only discovery.".to_string(),
        "skills.setActive" => "Set one local skill's active state; grants no permissions and executes nothing.".to_string(),
        "skills.read" => "Read one local skill package or package resource; not a generic file reader.".to_string(),
        "skills.search" => "Search local skill metadata and content; read-only discovery.".to_string(),
        "skills.active" => "List active local skill state, including stale entries; read-only.".to_string(),
        "skills.activate" => "Mark one valid local skill active; executes nothing.".to_string(),
        "skills.deactivate" => "Remove active state for one local skill; executes nothing.".to_string(),
        "skills.install" => "Start an asynchronous local skill installation; use install get/cancel for lifecycle follow-up.".to_string(),
        "skills.install.get" => "Inspect or briefly wait for one skill installation; read-only lifecycle inspection.".to_string(),
        "skills.install.cancel" => "Request cooperative cancellation of one skill installation before commit.".to_string(),
        "skills.run" => "Run an executable from an active local skill as a managed Job.".to_string(),
        "room.notebook.append" => "Append one durable Room notebook passage; ANCHOR updates current state for its scope.".to_string(),
        "room.notebook.recent" => "Read recent Room notebook passages; read-only.".to_string(),
        "room.notebook.selectExact" => "Read Room notebook passages for one exact Room-local calendar date; read-only.".to_string(),
        "room.notebook.search" => "Search Room notebook passages by bounded substring fields; read-only.".to_string(),
        "room.notebook.current" => "Read recoverable current Room notebook state for one scope; read-only.".to_string(),
        "room.notebook.update" => "Update editable fields of one Room notebook passage; scope and datetime stay immutable.".to_string(),
        "room.notebook.remove" => "Remove one Room notebook passage; destructive.".to_string(),
        "room.diary.append" => "Append one durable Room diary entry to the current logical diary day.".to_string(),
        "room.diary.recent" => "Read recent Room diary entries; read-only.".to_string(),
        "room.diary.selectExact" => "Read Room diary entries for one exact Room-local logical date; read-only.".to_string(),
        _ => "Agentic GPT local tool.".to_string(),
    }
}

fn tool_is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "process.exec"
            | "process.batch"
            | "job.cancel"
            | "tmux.sessions"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.batch"
            | "mcp.callTool"
            | "room.notebook.append"
            | "room.notebook.update"
            | "room.notebook.remove"
            | "room.diary.append"
            | "skills.activate"
            | "skills.deactivate"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.run"
            | "file.edit"
    )
}

fn tool_is_destructive(name: &str) -> bool {
    matches!(
        name,
        "file.edit"
            | "job.cancel"
            | "tmux.sessions"
            | "tmux.closeSession"
            | "room.notebook.remove"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.setActive"
            | "skills.run"
    )
}

fn tool_is_open_world(name: &str) -> bool {
    matches!(
        name,
        "process.exec"
            | "process.batch"
            | "tmux.sessions"
            | "mcp.batch"
            | "mcp.callTool"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "skills.install"
            | "skills.run"
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use agentic_gpt_protocol::{AgentMessage, SkillActivationRequest};
    use rmcp::{model::CallToolRequestParams, ServiceExt};
    use serde::Deserialize;
    use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::{mpsc, Mutex, RwLock};

    use super::*;
    use crate::{
        config::Config, jobs::SkillLeaseManager, skill_installs::InstallManager,
        state::RuntimeModel,
    };

    #[derive(Debug, Deserialize)]
    struct ToolContractCase {
        id: String,
        tool: String,
        kind: String,
        arguments: Value,
        expect: Value,
    }

    #[test]
    fn normal_and_room_tool_sets_are_exact() {
        let normal = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let room = AgentMcpServer::new(test_state(CapabilityProfile::Room));
        let normal_names = normal
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        let room_names = room
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(normal_names.len(), 23);
        assert_eq!(room_names.len(), 35);
        assert!(!normal_names.iter().any(|name| name.starts_with("room.")));
        assert!(room_names.iter().any(|name| name == "room.diary.append"));
        assert!(room_names.iter().any(|name| name == "room.notebook.remove"));
        assert!(!normal_names
            .iter()
            .any(|name| name == "user.notify.deliver"));
        assert_eq!(normal_names, {
            let mut expected = NORMAL_TOOLS
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>();
            expected.sort();
            expected
        });
        let serialized = serde_json::to_string(&normal.tools).unwrap();
        assert!(!serialized.contains("agentId"));
        assert!(!serialized.contains("confirmMethod"));
        let removed_tool = ["file", "batch"].join(".");
        assert!(!normal_names.iter().any(|name| name == &removed_tool));
        assert!(serialized.contains("mcp.list"));
        assert!(serialized.contains("skills.setActive"));
        assert!(serialized.contains("tmux.sessions"));
        assert!(serialized.contains("tmux.panes"));
    }

    #[test]
    fn compact_tool_schema_budgets_hold() {
        let normal = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let room = AgentMcpServer::new(test_state(CapabilityProfile::Room));
        for (label, tools, max_total, max_inputs) in [
            // The frozen file schemas add bounded descriptors to the original
            // compact-surface budgets; retain explicit finite caps for the
            // resulting Normal/Room surfaces.
            ("normal", normal.tools.as_ref(), 32_000usize, 16_000usize),
            ("room", room.tools.as_ref(), 48_000usize, 24_000usize),
        ] {
            let serialized = serde_json::to_vec(tools).unwrap();
            let input_bytes = tools
                .iter()
                .map(|tool| {
                    let value = serde_json::to_value(tool).unwrap();
                    serde_json::to_vec(&value["inputSchema"]).unwrap().len()
                })
                .sum::<usize>();
            assert!(
                serialized.len() <= max_total,
                "{label} tool schemas use {} bytes, budget is {max_total}",
                serialized.len()
            );
            assert!(
                input_bytes <= max_inputs,
                "{label} input schemas use {input_bytes} bytes, budget is {max_inputs}"
            );
        }
    }

    #[test]
    fn file_surface_schema_is_exact() -> anyhow::Result<()> {
        let names = AgentMcpServer::new(test_state(CapabilityProfile::Normal))
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        let removed_tool = ["file", "batch"].join(".");
        assert!(!names.iter().any(|name| name == &removed_tool));

        let read = serde_json::to_value(tool_descriptor("file.read"))?;
        assert_eq!(read["inputSchema"]["required"], json!([]));
        assert!(read["inputSchema"]["properties"].get("requests").is_some());
        let read_one_of = read["inputSchema"]["oneOf"].as_array().unwrap();
        assert_eq!(read_one_of.len(), 2);
        assert_eq!(read_one_of[0]["required"], json!(["path"]));
        assert_eq!(read_one_of[0]["not"]["required"], json!(["requests"]));
        assert_eq!(read_one_of[1]["required"], json!(["requests"]));
        let read_batch_forbidden = read_one_of[1]["not"]["anyOf"].as_array().unwrap();
        for field in ["path", "metadata", "startLine", "endLine"] {
            assert!(read_batch_forbidden
                .iter()
                .any(|branch| branch["required"] == json!([field])));
        }

        let search = serde_json::to_value(tool_descriptor("file.search"))?;
        assert!(search["inputSchema"]["properties"]
            .get("requests")
            .is_some());
        let search_one_of = search["inputSchema"]["oneOf"].as_array().unwrap();
        assert_eq!(search_one_of[0]["required"], json!(["path", "query"]));
        assert_eq!(search_one_of[0]["not"]["required"], json!(["requests"]));
        assert_eq!(search_one_of[1]["required"], json!(["requests"]));
        for field in [
            "path",
            "query",
            "mode",
            "caseSensitive",
            "include",
            "exclude",
            "contextLines",
            "maxResults",
            "hidden",
            "respectGitignore",
        ] {
            assert!(search_one_of[1]["not"]["anyOf"]
                .as_array()
                .unwrap()
                .iter()
                .any(|branch| branch["required"] == json!([field])));
        }

        let edit = serde_json::to_value(tool_descriptor("file.edit"))?;
        let edit_fields = edit["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let expected_edit_fields = ["needConfirm", "patch"]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>();
        assert_eq!(edit_fields, expected_edit_fields);
        assert_eq!(edit["inputSchema"]["required"], json!(["patch"]));
        Ok(())
    }

    #[tokio::test]
    async fn deterministic_tool_contract_corpus_exercises_public_dispatch() -> anyhow::Result<()> {
        let cases: Vec<ToolContractCase> = serde_json::from_str(include_str!(
            "../../../tests/tool-contract-cases/cases.json"
        ))?;
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        let fixture_path = workspace.join("contract-fixture.txt");
        std::fs::write(&fixture_path, "prefix\nneedle\nsuffix\n")?;

        for case in cases {
            let descriptor = serde_json::to_value(tool_descriptor(&case.tool))?;
            if case.kind != "negative" {
                validate_stdio_arguments(&case.tool, &case.arguments).unwrap_or_else(|error| {
                    panic!("{} descriptor arguments rejected: {error}", case.id)
                });
            }
            let expected = &case.expect;

            if case.kind == "descriptor" {
                for phrase in expected["descriptionIncludes"]
                    .as_array()
                    .expect("descriptor phrases")
                {
                    let phrase = phrase.as_str().expect("descriptor phrase string");
                    assert!(
                        descriptor["description"]
                            .as_str()
                            .is_some_and(|description| description.contains(phrase)),
                        "{} missing descriptor phrase {phrase:?}",
                        case.id
                    );
                }
                for required in expected["required"].as_array().expect("required fields") {
                    assert!(
                        descriptor["inputSchema"]["required"]
                            .as_array()
                            .is_some_and(|fields| fields.contains(required)),
                        "{} missing required field {required}",
                        case.id
                    );
                }
                continue;
            }

            let arguments = case.arguments;
            if case.kind == "negative" {
                match server.dispatch(&case.tool, arguments).await {
                    Ok(value) => {
                        if let Some(code) = expected["errorCode"].as_str() {
                            assert_eq!(value["error"]["code"], code, "{} error code", case.id);
                        }
                        if let Some(message) = expected["errorIncludes"].as_str() {
                            assert!(
                                value["error"]["message"]
                                    .as_str()
                                    .is_some_and(|text| text.contains(message)),
                                "{} error message missing {message:?}: {}",
                                case.id,
                                value
                            );
                        }
                    }
                    Err(error) => {
                        let expected_text = expected["errorIncludes"]
                            .as_str()
                            .expect("negative dispatch error phrase");
                        assert!(
                            error.to_string().contains(expected_text),
                            "{} dispatch error missing {expected_text:?}: {error}",
                            case.id
                        );
                    }
                }
                continue;
            }

            let value = server
                .dispatch(&case.tool, arguments)
                .await
                .unwrap_or_else(|error| panic!("{} dispatch failed: {error}", case.id));
            if let Some(fields) = expected["resultFields"].as_array() {
                for field in fields {
                    let field = field.as_str().expect("result field string");
                    assert!(
                        value.get(field).is_some(),
                        "{} missing result field {field}",
                        case.id
                    );
                }
            }
            if let Some(status) = expected["status"].as_str() {
                assert_eq!(value["status"], status, "{} status", case.id);
            }
            if expected["noRevision"].as_bool() == Some(true) {
                assert!(
                    !serde_json::to_string(&value)?.contains("revision"),
                    "{} unexpectedly exposes a revision",
                    case.id
                );
            }
            if let Some(count) = expected["operationCount"].as_u64() {
                assert_eq!(
                    value["results"].as_array().map_or(0, Vec::len),
                    count as usize,
                    "{} operation count",
                    case.id
                );
            }
            if let Some(total) = expected["groupTotal"].as_u64() {
                assert_eq!(
                    value["groupCounts"]["total"], total,
                    "{} group total",
                    case.id
                );
            }
            if let Some(status) = expected["groupStatus"].as_str() {
                assert!(
                    value["groups"]
                        .as_array()
                        .is_some_and(|groups| groups.iter().any(|group| group["status"] == status)),
                    "{} missing group status {status}",
                    case.id
                );
            }
            if let Some(committed) = expected["committed"].as_bool() {
                assert!(
                    value["groups"].as_array().is_some_and(|groups| groups
                        .iter()
                        .any(|group| group["committed"] == committed)),
                    "{} missing committed={committed}",
                    case.id
                );
            }
            if let Some(failed) = expected["failedGroups"].as_u64() {
                assert_eq!(
                    value["groupCounts"]["failed"], failed,
                    "{} failed groups",
                    case.id
                );
            }
            if let Some(failure_count) = expected["failureCount"].as_u64() {
                assert_eq!(
                    value["failureCount"], failure_count,
                    "{} failure count",
                    case.id
                );
            }
            if let Some(added) = expected["changedLinesAdded"].as_u64() {
                assert_eq!(
                    value["changedLines"]["added"], added,
                    "{} added lines",
                    case.id
                );
            }
            if let Some(removed) = expected["changedLinesRemoved"].as_u64() {
                assert_eq!(
                    value["changedLines"]["removed"], removed,
                    "{} removed lines",
                    case.id
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn absent_room_tool_is_rejected_for_normal_worker() {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let error = server
            .call(CallToolRequestParams::new("room.diary.recent"))
            .await
            .expect_err("Room-only tool must not be callable by Normal worker");
        assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn stdio_resumes_stale_logical_session_before_first_tool_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, mut client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let server_task = tokio::spawn(async move {
            let transport =
                AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
            let running = server
                .serve(ResumableStdioTransport::new(transport))
                .await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });
        let mut client_read = BufReader::new(client_read);

        client_write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await?;
        client_write
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"tools/call\",\"params\":{\"name\":\"agent.info\",\"arguments\":{}}}\n",
            )
            .await?;
        client_write.flush().await?;

        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client_read.read_line(&mut line),
        )
        .await??;
        let response: Value = serde_json::from_str(&line)?;
        assert_eq!(response["id"], 0);
        assert!(response.get("error").is_none());
        assert_eq!(
            response["result"]["structuredContent"]["identity"]["profile"],
            "normal"
        );
        assert!(
            !line.contains("agentic-gpt-internal-init-"),
            "private initialize response leaked into the tunnel stream"
        );

        client_write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n")
            .await?;
        client_write.flush().await?;
        line.clear();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client_read.read_line(&mut line),
        )
        .await??;
        let follow_up: Value = serde_json::from_str(&line)?;
        assert_eq!(follow_up["id"], 1);
        assert_eq!(
            follow_up["result"]["tools"].as_array().map(Vec::len),
            Some(23)
        );

        drop(client_write);
        drop(client_read);
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn in_process_stdio_initialize_list_and_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let server_task = tokio::spawn(async move {
            let transport =
                AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
            let running = server
                .serve(ResumableStdioTransport::new(transport))
                .await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 23);
        let result = client
            .call_tool(CallToolRequestParams::new("job.list"))
            .await?;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result.structured_content.as_ref().unwrap()["jobs"],
            json!([])
        );
        let skills = client
            .call_tool(CallToolRequestParams::new("skills.list"))
            .await?;
        assert_eq!(skills.is_error, Some(false));
        let bootstrap = client
            .call_tool(CallToolRequestParams::new("bootstrap"))
            .await;
        assert!(bootstrap.is_err());
        let _ = client.cancel().await;
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn in_process_room_stdio_initialize_list_and_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
        let server_task = tokio::spawn(async move {
            let transport =
                AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
            let running = server
                .serve(ResumableStdioTransport::new(transport))
                .await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 35);
        let result = client
            .call_tool(CallToolRequestParams::new("room.diary.recent"))
            .await?;
        assert_eq!(result.is_error, Some(false));
        assert!(result.structured_content.as_ref().unwrap()["entries"].is_array());
        let _ = client.cancel().await;
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn process_tools_reject_legacy_identity_and_confirmation_fields() {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let identity = server
            .call(
                CallToolRequestParams::new("process.exec").with_arguments(Map::from_iter([
                    ("program".to_string(), Value::String("true".to_string())),
                    (
                        "agentId".to_string(),
                        Value::String("stdio-test-agent".to_string()),
                    ),
                ])),
            )
            .await
            .expect_err("Tunnel process schemas must reject agentId");
        assert_eq!(identity.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        let confirmation = server
            .call(
                CallToolRequestParams::new("process.exec").with_arguments(Map::from_iter([
                    ("program".to_string(), Value::String("true".to_string())),
                    (
                        "confirmMethod".to_string(),
                        Value::String("hub".to_string()),
                    ),
                ])),
            )
            .await
            .expect_err("Tunnel process schemas must reject confirmMethod");
        assert_eq!(confirmation.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        for removed_name in [
            "session.start",
            "session.list",
            "session.inspect",
            "session.wait",
            "session.kill",
            "process.batchExec",
            "process.get",
            "process.list",
            "process.kill",
        ] {
            let removed = server
                .call(CallToolRequestParams::new(removed_name))
                .await
                .expect_err("Removed lifecycle aliases must not be callable");
            assert_eq!(removed.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn process_and_skill_creation_share_job_lifecycle() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let quick = server
            .dispatch("process.exec", json!({"program": "true", "waitSeconds": 5}))
            .await?;
        assert_eq!(quick["state"], "completed");
        assert!(quick.get("completedInline").is_none());
        assert!(quick.get("pollAfterMs").is_none());
        let quick_job = quick["jobId"].as_str().unwrap().to_string();
        assert!(quick_job.starts_with("job_"));
        let fetched = server
            .dispatch("job.get", json!({"jobId": quick_job, "waitSeconds": 0}))
            .await?;
        assert_eq!(fetched["kind"], "process");
        assert_eq!(fetched["state"], "completed");

        let long = server
            .dispatch(
                "process.exec",
                json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
            )
            .await?;
        assert_eq!(long["state"], "starting");
        assert!(long.get("completedInline").is_none());
        let long_job = long["jobId"].as_str().unwrap().to_string();
        for _ in 0..100 {
            let state = server
                .dispatch("job.get", json!({"jobId": long_job, "waitSeconds": 0}))
                .await?;
            if state["state"] == "running" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let listed = server
            .dispatch("job.list", json!({"kind": "process"}))
            .await?;
        assert!(listed["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["jobId"] == long_job));
        let cancelled = server
            .dispatch("job.cancel", json!({"jobId": long_job}))
            .await?;
        assert_eq!(cancelled["state"], "cancelled");
        assert_eq!(cancelled["cancelOutcome"], "cancelled");
        assert_eq!(
            cancelled["terminationEvidence"],
            "local_process_kill_completed"
        );

        let batch = server
            .dispatch(
                "process.batch",
                json!({
                    "elements": [
                        {"program": "true"},
                        {"program": "false"}
                    ],
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(batch["jobs"].as_array().unwrap().len(), 2);
        assert_eq!(batch["status"], "completed_with_errors");
        assert_eq!(batch["jobs"][0]["state"], "completed");
        assert_eq!(batch["jobs"][1]["state"], "failed");

        let rejected = server
            .dispatch(
                "process.batch",
                json!({
                    "elements": [
                        {"program": "true"},
                        {"program": "true", "workingDirectory": "/missing"}
                    ],
                    "waitSeconds": 0
                }),
            )
            .await?;
        assert_eq!(rejected["error"]["code"], "process_batch_rejected");
        assert!(rejected.get("completedInline").is_none());
        assert!(rejected.get("pollAfterMs").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn slim_job_shapes_group_and_wait_only_are_exact() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let quick = server
            .dispatch(
                "process.exec",
                json!({
                    "program": "true",
                    "group": "  workstream  ",
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(quick["state"], "completed");
        assert!(quick["jobId"]
            .as_str()
            .is_some_and(|id| id.starts_with("job_")));
        assert!(quick.get("group").is_none());
        assert!(quick.get("kind").is_none());
        assert!(quick.get("completedInline").is_none());
        assert!(quick.get("pollAfterMs").is_none());
        assert!(quick.get("job").is_none());
        let quick_job = quick["jobId"].as_str().unwrap().to_string();

        let ordinary = server
            .dispatch(
                "job.get",
                json!({"jobId": quick_job, "waitSeconds": 0, "waitOnly": true}),
            )
            .await?;
        assert_eq!(ordinary["group"], "workstream");
        assert_eq!(ordinary["kind"], "process");
        assert_eq!(ordinary["state"], "completed");
        assert!(ordinary.get("createdAt").is_none());
        assert!(ordinary.get("finishedAt").is_none());
        assert!(ordinary.get("detailAvailable").is_none());
        assert!(ordinary.get("rejectReason").is_none());

        let listed = server
            .dispatch(
                "job.list",
                json!({"group": "workstream", "kind": "process", "limit": 1}),
            )
            .await?;
        assert!(listed.get("nextCursor").is_none());
        assert_eq!(listed["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(listed["jobs"][0]["group"], "workstream");
        assert_eq!(listed["jobs"][0]["kind"], "process");
        assert_eq!(listed["jobs"][0]["jobId"], quick_job);
        assert!(listed["jobs"][0]["createdAt"].is_string());
        assert!(listed["jobs"][0].get("program").is_none());

        let running = server
            .dispatch(
                "process.exec",
                json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
            )
            .await?;
        let running_job = running["jobId"].as_str().unwrap().to_string();
        let active_keys = running
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            active_keys,
            BTreeSet::from_iter([
                "elapsedMs".to_string(),
                "jobId".to_string(),
                "state".to_string(),
            ])
        );

        let wait_only = server
            .dispatch(
                "job.get",
                json!({
                    "jobId": running_job,
                    "waitSeconds": 1,
                    "waitOnly": true
                }),
            )
            .await?;
        let wait_keys = wait_only
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            wait_keys,
            BTreeSet::from_iter([
                "elapsedMs".to_string(),
                "jobId".to_string(),
                "state".to_string(),
            ])
        );
        assert!(is_active_job_state(wait_only["state"].as_str().unwrap()));

        let ordinary_zero = server
            .dispatch(
                "job.get",
                json!({
                    "jobId": wait_only["jobId"],
                    "waitSeconds": 0,
                    "waitOnly": true
                }),
            )
            .await?;
        assert_eq!(ordinary_zero["kind"], "process");
        assert!(ordinary_zero.get("elapsedMs").is_some());
        assert!(ordinary_zero.get("createdAt").is_none());
        let _ = server
            .dispatch("job.cancel", json!({"jobId": wait_only["jobId"]}))
            .await?;

        let rejected = server
            .dispatch("process.exec", json!({"program": "vim", "waitSeconds": 5}))
            .await?;
        assert_eq!(rejected["state"], "rejected");
        assert_eq!(rejected["error"]["code"], "requires_tty_not_supported");
        assert!(rejected.get("rejectReason").is_none());
        assert!(rejected.get("durationMs").is_none());

        let failed = server
            .dispatch(
                "process.exec",
                json!({"program": "false", "waitSeconds": 5}),
            )
            .await?;
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["exitCode"], 1);
        assert!(failed.get("error").is_none());
        Ok(())
    }

    #[test]
    fn managed_job_input_schemas_advertise_group_wait_only_and_cursor() {
        for name in [
            "process.exec",
            "process.batch",
            "mcp.callTool",
            "mcp.batch",
            "skills.run",
        ] {
            let descriptor = serde_json::to_value(tool_descriptor(name)).unwrap();
            assert!(descriptor["inputSchema"]["properties"]["group"].is_object());
            assert_eq!(
                descriptor["inputSchema"]["properties"]["group"]["maxLength"],
                32
            );
        }
        let get = serde_json::to_value(tool_descriptor("job.get")).unwrap();
        assert_eq!(
            get["inputSchema"]["properties"]["waitOnly"]["default"],
            false
        );
        let list = serde_json::to_value(tool_descriptor("job.list")).unwrap();
        for field in ["group", "kind", "state", "limit", "cursor"] {
            assert!(
                list["inputSchema"]["properties"][field].is_object(),
                "{field}"
            );
        }
    }

    #[test]
    fn mcp_batch_slim_conversion_distinguishes_job_truncation_from_aggregate_omission(
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let mut truncated_job = test_terminal_job();
        truncated_job.job_id = "job_testboot_truncated".to_string();
        truncated_job.kind = JobKind::Mcp;
        truncated_job.mcp_server_id = Some("fake".to_string());
        truncated_job.mcp_tool_name = Some("large".to_string());
        let truncated_detail = JobDetail {
            job: truncated_job,
            detail_available: true,
            result: None,
            error: None,
            result_truncated: true,
            result_bytes: Some(agentic_gpt_protocol::McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES),
            result_sha256: Some("sha256:truncated".to_string()),
            result_preview: Some("preview".to_string()),
        };
        let mut retained_job = test_terminal_job();
        retained_job.job_id = "job_testboot_retained".to_string();
        retained_job.kind = JobKind::Mcp;
        retained_job.created_at = now;
        retained_job.updated_at = now;
        retained_job.finished_at = Some(now);
        retained_job.mcp_server_id = Some("fake".to_string());
        retained_job.mcp_tool_name = Some("large".to_string());
        let retained_detail = JobDetail {
            job: retained_job,
            detail_available: true,
            result: Some(json!("x".repeat(
                agentic_gpt_protocol::McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES
            ))),
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
        };
        let value = slim_mcp_batch_response(serde_json::to_value(McpBatchResponse {
            batch_id: "batch_test".to_string(),
            status: McpBatchStatus::Completed,
            completed_inline: true,
            poll_after_ms: 0,
            results: vec![
                McpBatchChildResponse {
                    index: 0,
                    id: Some("truncated".to_string()),
                    result_omitted: false,
                    detail: truncated_detail,
                },
                McpBatchChildResponse {
                    index: 1,
                    id: Some("retained".to_string()),
                    result_omitted: false,
                    detail: retained_detail,
                },
            ],
            aggregate_truncated: false,
            aggregate_bytes: None,
            error: None,
        })?)?;
        assert!(value.get("batchId").is_none());
        assert!(value.get("completedInline").is_none());
        assert!(value.get("pollAfterMs").is_none());
        assert!(value.get("aggregateTruncated").is_none());
        assert!(value.get("aggregateBytes").is_none());
        assert_eq!(value["results"][0]["resultTruncated"], true);
        assert!(value["results"][0].get("resultOmitted").is_none());
        assert_eq!(value["results"][1]["resultOmitted"], true);
        assert!(value["results"][1].get("result").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn managed_batch_uses_one_confirmation_for_all_elements() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        {
            let mut config = server.state.config.write().await;
            config.confirmation_provider =
                crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
        }
        let (sender, mut receiver) = mpsc::unbounded_channel();
        *server.state.hub_sender.lock().await = Some(sender);
        let confirmation_count = Arc::new(AtomicUsize::new(0));
        let confirmation_count_clone = confirmation_count.clone();
        let response_state = server.state.clone();
        let responder = tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                if let AgentMessage::ConfirmationRequest { request_id, .. } = message {
                    confirmation_count_clone.fetch_add(1, Ordering::SeqCst);
                    if let Some(sender) = response_state
                        .pending_confirmations
                        .lock()
                        .await
                        .remove(&request_id)
                    {
                        let _ = sender.send("allow_once".to_string());
                    }
                }
            }
        });
        let batch = server
            .dispatch(
                "process.batch",
                json!({
                    "elements": [{"program": "true"}, {"program": "true"}],
                    "needConfirm": true,
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(batch["status"], json!("completed"));
        assert_eq!(confirmation_count.load(Ordering::SeqCst), 1);
        let audit = std::fs::read_to_string(
            server
                .state
                .config
                .read()
                .await
                .workspace_root
                .join(".agentic-gpt-audit.jsonl"),
        )?;
        assert_eq!(audit.lines().count(), 2);
        assert!(audit.lines().all(|line| {
            line.contains("\"policyDecision\":\"Confirm\"")
                && line.contains("\"confirmationResult\":\"allow_once\"")
        }));
        responder.abort();
        Ok(())
    }

    #[tokio::test]
    async fn denied_managed_batch_creates_no_jobs() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        {
            let mut config = server.state.config.write().await;
            config.confirmation_provider =
                crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
        }
        let (sender, mut receiver) = mpsc::unbounded_channel();
        *server.state.hub_sender.lock().await = Some(sender);
        let response_state = server.state.clone();
        let responder = tokio::spawn(async move {
            if let Some(AgentMessage::ConfirmationRequest { request_id, .. }) =
                receiver.recv().await
            {
                if let Some(sender) = response_state
                    .pending_confirmations
                    .lock()
                    .await
                    .remove(&request_id)
                {
                    let _ = sender.send("deny".to_string());
                }
            }
        });
        let batch = server
            .dispatch(
                "process.batch",
                json!({
                    "elements": [{"program": "true"}, {"program": "true"}],
                    "needConfirm": true,
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(batch["error"]["code"], "process_batch_rejected");
        let jobs = server.dispatch("job.list", json!({})).await?;
        assert_eq!(jobs["jobs"], json!([]));
        responder.abort();
        Ok(())
    }

    #[tokio::test]
    async fn tmux_actions_reject_incompatible_fields() {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let invalid = [
            json!({"action": "list", "needConfirm": false}),
            json!({"action": "create", "name": "demo", "cwd": ".", "needConfirm": false}),
            json!({"action": "close", "name": "demo", "cwd": "."}),
        ];
        for arguments in invalid {
            assert!(server.dispatch("tmux.sessions", arguments).await.is_err());
        }
        for arguments in [
            json!({"action": "list", "target": "%0"}),
            json!({"action": "capture", "target": "%0", "session": "demo"}),
        ] {
            assert!(server.dispatch("tmux.panes", arguments).await.is_err());
        }
    }

    #[test]
    fn managed_terminal_log_includes_duration() {
        let started_at = Utc::now() - chrono::Duration::milliseconds(42);
        let mut job = test_terminal_job();
        job.created_at = started_at;
        job.started_at = Some(started_at);
        job.updated_at = started_at + chrono::Duration::milliseconds(42);
        job.finished_at = Some(job.updated_at);
        job.args = vec!["sentinel-argument".to_string()];
        job.working_directory = Some("/sentinel/path".to_string());
        job.command_preview = Some("true sentinel-argument".to_string());
        job.stdout_tail = "sentinel-stdout".to_string();
        job.stderr_tail = "sentinel-stderr".to_string();
        let message = managed_terminal_event_message("normal", "tunnel:process.exec", &job);
        assert!(message.contains("durationMs=42"));
        let human_job_id = message
            .split("job=")
            .nth(1)
            .and_then(|value| value.split(';').next())
            .unwrap();
        assert_eq!(human_job_id.len(), 12);
        assert!(!message.contains("jobId="));
        assert!(!message.contains("sentinel"));
    }

    #[test]
    fn inline_terminal_tracker_discards_pending_terminal_event() {
        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = emitted.clone();
        let tracker = HumanTerminalTracker::with_emitter(move |message| {
            sink.lock().unwrap().push(message);
        });
        let job = test_terminal_job();
        tracker.record("normal", "tunnel:process.exec", &job);
        assert_eq!(tracker.state.lock().unwrap().pending.len(), 1);
        tracker.finish_response(true, None);
        assert!(tracker.state.lock().unwrap().pending.is_empty());
        assert!(matches!(
            tracker.state.lock().unwrap().response,
            HumanResponseState::Inline
        ));
        assert!(emitted.lock().unwrap().is_empty());
    }

    #[test]
    fn active_response_precedes_terminal_for_both_serial_orderings() {
        for terminal_first in [true, false] {
            let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = emitted.clone();
            let tracker = HumanTerminalTracker::with_emitter(move |message| {
                sink.lock().unwrap().push(message);
            });
            let job = test_terminal_job();
            if terminal_first {
                tracker.record("normal", "tunnel:process.exec", &job);
                tracker.finish_response(false, Some("status=active".to_string()));
            } else {
                tracker.finish_response(false, Some("status=active".to_string()));
                tracker.record("normal", "tunnel:process.exec", &job);
            }
            let emitted = emitted.lock().unwrap();
            assert_eq!(emitted.len(), 2);
            assert_eq!(emitted[0], "status=active");
            assert!(emitted[1].starts_with("managed_job;"));
        }
    }

    #[test]
    fn concurrent_check_clear_enqueue_interleaving_is_linearizable() {
        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = emitted.clone();
        let tracker = Arc::new(HumanTerminalTracker::with_emitter(move |message| {
            sink.lock().unwrap().push(message);
        }));
        let job = test_terminal_job();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let record_tracker = tracker.clone();
            let record_barrier = barrier.clone();
            scope.spawn(move || {
                record_barrier.wait();
                record_tracker.record("normal", "tunnel:process.exec", &job);
            });
            let response_tracker = tracker.clone();
            scope.spawn(move || {
                barrier.wait();
                response_tracker.finish_response(false, Some("status=active".to_string()));
            });
        });
        let emitted = emitted.lock().unwrap();
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0], "status=active");
        assert_eq!(emitted[1].matches("managed_job;").count(), 1);
    }

    fn test_terminal_job() -> JobInfo {
        let now = Utc::now();
        JobInfo {
            agent_id: "agent".to_string(),
            job_id: "job_testboot_0123456789abcdef".to_string(),
            group: None,
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state: JobState::Completed,
            created_at: now,
            started_at: Some(now),
            updated_at: now,
            finished_at: Some(now),
            program: Some("true".to_string()),
            args: Vec::new(),
            working_directory: None,
            command_preview: Some("true".to_string()),
            exit_code: Some(0),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        }
    }

    #[test]
    fn batch_lifecycle_detection_reads_job_envelopes() {
        let active = json!({
            "jobs": [{"jobId": "job_a", "state": "running"}]
        });
        assert!(value_has_active_job(&active));
        assert!(!value_has_terminal_failure(&active));

        let failed = json!({
            "jobs": [{
                "jobId": "job_b",
                "state": "failed",
                "rejectReason": "spawn_failed"
            }]
        });
        assert!(!value_has_active_job(&failed));
        assert!(value_has_terminal_failure(&failed));
        assert_eq!(
            human_failure_reason(&failed, None).as_deref(),
            Some("spawn_failed")
        );
    }

    #[test]
    fn tunnel_and_local_ingress_advertise_identical_surface() {
        let state = test_state(CapabilityProfile::Normal);
        let tunnel = AgentMcpServer::with_ingress(state.clone(), RequestIngress::TunnelStdio);
        let local = AgentMcpServer::with_ingress(state, RequestIngress::LocalUnix);
        assert_eq!(
            serde_json::to_value(tunnel.tools.as_ref()).unwrap(),
            serde_json::to_value(local.tools.as_ref()).unwrap()
        );
        assert_eq!(tunnel.ingress.label(), "tunnel:stdio");
        assert_eq!(local.ingress.label(), "local:unix");
    }

    #[tokio::test]
    async fn local_skill_audit_uses_local_request_source() -> anyhow::Result<()> {
        let server = AgentMcpServer::with_ingress(
            test_state(CapabilityProfile::Normal),
            RequestIngress::LocalUnix,
        );
        let workspace = server.state.config.read().await.workspace_root.clone();
        let scripts = workspace.join("skills/demo/scripts");
        std::fs::create_dir_all(&scripts)?;
        std::fs::write(workspace.join("skills/demo/SKILL.md"), "# Demo\n")?;
        let script = scripts.join("check.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf done\n")?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        crate::skills::activate(
            &server.state,
            SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await?;
        let result = server
            .dispatch(
                "skills.run",
                json!({"id": "demo", "path": "scripts/check.sh", "waitSeconds": 5}),
            )
            .await?;
        assert_eq!(result["state"], "completed");
        assert!(result.get("kind").is_none());
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
        assert!(audit.contains("\"requestSource\":\"local:skills.run\""));
        assert!(!audit.contains("\"requestSource\":\"tunnel:skills.run\""));
        Ok(())
    }

    #[tokio::test]
    async fn local_mcp_call_audit_uses_local_request_source() -> anyhow::Result<()> {
        let server = AgentMcpServer::with_ingress(
            test_state(CapabilityProfile::Normal),
            RequestIngress::LocalUnix,
        );
        let workspace = server.state.config.read().await.workspace_root.clone();
        let result = server
            .dispatch(
                "mcp.callTool",
                json!({
                    "serverId": "missing",
                    "toolName": "noop",
                    "arguments": {}
                }),
            )
            .await?;
        assert_eq!(result["state"], "rejected");
        assert!(result.get("kind").is_none());
        assert!(result.get("rejectReason").is_none());
        assert_eq!(result["error"]["code"], "mcp_server_not_found");
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
        assert!(audit.contains("\"requestSource\":\"local:mcp.callTool\""));
        assert!(audit.contains("\"terminalState\":\"rejected\""));
        assert!(audit.contains("\"terminationEvidence\":\"server_config_validation\""));
        assert!(!audit.contains("\"requestSource\":\"hub:mcp\""));
        Ok(())
    }

    #[tokio::test]
    async fn tunnel_skill_audit_uses_tunnel_request_source() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        let scripts = workspace.join("skills/demo/scripts");
        std::fs::create_dir_all(&scripts)?;
        std::fs::write(workspace.join("skills/demo/SKILL.md"), "# Demo\n")?;
        let script = scripts.join("check.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf done\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        }
        crate::skills::activate(
            &server.state,
            SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await?;
        let result = server
            .dispatch(
                "skills.run",
                json!({"id": "demo", "path": "scripts/check.sh", "waitSeconds": 5}),
            )
            .await?;
        assert_eq!(result["state"], "completed");
        assert!(result.get("kind").is_none());
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
        assert!(audit.contains("\"requestSource\":\"tunnel:skills.run\""));
        Ok(())
    }

    #[tokio::test]
    async fn room_profile_dispatches_room_memory_tools() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
        let bootstrap = server.dispatch("bootstrap", json!({})).await?;
        assert!(bootstrap.get("entrypoint").is_some());
        let skills = server.dispatch("skills.list", json!({})).await?;
        assert!(skills["skills"].is_array());
        let notebook = server.dispatch("room.notebook.recent", json!({})).await?;
        assert!(notebook["passages"].is_array());
        let diary = server.dispatch("room.diary.recent", json!({})).await?;
        assert!(diary["entries"].is_array());
        Ok(())
    }

    #[tokio::test]
    async fn every_room_adapter_rejects_legacy_identity_fields() {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
        let cases = [
            (
                "room.notebook.append",
                json!({"scope":"x","significance":"Normal","abstract":"a","content":"c","agentId":"foreign"}),
            ),
            ("room.notebook.recent", json!({"agentId":"foreign"})),
            (
                "room.notebook.selectExact",
                json!({"date":"2026-07-25","agentId":"foreign"}),
            ),
            (
                "room.notebook.search",
                json!({"query":"x","agentId":"foreign"}),
            ),
            (
                "room.notebook.current",
                json!({"scope":"x","agentId":"foreign"}),
            ),
            (
                "room.notebook.update",
                json!({"id":"x","agentId":"foreign"}),
            ),
            (
                "room.notebook.remove",
                json!({"id":"x","agentId":"foreign"}),
            ),
            (
                "room.diary.append",
                json!({"entry":"x","agentId":"foreign"}),
            ),
            ("room.diary.recent", json!({"agentId":"foreign"})),
            (
                "room.diary.selectExact",
                json!({"date":"2026-07-25","agentId":"foreign"}),
            ),
        ];
        for (name, arguments) in cases {
            assert!(
                server.dispatch(name, arguments).await.is_err(),
                "{name} must reject unknown identity fields"
            );
        }
    }

    #[tokio::test]
    async fn file_read_dispatch_supports_content_and_metadata_modes() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("read-me.txt"), "first\nsecond\n")?;
        let content = server
            .dispatch(
                "file.read",
                json!({"path":"read-me.txt", "startLine": 2, "endLine": 2}),
            )
            .await?;
        assert_eq!(content["content"], "second\n");
        assert!(content.get("metadata").is_none());
        let metadata = server
            .dispatch("file.read", json!({"path":"read-me.txt", "metadata": true}))
            .await?;
        assert_eq!(metadata["content"], "first\nsecond\n");
        assert_eq!(metadata["metadata"]["totalLines"], 2);
        assert!(metadata.get("revision").is_none());
        let missing = server
            .dispatch("file.read", json!({"path":"missing.txt"}))
            .await?;
        assert_eq!(missing["error"]["code"], "file_not_found");
        Ok(())
    }

    #[tokio::test]
    async fn in_process_stdio_file_read_batch_descriptor_and_call_contract() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("stdio-batch.txt"), "needle\n")?;
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server_task = tokio::spawn(async move {
            let transport =
                AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
            let running = server
                .serve(ResumableStdioTransport::new(transport))
                .await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        let read_descriptor = tools.iter().find(|tool| tool.name == "file.read").unwrap();
        let read_schema = serde_json::to_value(read_descriptor)?;
        assert!(read_schema["inputSchema"]["properties"]["requests"].is_object());
        assert!(read_schema["inputSchema"]["oneOf"].is_array());
        let removed_tool = ["file", "batch"].join(".");
        assert!(!tools.iter().any(|tool| tool.name == removed_tool));

        let result = client
            .call_tool(
                CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                    "requests".to_string(),
                    json!([{"path":"stdio-batch.txt"},{"path":"missing.txt"}]),
                )])),
            )
            .await?;
        let value = result.structured_content.unwrap();
        assert_eq!(value["results"][0]["status"], "completed");
        assert_eq!(value["results"][0]["result"]["content"], "needle\n");
        assert_eq!(value["results"][1]["status"], "failed");
        assert!(value["results"][0]["result"].get("revision").is_none());
        let _ = client.cancel().await;
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn file_search_dispatch_supports_literal_and_regex_queries() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("search.rs"), "Alpha\nBeta 42\n")?;
        let clipped = server
            .dispatch(
                "file.search",
                json!({"path":"search.rs", "query":"Beta", "contextLines":8}),
            )
            .await?;
        assert_eq!(clipped["contextLines"], 5);
        assert_eq!(
            clipped["warnings"][0],
            "context_lines_clipped_to_configured_limit"
        );
        assert_eq!(clipped["matches"].as_array().map(Vec::len), Some(1));

        server
            .state
            .config
            .write()
            .await
            .limits
            .max_file_search_context_lines = 20;
        let expanded = server
            .dispatch(
                "file.search",
                json!({"path":"search.rs", "query":"Beta", "contextLines":8}),
            )
            .await?;
        assert_eq!(expanded["matches"].as_array().map(Vec::len), Some(1));
        assert!(expanded.get("contextLines").is_none());
        assert!(expanded.get("warnings").is_none());

        server
            .state
            .config
            .write()
            .await
            .limits
            .max_file_search_context_lines = 0;
        let disabled_context = server
            .dispatch(
                "file.search",
                json!({"path":"search.rs", "query":"Beta", "contextLines":5}),
            )
            .await?;
        assert_eq!(disabled_context["contextLines"], 0);
        assert!(disabled_context["warnings"].is_array());

        let literal = server
            .dispatch(
                "file.search",
                json!({"path":".", "query":"alpha", "caseSensitive":false, "include":["**/*.rs"]}),
            )
            .await?;
        assert_eq!(literal["matches"].as_array().map(Vec::len), Some(1));
        let regex = server
            .dispatch(
                "file.search",
                json!({"path":"search.rs", "query":"Beta \\d+", "mode":"regex"}),
            )
            .await?;
        assert_eq!(regex["matches"].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn file_read_and_search_batches_preserve_order_and_isolate_failures() -> anyhow::Result<()>
    {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("batch-a.txt"), "needle-a\n")?;
        std::fs::write(workspace.join("batch-b.txt"), "needle-b\n")?;

        let reads = server
            .dispatch(
                "file.read",
                json!({"requests":[
                    {"path":"batch-a.txt"},
                    {"path":"missing.txt"},
                    {"path":"batch-b.txt","metadata":true}
                ]}),
            )
            .await?;
        assert_eq!(reads["results"][0]["index"], 0);
        assert_eq!(reads["results"][0]["result"]["content"], "needle-a\n");
        assert_eq!(reads["results"][1]["status"], "failed");
        assert_eq!(reads["results"][2]["index"], 2);
        assert_eq!(reads["results"][2]["result"]["content"], "needle-b\n");
        assert!(reads["results"][2]["result"]["metadata"].is_object());

        let searches = server
            .dispatch(
                "file.search",
                json!({"requests":[
                    {"path":"batch-a.txt","query":"needle-a"},
                    {"path":"missing.txt","query":"needle"},
                    {"path":"batch-b.txt","query":"needle-b"}
                ]}),
            )
            .await?;
        assert_eq!(
            searches["results"][0]["result"]["matches"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(searches["results"][1]["status"], "failed");
        assert_eq!(
            searches["results"][2]["result"]["matches"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );

        assert!(server
            .dispatch("file.read", json!({"path":"batch-a.txt","requests":[]}))
            .await
            .is_err());
        assert!(server
            .dispatch(
                "file.search",
                json!({"path":"batch-a.txt","query":"x","requests":[]})
            )
            .await
            .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_apply_patch_supports_multi_file_changes_and_slim_response(
    ) -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("update.txt"), "old\n")?;
        std::fs::write(workspace.join("delete.txt"), "gone\n")?;
        std::fs::write(workspace.join("move.txt"), "move\n")?;
        let patch = "*** Begin Patch\n*** Add File: add.txt\n+added\n*** Update File: update.txt\n@@\n-old\n+new\n*** Delete File: delete.txt\n*** Update File: move.txt\n*** Move to: moved.txt\n@@\n-move\n+moved\n*** End Patch";
        let result = server.dispatch("file.edit", json!({"patch":patch})).await?;
        assert_eq!(result["status"], "completed");
        assert_eq!(result["changed"], 4);
        assert_eq!(result["changes"].as_array().map(Vec::len), Some(4));
        assert_eq!(
            std::fs::read_to_string(workspace.join("add.txt"))?,
            "added\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("update.txt"))?,
            "new\n"
        );
        assert!(!workspace.join("delete.txt").exists());
        assert_eq!(
            std::fs::read_to_string(workspace.join("moved.txt"))?,
            "moved\n"
        );
        assert!(!workspace.join("move.txt").exists());
        assert_eq!(
            result["changes"][0],
            json!({"path":"add.txt","action":"created"})
        );
        assert_eq!(
            result["changes"][1],
            json!({"path":"update.txt","action":"updated"})
        );
        assert_eq!(
            result["changes"][2],
            json!({"path":"delete.txt","action":"deleted"})
        );
        assert_eq!(
            result["changes"][3],
            json!({"path":"move.txt","action":"moved","destination":"moved.txt"})
        );
        assert!(result.get("summary").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_later_commit_failure_reports_prior_and_skipped_changes() -> anyhow::Result<()>
    {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("unchanged.txt"), "same\n")?;
        crate::file_ops::inject_commit_failure(&workspace.join("failed.txt"));
        let result = server
            .dispatch(
                "file.edit",
                json!({"patch":"*** Begin Patch
*** Update File: unchanged.txt
@@
-same
+same
*** Add File: committed.txt
+committed
*** Add File: failed.txt
+failed
*** Add File: skipped.txt
+skipped
*** End Patch"}),
            )
            .await?;
        assert_eq!(result["status"], "completed_with_errors");
        assert_eq!(result["changes"][0]["status"], "unchanged");
        assert_eq!(result["changes"][1]["status"], "created");
        assert_eq!(result["changes"][2]["status"], "failed");
        assert_eq!(result["changes"][2]["error"]["code"], "file_write_failed");
        assert_eq!(result["changes"][3]["status"], "skipped-not-attempted");
        assert_eq!(result["summary"]["committed"], 1);
        assert_eq!(result["summary"]["failed"], 1);
        assert_eq!(result["summary"]["skipped"], 1);
        assert!(workspace.join("committed.txt").is_file());
        assert!(!workspace.join("failed.txt").exists());
        assert!(!workspace.join("skipped.txt").exists());

        let audit_lines = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| record["tool"] == "file.edit")
            .collect::<Vec<_>>();
        assert_eq!(audit_lines.len(), 4);
        assert_eq!(audit_lines[0]["outcome"], "unchanged");
        assert_eq!(audit_lines[0]["committed"], false);
        assert_eq!(audit_lines[1]["outcome"], "created");
        assert_eq!(audit_lines[2]["outcome"], "failed");
        assert_eq!(audit_lines[2]["errorCode"], "file_write_failed");
        assert_eq!(audit_lines[3]["outcome"], "skipped-not-attempted");
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_apply_patch_context_mismatch_and_confirmation_do_not_write(
    ) -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("good.txt"), "good\n")?;
        std::fs::write(workspace.join("bad.txt"), "actual\n")?;
        let mismatch = server
            .dispatch("file.edit", json!({"patch":"*** Begin Patch\n*** Update File: good.txt\n@@\n-good\n+changed\n*** Update File: bad.txt\n@@\n-wrong\n+never\n*** End Patch"}))
            .await?;
        assert_eq!(mismatch["error"]["code"], "file_patch_conflict");
        assert_eq!(
            std::fs::read_to_string(workspace.join("good.txt"))?,
            "good\n"
        );

        server
            .state
            .config
            .write()
            .await
            .confirmation_provider
            .channels
            .clear();
        let denied = server
            .dispatch("file.edit", json!({"patch":"*** Begin Patch\n*** Update File: good.txt\n@@\n-good\n+confirmed\n*** End Patch","needConfirm":true}))
            .await?;
        assert_eq!(denied["error"]["code"], "file_confirmation_unavailable");
        assert_eq!(
            std::fs::read_to_string(workspace.join("good.txt"))?,
            "good\n"
        );
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_apply_patch_revalidates_external_change_before_commit() -> anyhow::Result<()>
    {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        let path = workspace.join("race.txt");
        std::fs::write(&path, "before\n")?;
        crate::file_ops::inject_external_change(&path, b"external\n");
        let result = server
            .dispatch(
                "file.edit",
                json!({"patch":"*** Begin Patch\n*** Update File: race.txt\n@@\n-before\n+agent\n*** End Patch"}),
            )
            .await?;
        assert_eq!(result["error"]["code"], "file_revision_conflict");
        assert_eq!(std::fs::read_to_string(&path)?, "external\n");
        Ok(())
    }

    #[tokio::test]
    async fn file_lock_registry_prunes_released_paths() -> anyhow::Result<()> {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        {
            let _guard =
                crate::file_ops::lock_target(&server.state, &workspace.join("one.txt")).await;
        }
        {
            let _guard =
                crate::file_ops::lock_target(&server.state, &workspace.join("two.txt")).await;
        }
        assert_eq!(server.state.file_locks.lock().await.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes() -> anyhow::Result<()>
    {
        let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
        let mcp = server.dispatch("mcp.list", json!({})).await?;
        assert!(mcp["servers"].is_array());
        let skills = server.dispatch("skills.list", json!({})).await?;
        assert!(skills["skills"].is_array());
        assert!(skills.get("activeSkills").is_none());
        assert!(skills.get("warnings").is_none());
        let panes = server
            .dispatch("tmux.panes", json!({"action": "list"}))
            .await?;
        assert!(panes.get("panes").is_some() || panes.get("error").is_some());
        let sessions = server
            .dispatch("tmux.sessions", json!({"action": "list"}))
            .await?;
        assert!(sessions.get("sessions").is_some() || sessions.get("error").is_some());
        Ok(())
    }

    fn test_state(profile: CapabilityProfile) -> AppState {
        let root = std::env::temp_dir().join(format!("agentic-stdio-{}", Uuid::new_v4()));
        let workspace_root = root.join("workspace");
        let mut config = Config::default_config().expect("default config");
        config.agent_id = "stdio-test-agent".to_string();
        config.workspace_root = workspace_root.clone();
        config.path_policy.write_roots = vec![workspace_root.clone()];
        config.ensure_workspace().expect("workspace");
        let bootstrap_root = workspace_root.join("bootstrap");
        std::fs::create_dir_all(&bootstrap_root).expect("bootstrap directory");
        std::fs::write(
            bootstrap_root.join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room Bootstrap\ndescription: Test bootstrap\nschemaVersion: 1\n---\n",
        )
        .expect("bootstrap entrypoint");
        AppState {
            config_path: PathBuf::from("stdio-test-config.json"),
            config: Arc::new(RwLock::new(config)),
            private_state: crate::private_state::PrivateStatePaths::for_test(
                std::env::temp_dir().join(format!(
                    "agentic-test-private-{}",
                    uuid::Uuid::new_v4().simple()
                )),
            ),
            job_history: crate::job_history::JobHistoryStore::disabled(
                std::env::temp_dir().join("agentic-stdio-test-jobs.sqlite3"),
            ),
            runtime: RuntimeModel::tunnel(profile, false),
            started_at: chrono::Utc::now(),
            boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            supervised: true,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(SkillLeaseManager::new()),
            skill_installs: Arc::new(InstallManager::new()),
        }
    }
}
