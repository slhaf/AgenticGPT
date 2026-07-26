use std::sync::{Arc, Mutex};

use agentic_gpt_protocol::{ExecRequest, HubCommand, SessionInfo};
use anyhow::Result;
use chrono::Utc;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ErrorData, ListToolsResult, Meta,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use crate::{
    local_service,
    state::{AppState, CapabilityProfile},
};

const INSTRUCTIONS: &str = "Agentic GPT local Tunnel worker. Start with agent.info to inspect the active profile, exact workspace/path policy, capacity, confirmation channels, and connection state. Use file.read/search/edit/batch for bounded UTF-8 workspace work, process.* for managed local processes, tmux for persistent workspaces, skills for the local skills workspace, and bootstrap for Room startup guidance. All calls remain subject to path policy, configured confirmation, audit, and bounded waits.";

const NORMAL_TOOLS: &[&str] = &[
    "agent.info",
    "mcp.callTool",
    "mcp.list",
    "process.batchExec",
    "process.exec",
    "process.get",
    "process.kill",
    "process.list",
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

pub(crate) async fn serve(state: AppState) -> Result<()> {
    let server = StdioMcpServer::new(state);
    let running = server.serve(stdio()).await?;
    let _ = running.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct StdioMcpServer {
    state: AppState,
    tools: Arc<Vec<Tool>>,
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

    fn record(&self, profile: &str, source: &str, session: &SessionInfo) {
        let message = managed_terminal_event_message(profile, source, session);
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

impl StdioMcpServer {
    fn new(state: AppState) -> Self {
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
                    "stdio_tool; run={}; tool={name}; profile={}; status=failed; durationMs={}; errorCode={}",
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
        let session: Option<agentic_gpt_protocol::SessionInfo> = value
            .get("session")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        if let Some(session) = session.as_ref() {
            crate::hub::report_session(&self.state, session.clone());
        }
        let is_error = value.get("error").is_some();
        let reason = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let session_id = session.as_ref().map(|session| session.session_id.as_str());
        let exit_code = session.as_ref().and_then(|session| session.exit_code);
        let active = value_has_active_session(&value);
        let terminal_failure = value_has_terminal_failure(&value);
        let human_reason = reason
            .clone()
            .or_else(|| human_failure_reason(&value, session.as_ref()));
        let mut lifecycle = format!(
            "stdio_tool; run={}; tool={name}; profile={}; status={}; durationMs={}",
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
        if let Some(session_id) = session_id {
            lifecycle.push_str(&format!(
                "; session={}",
                crate::utils::compact_id(session_id)
            ));
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
            session,
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
                .map(|value| !value_has_active_session(value))
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
                let config = self.state.config.read().await.clone();
                let resolved = crate::file_ops::resolve_path(
                    &config,
                    &args.path,
                    crate::file_ops::Access::Read,
                );
                match resolved {
                    Ok(resolved) => crate::file_ops::to_result(crate::file_ops::read(
                        &resolved,
                        args.include_content,
                        args.start_line,
                        args.end_line,
                    )),
                    Err(error) => Ok(error.value()),
                }
            }
            "process.exec" => {
                self.dispatch_process_exec(arguments, terminal_tracker)
                    .await
            }
            "process.batchExec" => {
                self.dispatch_process_batch(arguments, terminal_tracker)
                    .await
            }
            "process.get" => self.dispatch_process_get(arguments).await,
            "process.kill" => self.dispatch_process_kill(arguments).await,
            "process.list" => self.dispatch_process_list(arguments).await,
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
                let config = self.state.config.read().await.clone();
                match crate::mcp::call_tool(
                    &self.state,
                    agentic_gpt_protocol::McpCallToolRequest {
                        agent_id: config.agent_id,
                        server_id: args.server_id,
                        tool_name: args.tool_name,
                        arguments: args.arguments,
                    },
                )
                .await
                {
                    Ok(value) => Ok(value),
                    Err(error) => Ok(json!({
                        "error": { "code": "mcp_call_tool_failed", "message": error.to_string() }
                    })),
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
            _ => Err(anyhow::anyhow!("unknown stdio tool: {name}")),
        }
    }

    async fn dispatch_process_exec(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: ProcessExecArgs = from_value(arguments)?;
        let config = self.state.config.read().await.clone();
        let terminal_event_hook = managed_terminal_event_hook(
            self.state.runtime.profile,
            "tunnel:process.exec",
            terminal_tracker,
        );
        let session_id = task_id("sess");
        let info = crate::sessions::start_managed_session_async(
            self.state.clone(),
            session_id,
            ExecRequest {
                agent_id: config.agent_id,
                program: args.program,
                args: args.args,
                need_confirm: args.need_confirm,
                confirm_method: None,
                working_directory: args.working_directory,
            },
            crate::sessions::ManagedSessionOptions {
                terminal_event_hook: Some(terminal_event_hook),
                ..crate::sessions::ManagedSessionOptions::for_source("tunnel:process.exec")
            },
        )
        .await;
        let info = crate::sessions::wait_for_session(
            &self.state,
            info,
            args.wait_seconds.unwrap_or(5).min(30),
        )
        .await;
        Ok(managed_process_response(info))
    }

    async fn dispatch_process_get(&self, arguments: Value) -> Result<Value> {
        let args: ProcessGetArgs = from_value(arguments)?;
        let info = crate::sessions::inspect_session(&self.state, &args.session_id).await;
        match info {
            Some(info) => {
                let info = crate::sessions::wait_for_session(
                    &self.state,
                    info,
                    args.wait_seconds.unwrap_or(0).min(30),
                )
                .await;
                Ok(serde_json::to_value(info)?)
            }
            None => Ok(session_not_found(args.session_id)),
        }
    }

    async fn dispatch_process_batch(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: ProcessBatchArgs = from_value(arguments)?;
        let batch_id = task_id("batch");
        let started_at = Utc::now();
        let wait_seconds = args.wait_seconds.unwrap_or(5).min(30);
        let inputs = args.elements;
        if inputs.is_empty() {
            return Ok(serde_json::to_value(TunnelBatchResponse {
                batch_id,
                status: "completed".to_string(),
                results: Vec::new(),
                started_at,
                updated_at: Utc::now(),
            })?);
        }
        let config = self.state.config.read().await.clone();
        let mut prepared = Vec::with_capacity(inputs.len());
        let mut rejection_reasons = vec![None; inputs.len()];
        let mut first_rejection = None;
        for (index, input) in inputs.iter().cloned().enumerate() {
            let working_directory = input
                .working_directory
                .clone()
                .or_else(|| args.working_directory.clone());
            let decision = crate::policy::policy_decision_for_profile(
                &config,
                self.state.runtime.profile,
                &input.program,
                &input.args,
                args.need_confirm,
            );
            let resolved =
                match crate::exec::resolve_working_directory(&config, working_directory.as_deref())
                {
                    Ok(directory) => directory,
                    Err(reason) => {
                        rejection_reasons[index] = Some(reason.clone());
                        first_rejection.get_or_insert((index, reason));
                        continue;
                    }
                };
            if let Err(reason) =
                crate::exec::preflight(&config, &resolved, &input.program, &input.args)
            {
                rejection_reasons[index] = Some(reason.clone());
                first_rejection.get_or_insert((index, reason));
                continue;
            }
            if decision == crate::policy::PolicyDecision::Deny {
                rejection_reasons[index] = Some("policy_denied".to_string());
                first_rejection.get_or_insert((index, "policy_denied".to_string()));
                continue;
            }
            prepared.push(PreparedTunnelBatchElement {
                index,
                input,
                resolved_working_directory: resolved,
                decision,
            });
        }
        if let Some((index, reason)) = first_rejection {
            return Ok(serde_json::to_value(rejected_batch_response_with_reasons(
                batch_id,
                inputs,
                rejection_reasons,
                index,
                reason,
                started_at,
            ))?);
        }

        let confirmation_elements = prepared
            .iter()
            .map(|element| crate::exec::PreparedBatchElement {
                index: element.index,
                program: element.input.program.clone(),
                args: element.input.args.clone(),
                working_directory: element
                    .input
                    .working_directory
                    .clone()
                    .or_else(|| args.working_directory.clone()),
                resolved_working_directory: element.resolved_working_directory.clone(),
                decision: element.decision,
                reject_reason: None,
            })
            .collect::<Vec<_>>();
        let needs_confirmation = confirmation_elements
            .iter()
            .filter(|element| element.decision == crate::policy::PolicyDecision::Confirm)
            .cloned()
            .collect::<Vec<_>>();
        let confirmation_result = if !needs_confirmation.is_empty() {
            let confirmation = crate::confirmation::request_batch_confirmation(
                &self.state,
                &config,
                None,
                &needs_confirmation,
                &confirmation_elements,
            )
            .await;
            if confirmation != "allow_once" {
                return Ok(serde_json::to_value(rejected_batch_response(
                    batch_id,
                    inputs,
                    needs_confirmation[0].index,
                    confirmation,
                    started_at,
                ))?);
            }
            Some(confirmation)
        } else {
            None
        };

        let agent_id = config.agent_id;
        let terminal_event_hook = managed_terminal_event_hook(
            self.state.runtime.profile,
            "tunnel:process.batchExec",
            terminal_tracker,
        );
        let specs = prepared
            .into_iter()
            .map(|element| crate::sessions::ManagedProcessSpec {
                request: ExecRequest {
                    agent_id: agent_id.clone(),
                    program: element.input.program,
                    args: element.input.args,
                    need_confirm: args.need_confirm,
                    confirm_method: None,
                    working_directory: element
                        .input
                        .working_directory
                        .or_else(|| args.working_directory.clone()),
                },
                working_directory: element.resolved_working_directory,
                decision: element.decision,
                confirmation_result: confirmation_result.clone(),
                request_source: "tunnel:process.batchExec".to_string(),
                terminal_event_hook: Some(terminal_event_hook.clone()),
            })
            .collect::<Vec<_>>();
        let infos =
            match crate::sessions::start_prepared_managed_batch(self.state.clone(), specs).await {
                Ok(infos) => infos,
                Err(reason) => {
                    return Ok(serde_json::to_value(rejected_batch_response(
                        batch_id, inputs, 0, reason, started_at,
                    ))?);
                }
            };
        let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds);
        let mut latest = infos;
        loop {
            let mut all_terminal = true;
            for info in &mut latest {
                if let Some(session) =
                    crate::sessions::inspect_session(&self.state, &info.session_id).await
                {
                    *info = session;
                }
                if matches!(
                    info.state.as_str(),
                    "starting" | "running" | "waiting_confirmation"
                ) {
                    all_terminal = false;
                }
            }
            if all_terminal || Instant::now() >= deadline {
                break;
            }
            sleep(std::time::Duration::from_millis(20)).await;
        }
        let results = latest
            .into_iter()
            .enumerate()
            .map(|(index, info)| TunnelBatchResult {
                index,
                program: info.program.clone(),
                args: info.args.clone(),
                working_directory: info.working_directory.clone(),
                outcome: "managed".to_string(),
                process: Some(managed_process_response(info)),
                reject_reason: None,
            })
            .collect::<Vec<_>>();
        let status = if results.iter().any(|result| {
            result
                .process
                .as_ref()
                .and_then(|process| process.get("session"))
                .and_then(|session| session.get("state"))
                .and_then(Value::as_str)
                .map(|state| matches!(state, "starting" | "running" | "waiting_confirmation"))
                .unwrap_or(false)
        }) {
            "running"
        } else if results.iter().any(|result| {
            result
                .process
                .as_ref()
                .and_then(|process| process.get("session"))
                .and_then(|session| session.get("state"))
                .and_then(Value::as_str)
                .map(|state| state != "exited")
                .unwrap_or(true)
        }) {
            "partial_failed"
        } else {
            "completed"
        };
        Ok(serde_json::to_value(TunnelBatchResponse {
            batch_id,
            status: status.to_string(),
            results,
            started_at,
            updated_at: Utc::now(),
        })?)
    }

    async fn dispatch_process_kill(&self, arguments: Value) -> Result<Value> {
        let args: ProcessKillArgs = from_value(arguments)?;
        match crate::sessions::kill_session(&self.state, &args.session_id).await {
            Some(info) => Ok(serde_json::to_value(info)?),
            None => Ok(session_not_found(args.session_id)),
        }
    }

    async fn dispatch_process_list(&self, arguments: Value) -> Result<Value> {
        let _: EmptyArgs = from_value(arguments)?;
        Ok(json!({
            "sessions": crate::sessions::current_sessions(&self.state).await,
        }))
    }

    async fn dispatch_skill_run(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let args: SkillRunArgs = from_value(arguments)?;
        let request = agentic_gpt_protocol::SkillRunRequest {
            id: args.id,
            path: args.path,
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
        let info = crate::sessions::start_skill_session_async_with_hook_and_source(
            self.state.clone(),
            task_id("sess"),
            ExecRequest {
                agent_id: config.agent_id,
                program: program.to_string_lossy().to_string(),
                args: request.args.unwrap_or_default(),
                need_confirm: false,
                confirm_method: None,
                working_directory: request.working_directory,
            },
            &request.id,
            &request.path,
            "tunnel:skills.run",
            Some(managed_terminal_event_hook(
                self.state.runtime.profile,
                "tunnel:skills.run",
                terminal_tracker,
            )),
        )
        .await;
        let info = crate::sessions::wait_for_session(&self.state, info, wait_seconds).await;
        Ok(managed_process_response(info))
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
        Ok(json!({
            "skills": skills,
            "activeSkills": active.active_skills,
            "warnings": warnings,
        }))
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

impl ServerHandler for StdioMcpServer {
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
                "agentic-gpt-stdio-worker",
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
    working_directory: Option<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessGetArgs {
    session_id: String,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessKillArgs {
    session_id: String,
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
    working_directory: Option<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug)]
struct PreparedTunnelBatchElement {
    index: usize,
    input: BatchElementArgs,
    resolved_working_directory: std::path::PathBuf,
    decision: crate::policy::PolicyDecision,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelBatchResult {
    index: usize,
    program: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    working_directory: Option<String>,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reject_reason: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelBatchResponse {
    batch_id: String,
    status: String,
    results: Vec<TunnelBatchResult>,
    started_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileReadArgs {
    path: String,
    #[serde(default = "default_true")]
    include_content: bool,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

fn default_true() -> bool {
    true
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProcessResponse {
    session_id: String,
    completed_inline: bool,
    poll_after_ms: u64,
    session: SessionInfo,
}

async fn dispatch(server: &StdioMcpServer, command: HubCommand) -> Result<Value> {
    local_service::dispatch(server.state.clone(), command).await
}

fn session_not_found(session_id: String) -> Value {
    json!({
        "error": {
            "code": "session_not_found",
            "message": format!("Session was not found: {session_id}"),
        }
    })
}

fn managed_process_response(info: SessionInfo) -> Value {
    let completed_inline = !matches!(
        info.state.as_str(),
        "starting" | "running" | "waiting_confirmation"
    );
    serde_json::to_value(ManagedProcessResponse {
        session_id: info.session_id.clone(),
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        session: info,
    })
    .unwrap_or_else(|_| {
        json!({
            "error": {
                "code": "process_response_failed",
                "message": "failed to encode managed process response",
            }
        })
    })
}

fn rejected_batch_response(
    batch_id: String,
    inputs: Vec<BatchElementArgs>,
    rejected_index: usize,
    reason: String,
    started_at: chrono::DateTime<Utc>,
) -> TunnelBatchResponse {
    let mut reasons = vec![None; inputs.len()];
    reasons[rejected_index] = Some(reason.clone());
    rejected_batch_response_with_reasons(
        batch_id,
        inputs,
        reasons,
        rejected_index,
        reason,
        started_at,
    )
}

fn rejected_batch_response_with_reasons(
    batch_id: String,
    inputs: Vec<BatchElementArgs>,
    reasons: Vec<Option<String>>,
    rejected_index: usize,
    reason: String,
    started_at: chrono::DateTime<Utc>,
) -> TunnelBatchResponse {
    let results = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| TunnelBatchResult {
            index,
            program: input.program,
            args: input.args,
            working_directory: input.working_directory,
            outcome: if reasons[index].is_some() {
                "rejected".to_string()
            } else {
                "skipped".to_string()
            },
            process: None,
            reject_reason: if let Some(reason) = reasons[index].clone() {
                Some(reason)
            } else if index == rejected_index {
                Some(reason.clone())
            } else {
                Some("batch_rejected".to_string())
            },
        })
        .collect();
    TunnelBatchResponse {
        batch_id,
        status: "rejected".to_string(),
        results,
        started_at,
        updated_at: Utc::now(),
    }
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

fn value_has_active_session(value: &Value) -> bool {
    if value
        .get("session")
        .and_then(|session| session.get("state"))
        .and_then(Value::as_str)
        .is_some_and(|state| matches!(state, "starting" | "running" | "waiting_confirmation"))
    {
        return true;
    }
    value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| result.get("process"))
        .filter_map(process_state)
        .any(|state| matches!(state, "starting" | "running" | "waiting_confirmation"))
}

fn value_has_terminal_failure(value: &Value) -> bool {
    value.get("error").is_some()
        || value
            .get("session")
            .and_then(|session| session.get("state"))
            .and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "failed" | "killed"))
        || value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|result| {
                result
                    .get("outcome")
                    .and_then(Value::as_str)
                    .is_some_and(|outcome| matches!(outcome, "rejected" | "failed"))
                    || result
                        .get("process")
                        .and_then(process_state)
                        .is_some_and(|state| matches!(state, "failed" | "killed"))
            })
}

fn human_failure_reason(value: &Value, session: Option<&SessionInfo>) -> Option<String> {
    session
        .and_then(|session| session.reject_reason.clone())
        .or_else(|| {
            value
                .get("results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find_map(|result| {
                    result
                        .get("rejectReason")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            result
                                .get("process")
                                .and_then(process_reject_reason)
                                .map(str::to_string)
                        })
                })
        })
}

fn process_state(process: &Value) -> Option<&str> {
    process.get("state").and_then(Value::as_str).or_else(|| {
        process
            .get("session")
            .and_then(|session| session.get("state"))
            .and_then(Value::as_str)
    })
}

fn process_reject_reason(process: &Value) -> Option<&str> {
    process
        .get("rejectReason")
        .and_then(Value::as_str)
        .or_else(|| {
            process
                .get("session")
                .and_then(|session| session.get("rejectReason"))
                .and_then(Value::as_str)
        })
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
    source: &'static str,
    tracker: Arc<HumanTerminalTracker>,
) -> crate::sessions::TerminalEventHook {
    let profile = profile.label();
    Arc::new(move |session| {
        tracker.record(&profile, source, session);
    })
}

fn managed_terminal_event_message(profile: &str, source: &str, session: &SessionInfo) -> String {
    let duration_ms = (session.updated_at - session.started_at)
        .num_milliseconds()
        .max(0);
    let mut message = format!(
        "managed_session; source={source}; profile={profile}; status={}; session={}; durationMs={duration_ms}",
        session.state,
        crate::utils::compact_id(&session.session_id)
    );
    if let Some(exit_code) = session.exit_code {
        message.push_str(&format!("; exitCode={exit_code}"));
    }
    if let Some(reason) = session.reject_reason.as_deref() {
        message.push_str(&format!("; errorCode={}", bounded_error_code(reason)));
    }
    message
}

fn tool_descriptor(name: &str) -> Tool {
    let (properties, required) = tool_schema(name);
    let annotations = ToolAnnotations::new()
        .read_only(tool_is_read_only(name))
        .destructive(tool_is_destructive(name))
        .open_world(tool_is_open_world(name));
    Tool::new(
        name.to_string(),
        tool_description(name),
        schema(properties, required),
    )
    .with_annotations(annotations)
    .with_raw_output_schema(Arc::new(output_schema()))
    .with_meta(Meta(Map::from_iter([(
        "transport".to_string(),
        Value::String("stdio".to_string()),
    )])))
}

fn tool_schema(name: &str) -> (Map<String, Value>, &'static [&'static str]) {
    let required: &'static [&'static str] = match name {
        "process.exec" => &["program"],
        "process.batchExec" => &["elements"],
        "process.get" => &["sessionId"],
        "process.kill" => &["sessionId"],
        "file.read" => &["path"],
        "mcp.callTool" => &["serverId", "toolName"],
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
        "room.notebook.append" => &["scope", "significance", "abstract", "content"],
        "room.notebook.selectExact" => &["year", "month", "day"],
        "room.notebook.search" => &["query"],
        "room.notebook.current" => &["scope"],
        "room.notebook.update" | "room.notebook.remove" => &["id"],
        "room.diary.append" => &["entry"],
        "room.diary.selectExact" => &["year", "month", "day"],
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
            add("path", string("File or directory path within pathPolicy."));
            add(
                "includeContent",
                boolean("Return bounded UTF-8 content; defaults to true."),
            );
            add("startLine", number("1-based inclusive start line."));
            add("endLine", number("1-based inclusive end line."));
        }
        "mcp.list" => add("serverId", string("Optional configured MCP server id.")),
        "process.exec" => {
            add("program", string("Executable name or path."));
            add("args", strings("Direct argument vector."));
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
        "process.batchExec" => {
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
                "workingDirectory",
                string("Default process working directory."),
            );
            add(
                "waitSeconds",
                number("Bounded inline wait in seconds, capped at 30."),
            );
        }
        "process.get" => {
            add("sessionId", string("Managed session id."));
            add(
                "waitSeconds",
                number("Bounded wait in seconds, capped at 30."),
            );
        }
        "process.kill" => {
            add("sessionId", string("Managed session id."));
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
        "mcp.callTool" => {
            add("serverId", string("Configured MCP server id."));
            add("toolName", string("Downstream MCP tool name."));
            add(
                "arguments",
                json!({"type": "object", "description": "Downstream tool arguments."}),
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
            add("args", strings("Script argument vector."));
            add("workingDirectory", string("Optional working directory."));
            add("waitSeconds", number("Bounded inline wait, capped at 30."));
        }
        "room.notebook.append" => {
            add("datetime", string("Optional ISO-8601 timestamp."));
            add("scope", string("Path-safe notebook namespace."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
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
            add("year", number("Calendar year."));
            add("month", number("Calendar month."));
            add("day", number("Calendar day."));
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
            add("timeHint", string("Optional daypart label."));
            add("tags", strings("Optional labels."));
            add("entry", string("Diary entry text."));
        }
        "room.diary.recent" => {
            add("days", number("Logical diary days to scan."));
            add("limit", number("Maximum entries."));
        }
        "room.diary.selectExact" => {
            add("year", number("Diary year."));
            add("month", number("Diary month."));
            add("day", number("Diary day."));
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
        "agent.info" => "Inspect bounded local Agent identity, capabilities, policy, capacity, confirmation, connection, and config health.".to_string(),
        "file.read" => "Read bounded UTF-8 file content or metadata under path policy.".to_string(),
        "process.exec" => "Start one managed local process and wait briefly.".to_string(),
        "process.batchExec" => "Start multiple managed local processes.".to_string(),
        "process.get" => "Inspect or briefly wait for one managed local process.".to_string(),
        "process.kill" => "Kill one managed local process.".to_string(),
        "process.list" => "List active managed local processes.".to_string(),
        "session.start" => "Start a managed local process session.".to_string(),
        "session.list" => "List local managed sessions.".to_string(),
        "session.inspect" => "Inspect a managed session.".to_string(),
        "session.wait" => "Wait for a managed session with a bounded timeout.".to_string(),
        "session.kill" => "Kill a managed session.".to_string(),
        "tmux.listSessions" => "List persistent tmux sessions.".to_string(),
        "tmux.sessions" => "List, create, or close tmux sessions.".to_string(),
        "tmux.listPanes" => "List tmux panes.".to_string(),
        "tmux.panes" => "List or capture tmux panes.".to_string(),
        "tmux.capturePane" => "Capture bounded tmux pane history.".to_string(),
        "tmux.pasteText" => "Paste text into a tmux pane.".to_string(),
        "tmux.exec" => "Submit a command to a tmux shell pane.".to_string(),
        "tmux.createSession" => "Create a tmux session.".to_string(),
        "tmux.closeSession" => "Close a tmux session.".to_string(),
        "mcp.listServers" => "List configured downstream MCP servers.".to_string(),
        "mcp.listTools" => "List tools exposed by a downstream MCP server.".to_string(),
        "mcp.list" => "List configured MCP servers or one server's tools.".to_string(),
        "mcp.callTool" => "Call a downstream MCP tool.".to_string(),
        "bootstrap" => "Load the local bootstrap manifest.".to_string(),
        "bootstrap.read" => "Read one local bootstrap guide.".to_string(),
        "skills.list" => "List local skills.".to_string(),
        "skills.setActive" => "Activate or deactivate one local skill.".to_string(),
        "skills.read" => "Read a local skill or resource.".to_string(),
        "skills.search" => "Search local skills.".to_string(),
        "skills.active" => "List active local skills.".to_string(),
        "skills.activate" => "Activate a local skill.".to_string(),
        "skills.deactivate" => "Deactivate a local skill.".to_string(),
        "skills.install" => "Start a local skill installation.".to_string(),
        "skills.install.get" => "Get local skill installation status.".to_string(),
        "skills.install.cancel" => "Cancel a local skill installation.".to_string(),
        "skills.run" => "Run an executable from a local skill.".to_string(),
        "room.notebook.append" => "Append a Room notebook passage.".to_string(),
        "room.notebook.recent" => "Read recent Room notebook passages.".to_string(),
        "room.notebook.selectExact" => "Read Room notebook passages for a date.".to_string(),
        "room.notebook.search" => "Search Room notebook passages.".to_string(),
        "room.notebook.current" => "Read current Room notebook state.".to_string(),
        "room.notebook.update" => "Update a Room notebook passage.".to_string(),
        "room.notebook.remove" => "Remove a Room notebook passage.".to_string(),
        "room.diary.append" => "Append a Room diary entry.".to_string(),
        "room.diary.recent" => "Read recent Room diary entries.".to_string(),
        "room.diary.selectExact" => "Read Room diary entries for a date.".to_string(),
        _ => "Agentic GPT local tool.".to_string(),
    }
}

fn tool_is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "process.exec"
            | "process.batchExec"
            | "session.kill"
            | "process.kill"
            | "tmux.sessions"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
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
            | "file.batch"
    )
}

fn tool_is_destructive(name: &str) -> bool {
    matches!(
        name,
        "file.batch"
            | "file.edit"
            | "session.kill"
            | "process.kill"
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
            | "process.batchExec"
            | "tmux.sessions"
            | "mcp.callTool"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "skills.install"
            | "skills.run"
    )
}

pub(crate) fn standalone_surface(profile: CapabilityProfile) -> (Vec<String>, String) {
    let mut names = NORMAL_TOOLS.to_vec();
    if profile == CapabilityProfile::Room {
        names.extend_from_slice(ROOM_BOOTSTRAP_TOOLS);
        names.extend_from_slice(ROOM_ONLY_TOOLS);
    }
    names.sort_unstable();
    let names = names.into_iter().map(str::to_string).collect::<Vec<_>>();
    let tools = names
        .iter()
        .map(|name| {
            let (properties, required) = tool_schema(name);
            json!({
                "name": name,
                "inputSchema": schema(properties, required),
                "annotations": {
                    "readOnly": tool_is_read_only(name),
                    "destructive": tool_is_destructive(name),
                    "openWorld": tool_is_open_world(name),
                }
            })
        })
        .collect::<Vec<_>>();
    let canonical = json!({"surfaceSchemaVersion": 1, "tools": tools});
    let digest =
        Sha256::digest(serde_json::to_vec(&canonical).expect("standalone surface is serializable"));
    (names, format!("sha256:{digest:x}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use agentic_gpt_protocol::{AgentMessage, SkillActivationRequest};
    use rmcp::{model::CallToolRequestParams, ServiceExt};
    use tokio::io::split;
    use tokio::sync::{mpsc, Mutex, RwLock};

    use super::*;
    use crate::{
        config::Config, sessions::SkillLeaseManager, skill_installs::InstallManager,
        state::RuntimeModel,
    };

    #[test]
    fn normal_and_room_tool_sets_are_exact() {
        let normal = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let room = StdioMcpServer::new(test_state(CapabilityProfile::Room));
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
        assert_eq!(normal_names.len(), 19);
        assert_eq!(room_names.len(), 31);
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
        assert!(serialized.contains("mcp.list"));
        assert!(serialized.contains("skills.setActive"));
        assert!(serialized.contains("tmux.sessions"));
        assert!(serialized.contains("tmux.panes"));
    }

    #[test]
    fn compact_tool_schema_budgets_hold() {
        let normal = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let room = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        for (label, tools, max_total, max_inputs) in [
            ("normal", normal.tools.as_ref(), 11_500usize, 6_200usize),
            ("room", room.tools.as_ref(), 18_000usize, 9_600usize),
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

    #[tokio::test]
    async fn absent_room_tool_is_rejected_for_normal_worker() {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let error = server
            .call(CallToolRequestParams::new("room.diary.recent"))
            .await
            .expect_err("Room-only tool must not be callable by Normal worker");
        assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn in_process_stdio_initialize_list_and_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let server_task = tokio::spawn(async move {
            let running = server.serve((server_read, server_write)).await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 19);
        let result = client
            .call_tool(CallToolRequestParams::new("process.list"))
            .await?;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result.structured_content.as_ref().unwrap()["sessions"],
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
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        let server_task = tokio::spawn(async move {
            let running = server.serve((server_read, server_write)).await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 31);
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
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
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

        let removed = server
            .call(CallToolRequestParams::new("session.list"))
            .await
            .expect_err("Removed Tunnel aliases must not be callable");
        assert_eq!(removed.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn process_exec_get_kill_and_batch_use_managed_sessions() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let quick = server
            .dispatch("process.exec", json!({"program": "true", "waitSeconds": 5}))
            .await?;
        assert_eq!(quick["completedInline"], json!(true));
        assert!(quick.get("agentId").is_none());
        let quick_session = quick["sessionId"].as_str().unwrap().to_string();
        let fetched = server
            .dispatch(
                "process.get",
                json!({"sessionId": quick_session, "waitSeconds": 0}),
            )
            .await?;
        assert_eq!(fetched["state"], json!("exited"));

        let long = server
            .dispatch(
                "process.exec",
                json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
            )
            .await?;
        assert_eq!(long["completedInline"], json!(false));
        let long_session = long["sessionId"].as_str().unwrap().to_string();
        let listed = server.dispatch("process.list", json!({})).await?;
        assert!(listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["sessionId"] == long_session));
        let killed = server
            .dispatch("process.kill", json!({"sessionId": long_session}))
            .await?;
        assert!(matches!(
            killed["state"].as_str(),
            Some("killed") | Some("failed") | Some("exited")
        ));

        let batch = server
            .dispatch(
                "process.batchExec",
                json!({
                    "elements": [
                        {"program": "true"},
                        {"program": "false"}
                    ],
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(batch["results"].as_array().unwrap().len(), 2);
        assert_eq!(batch["status"], json!("partial_failed"));
        assert!(batch["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["outcome"] == "managed"
                && result["process"]["sessionId"].is_string()));

        let rejected = server
            .dispatch(
                "process.batchExec",
                json!({
                    "elements": [
                        {"program": "true"},
                        {"program": "true", "workingDirectory": "/missing"}
                    ],
                    "waitSeconds": 0
                }),
            )
            .await?;
        assert_eq!(rejected["status"], json!("rejected"));
        assert_eq!(rejected["results"][0]["outcome"], json!("skipped"));
        assert_eq!(rejected["results"][1]["outcome"], json!("rejected"));
        Ok(())
    }

    #[tokio::test]
    async fn managed_batch_uses_one_confirmation_for_all_elements() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        {
            let mut config = server.state.config.write().await;
            config.confirmation_provider.set_legacy("hub").unwrap();
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
                "process.batchExec",
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
    async fn denied_managed_batch_creates_no_sessions() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        {
            let mut config = server.state.config.write().await;
            config.confirmation_provider.set_legacy("hub").unwrap();
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
                "process.batchExec",
                json!({
                    "elements": [{"program": "true"}, {"program": "true"}],
                    "needConfirm": true,
                    "waitSeconds": 5
                }),
            )
            .await?;
        assert_eq!(batch["status"], json!("rejected"));
        let sessions = server.dispatch("process.list", json!({})).await?;
        assert_eq!(sessions["sessions"], json!([]));
        responder.abort();
        Ok(())
    }

    #[tokio::test]
    async fn tmux_actions_reject_incompatible_fields() {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
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
        let session = SessionInfo {
            agent_id: "agent".to_string(),
            session_id: "session".to_string(),
            state: "exited".to_string(),
            program: "true".to_string(),
            args: vec!["sentinel-argument".to_string()],
            working_directory: Some("/sentinel/path".to_string()),
            command_preview: "true sentinel-argument".to_string(),
            started_at,
            updated_at: started_at + chrono::Duration::milliseconds(42),
            exit_code: Some(0),
            stdout_tail: "sentinel-stdout".to_string(),
            stderr_tail: "sentinel-stderr".to_string(),
            truncated: false,
            reject_reason: None,
        };
        let message = managed_terminal_event_message("normal", "tunnel:process.exec", &session);
        assert!(message.contains("durationMs=42"));
        let human_session_id = message
            .split("session=")
            .nth(1)
            .and_then(|value| value.split(';').next())
            .unwrap();
        assert_eq!(human_session_id.len(), 12);
        assert!(human_session_id
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert!(!message.contains("sessionId="));
        assert!(!message.contains("sentinel"));
    }

    #[test]
    fn inline_terminal_tracker_discards_pending_terminal_event() {
        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = emitted.clone();
        let tracker = HumanTerminalTracker::with_emitter(move |message| {
            sink.lock().unwrap().push(message);
        });
        let session = test_terminal_session();
        tracker.record("normal", "tunnel:process.exec", &session);
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
            let session = test_terminal_session();
            if terminal_first {
                tracker.record("normal", "tunnel:process.exec", &session);
                tracker.finish_response(false, Some("status=active".to_string()));
            } else {
                tracker.finish_response(false, Some("status=active".to_string()));
                tracker.record("normal", "tunnel:process.exec", &session);
            }
            let emitted = emitted.lock().unwrap();
            assert_eq!(emitted.len(), 2);
            assert_eq!(emitted[0], "status=active");
            assert!(emitted[1].starts_with("managed_session;"));
        }
    }

    #[test]
    fn concurrent_check_clear_enqueue_interleaving_is_linearizable() {
        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = emitted.clone();
        let tracker = Arc::new(HumanTerminalTracker::with_emitter(move |message| {
            sink.lock().unwrap().push(message);
        }));
        let session = test_terminal_session();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let record_tracker = tracker.clone();
            let record_barrier = barrier.clone();
            scope.spawn(move || {
                record_barrier.wait();
                record_tracker.record("normal", "tunnel:process.exec", &session);
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
        assert_eq!(emitted[1].matches("managed_session;").count(), 1);
    }

    fn test_terminal_session() -> SessionInfo {
        SessionInfo {
            agent_id: "agent".to_string(),
            session_id: "sess_0123456789abcdef".to_string(),
            state: "exited".to_string(),
            program: "true".to_string(),
            args: Vec::new(),
            working_directory: None,
            command_preview: "true".to_string(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
            exit_code: Some(0),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
        }
    }

    #[test]
    fn batch_lifecycle_detection_reads_nested_managed_process_session() {
        let active = json!({
            "results": [{"process": {"session": {"state": "running"}}}]
        });
        assert!(value_has_active_session(&active));
        assert!(!value_has_terminal_failure(&active));

        let failed = json!({
            "results": [{"process": {"session": {
                "state": "failed",
                "rejectReason": "spawn_failed"
            }}}]
        });
        assert!(!value_has_active_session(&failed));
        assert!(value_has_terminal_failure(&failed));
        assert_eq!(
            human_failure_reason(&failed, None).as_deref(),
            Some("spawn_failed")
        );
    }

    #[tokio::test]
    async fn tunnel_skill_audit_uses_tunnel_request_source() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
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
        assert_eq!(result["session"]["state"], json!("exited"));
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
        assert!(audit.contains("\"requestSource\":\"tunnel:skills.run\""));
        Ok(())
    }

    #[tokio::test]
    async fn room_profile_dispatches_room_memory_tools() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Room));
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
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        let cases = [
            (
                "room.notebook.append",
                json!({"scope":"x","significance":"Normal","abstract":"a","content":"c","agentId":"foreign"}),
            ),
            ("room.notebook.recent", json!({"agentId":"foreign"})),
            (
                "room.notebook.selectExact",
                json!({"year":2026,"month":7,"day":25,"agentId":"foreign"}),
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
                json!({"year":2026,"month":7,"day":25,"agentId":"foreign"}),
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
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let workspace = server.state.config.read().await.workspace_root.clone();
        std::fs::write(workspace.join("read-me.txt"), "first\nsecond\n")?;
        let content = server
            .dispatch(
                "file.read",
                json!({"path":"read-me.txt", "startLine": 2, "endLine": 2}),
            )
            .await?;
        assert_eq!(content["content"], "second\n");
        assert_eq!(content["totalLines"], 2);
        let metadata = server
            .dispatch(
                "file.read",
                json!({"path":"read-me.txt", "includeContent": false}),
            )
            .await?;
        assert!(metadata.get("content").is_none());
        assert!(metadata["revision"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        let missing = server
            .dispatch("file.read", json!({"path":"missing.txt"}))
            .await?;
        assert_eq!(missing["error"]["code"], "file_not_found");
        Ok(())
    }

    #[tokio::test]
    async fn compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes() -> anyhow::Result<()>
    {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let mcp = server.dispatch("mcp.list", json!({})).await?;
        assert!(mcp["servers"].is_array());
        let skills = server.dispatch("skills.list", json!({})).await?;
        assert!(skills["skills"].is_array());
        assert!(skills["activeSkills"].is_array());
        assert!(skills["warnings"].is_array());
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
            runtime: RuntimeModel::tunnel(profile, false),
            started_at: chrono::Utc::now(),
            supervised: true,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(SkillLeaseManager::new()),
            skill_installs: Arc::new(InstallManager::new()),
        }
    }
}
