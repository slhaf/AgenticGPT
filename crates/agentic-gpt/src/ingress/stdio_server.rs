#[path = "stdio_schema.rs"]
mod stdio_schema;
#[path = "stdio_transport.rs"]
mod stdio_transport;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use agentic_gpt_protocol::{
    normalize_process_group, HubCommand, ProcessBatchExecRequest, ProcessCancelRequest,
    ProcessExecElement, ProcessExecRequest, ProcessInfo, ProcessListRequest, ProcessReadRequest,
};
#[cfg(test)]
use agentic_gpt_protocol::{McpBatchResponse, ProcessDetail};
use anyhow::Result;
use base64::Engine;
use chrono::Utc;
#[cfg(test)]
use rmcp::model::Meta;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, ErrorData, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext as McpRequestContext, RoleServer},
    transport::{async_rw::AsyncRwTransport, stdio},
    ServerHandler, ServiceExt,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(test)]
use crate::config::ToolNamespace;
use crate::{
    local_service,
    operation::{self, tool_namespace, AdmissionError, RequestContext, RequestIngress},
    operation_result::{
        rejection_error, slim_mcp_batch_response, slim_mcp_response, slim_process_response,
    },
    state::{AppState, CapabilityProfile},
};
use stdio_schema::{properties_for, tool_descriptor, tool_descriptors};

const INSTRUCTIONS: &str = "先用 agent.info 查看当前 profile、工作区、路径策略、容量、连接与确认通道；仅调用当前 tools/list 暴露的工具，按各工具的 schema 和说明构造参数。file.read/search 用于有界读取与搜索，file.edit 接受 Codex apply_patch。process.exec/batch 和 skills.run 启动受管理命令/脚本，用 process.read 统一读取状态与可用输出/结果，使用 process.cancel 请求取消。mcp.list 发现服务器/工具，mcp.callTool/batch 调用下游并登记 Process；等待到期不等于取消，已发生的外部副作用不回滚。tmux 用于持久终端；skills 包含 workspace 技能及只读、不可运行的内置 skill-installer，激活不执行代码或授予权限；bootstrap 与 room 工具用于 Room 引导和语义文档。Browser 先读 browser.manual，acquire 后以同名 repl 执行官方 SDK JavaScript；list 查租约，reset 仅恢复，release 最终清理。工具注解只是提示，不是授权；各操作受其实际策略、确认及资源限制控制，不构成对下游 MCP、tmux 或 Browser JavaScript 的通用沙箱。";
const BROWSER_REPL_RESULT_MARKER: &str = "__agentic_browser_repl_result";

pub(crate) async fn serve_stdio(state: AppState) -> Result<()> {
    let server = AgentMcpServer::new(state);
    let (stdin, stdout) = stdio();
    let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(stdin, stdout);
    let running = server
        .serve(stdio_transport::ResumableStdioTransport::new(transport))
        .await?;
    let _ = running.waiting().await?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct AgentMcpServer {
    state: AppState,
    ingress: RequestIngress,
    browser_repl_results: Arc<Mutex<HashMap<String, CallToolResult>>>,
}

struct BrowserAuditContext {
    tool: &'static str,
    lease_name: String,
    runtime_app_version: Option<String>,
    title: Option<String>,
    code_bytes: Option<usize>,
    code_sha256: Option<String>,
    timeout_ms: Option<u64>,
    idle_timeout_seconds: Option<u64>,
    outcome: String,
    error_code: Option<String>,
    started: Instant,
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

    fn record(&self, profile: &str, source: &str, process: &ProcessInfo) {
        let message = managed_terminal_event_message(profile, source, process);
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
        Self {
            state,
            ingress,
            browser_repl_results: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn current_tools(&self) -> Vec<Tool> {
        let config = self.state.config.read().await;
        tool_descriptors(&config.toolsets)
    }

    async fn tool_is_available(&self, name: &str) -> bool {
        if operation::is_event_api_operation(name) {
            return true;
        }
        if name == "privateevent.inject" {
            return self.ingress == RequestIngress::LocalUnix;
        }
        let Some(namespace) = tool_namespace(name) else {
            return false;
        };
        self.state
            .config
            .read()
            .await
            .toolsets
            .is_enabled(namespace)
    }

    fn take_browser_repl_result(&self, value: &mut Value) -> Option<CallToolResult> {
        let marker = value
            .as_object_mut()
            .and_then(|object| object.remove(BROWSER_REPL_RESULT_MARKER))
            .and_then(|value| value.as_str().map(str::to_owned))?;
        self.browser_repl_results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&marker)
    }

    fn stash_browser_repl_result(&self, result: CallToolResult) -> String {
        let marker = task_id("browser_repl");
        self.browser_repl_results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(marker.clone(), result);
        marker
    }
    #[cfg(test)]
    async fn call(&self, request: CallToolRequestParams) -> Result<Value, ErrorData> {
        self.call_with_result(request).await.map(|(value, _)| value)
    }

    async fn call_with_result(
        &self,
        request: CallToolRequestParams,
    ) -> Result<(Value, Option<CallToolResult>), ErrorData> {
        let name = request.name.to_string();
        if !self.tool_is_available(&name).await {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("Tool is not available: {name}"),
                None,
            ));
        }

        if !matches!(
            name.as_str(),
            "privateevent.inject" | "event.panel" | "event.settle"
        ) {
            if let Err(error) =
                crate::event_notifications::drain_completion_notifications(&self.state).await
            {
                crate::utils::log_warn(format!("event completion drain failed: {error}"));
            }
        }

        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let run_id = task_id("run");
        let report_request_id = task_id("req");
        let started_at = Utc::now();
        let terminal_tracker = Arc::new(HumanTerminalTracker::default());
        let reported_arguments = if name == "browser.repl" {
            browser_report_arguments(&arguments)
        } else {
            arguments.clone()
        };
        crate::hub::report_tool_arguments(
            &self.state,
            &run_id,
            &report_request_id,
            &name,
            reported_arguments,
            started_at,
        );
        let (mut value, file_read_result) = match self
            .dispatch_with_lifecycle(&name, arguments, terminal_tracker.clone())
            .await
        {
            Ok(result) => result,
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
                    None,
                    None,
                );
                return Err(ErrorData::invalid_params(error.to_string(), None));
            }
        };
        let browser_repl_result = self.take_browser_repl_result(&mut value);
        let mut special_result = browser_repl_result.or(file_read_result);
        if self.ingress != RequestIngress::Hub
            && !matches!(
                name.as_str(),
                "privateevent.inject" | "event.panel" | "event.settle"
            )
        {
            let panel = match self.state.event_store.panel() {
                Ok(panel) => panel,
                Err(error) => {
                    settle_unreturned_response(&self.state, &name, &value);
                    return Err(ErrorData::internal_error(error.to_string(), None));
                }
            };
            let decorate_result = if let Some(result) = special_result.as_mut() {
                crate::operation_result::attach_event_panel_to_tool_result(result, &panel)
            } else {
                crate::operation_result::attach_event_panel(&mut value, &panel)
            };
            if let Err(error) = decorate_result {
                settle_unreturned_response(&self.state, &name, &value);
                return Err(ErrorData::internal_error(error.to_string(), None));
            }
            crate::event_notifications::settle_initial_response(&self.state, &name, &value)
                .await
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        }
        let is_browser_repl = name == "browser.repl";
        let is_error = value.get("error").is_some()
            || (is_browser_repl
                && value
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false));
        let reason = if is_browser_repl {
            browser_error_code_from_value(&value)
        } else {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let process_id = value.get("processId").and_then(Value::as_str);
        let exit_code = value
            .get("exitCode")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok());
        let active = value_has_active_process(&value);
        let terminal_failure = value_has_terminal_failure(&value);
        let human_reason = reason.clone().or_else(|| human_failure_reason(&value));
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
        if let Some(process_id) = process_id {
            lifecycle.push_str(&format!(
                "; process={}",
                crate::utils::compact_id(process_id)
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
            if is_error || terminal_failure {
                "failed"
            } else {
                "completed"
            },
            started_at,
            if name == "browser.repl" {
                None
            } else {
                Some(value.clone())
            },
            reason,
            process_id.map(str::to_owned),
            exit_code,
            None,
        );
        Ok((value, special_result))
    }

    #[cfg(test)]
    async fn dispatch(&self, name: &str, arguments: Value) -> Result<Value> {
        let terminal_tracker = Arc::new(HumanTerminalTracker::default());
        let result = self
            .dispatch_with_lifecycle(name, arguments, terminal_tracker.clone())
            .await;
        let result = result.map(|(mut value, _)| {
            let _ = self.take_browser_repl_result(&mut value);
            value
        });
        terminal_tracker.finish_response(
            result
                .as_ref()
                .map(|value| !value_has_active_process(value))
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
    ) -> Result<(Value, Option<CallToolResult>)> {
        if name == "process.exec" {
            return self
                .dispatch_process_exec(arguments, terminal_tracker)
                .await
                .map(|value| (value, None));
        }
        if name == "process.batch" {
            return self
                .dispatch_process_batch(arguments, terminal_tracker)
                .await
                .map(|value| (value, None));
        }

        let admission = {
            let config = self.state.config.read().await;
            operation::authorize(
                self.state.runtime,
                &config,
                RequestContext::new(self.ingress, name),
            )
        };
        if let Err(error) = admission {
            return Ok((admission_error_value(error), None));
        }
        if tool_namespace(name).is_some() || operation::is_event_api_operation(name) {
            validate_stdio_arguments(name, &arguments)?;
        }
        let request_id = request_id();
        let mut file_read_result = None;
        let result = match name {
            "agent.info" => {
                let _: EmptyArgs = from_value(arguments)?;
                Ok(crate::agent_info::collect(&self.state).await)
            }
            "event.list" => {
                dispatch(
                    self,
                    HubCommand::EventList {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "event.get" => {
                dispatch(
                    self,
                    HubCommand::EventGet {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "event.mark" => {
                dispatch(
                    self,
                    HubCommand::EventMark {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "privateevent.inject" => {
                let request: agentic_gpt_protocol::EventInjectRequest = from_value(arguments)?;
                let low_ttl_seconds = self.state.config.read().await.events.low_ttl_seconds;
                map_result_value(
                    self.state.event_store.inject(&request, low_ttl_seconds),
                    "event_injection_failed",
                )
            }
            "browser.manual" => self.dispatch_browser_manual(arguments).await,
            "browser.acquire" => self.dispatch_browser_acquire(arguments).await,
            "browser.repl" => self.dispatch_browser_repl(arguments).await,
            "browser.reset" => self.dispatch_browser_reset(arguments).await,
            "browser.release" => self.dispatch_browser_release(arguments).await,
            "browser.list" => self.dispatch_browser_list(arguments).await,
            "file.read" => {
                let args: FileReadArgs = from_value(arguments)?;
                validate_file_read_args(&args)?;
                if let Some(requests) = args.requests {
                    let output = crate::file_ops::read_batch_with_images(
                        &self.state,
                        &requests.into_iter().map(Into::into).collect::<Vec<_>>(),
                    )
                    .await;
                    let (value, special_result) =
                        ordered_file_read_result(output.value, output.images_by_result, true);
                    file_read_result = special_result;
                    Ok(value)
                } else {
                    let config = self.state.config.read().await.clone();
                    let path = args.path.expect("validated single file.read path");
                    match crate::file_ops::resolve_path(
                        &config,
                        &path,
                        crate::file_ops::Access::Read,
                    ) {
                        Ok(resolved) => match crate::file_ops::read_with_images(
                            &resolved,
                            args.metadata.unwrap_or(false),
                            args.start_line,
                            args.end_line,
                            crate::file_ops::MAX_IMAGE_RESPONSE_BYTES,
                        ) {
                            Ok(output) => {
                                let (value, special_result) = ordered_file_read_result(
                                    output.value,
                                    vec![output.images],
                                    false,
                                );
                                file_read_result = special_result;
                                Ok(value)
                            }
                            Err(error) => Ok(error.value()),
                        },
                        Err(error) => Ok(error.value()),
                    }
                }
            }
            "file.search" => {
                let args: FileSearchArgs = from_value(arguments)?;
                validate_file_search_args(&args)?;
                if let Some(requests) = args.requests {
                    return Ok((
                        crate::file_ops::search_batch(
                            &self.state,
                            &requests.into_iter().map(Into::into).collect::<Vec<_>>(),
                        )
                        .await,
                        None,
                    ));
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
            "process.read" => self.dispatch_process_read(arguments).await,
            "process.cancel" => self.dispatch_process_cancel(arguments).await,
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
                            RequestContext::new(self.ingress, "tmux.createSession"),
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
                            RequestContext::new(self.ingress, "tmux.closeSession"),
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
                    RequestContext::new(self.ingress, "tmux.pasteText"),
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
                    RequestContext::new(self.ingress, "tmux.exec"),
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
                dispatch(
                    self,
                    HubCommand::McpListServers {
                        request_id,
                        suppress_event_panel: false,
                    },
                )
                .await
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
                    Err(error) => return Ok((error, None)),
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
                    None,
                )
                .await
                {
                    Ok(value) => {
                        let mut snapshots = Vec::new();
                        let response = slim_mcp_response(value, Some(&mut snapshots))?;
                        report_process_snapshots(&self.state, snapshots);
                        Ok(response)
                    }
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
                    Err(error) => return Ok((error, None)),
                };
                let config = self.state.config.read().await.clone();
                let response_budget = config.limits.process_response_bytes;
                let request_source = self.ingress.source("mcp.batch");
                match crate::mcp::batch::batch(
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
                    None,
                    response_budget,
                )
                .await
                {
                    Ok(value) => {
                        let mut snapshots = Vec::new();
                        let response = slim_mcp_batch_response(value, Some(&mut snapshots))?;
                        report_process_snapshots(&self.state, snapshots);
                        Ok(response)
                    }
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
            "room.diary.active" => {
                let request: agentic_gpt_protocol::RoomDiaryActiveRequest = from_value(arguments)?;
                let response = crate::room_reads::diary_active(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.diary.read" => {
                let request: agentic_gpt_protocol::RoomDiaryReadRequest = from_value(arguments)?;
                let response = crate::room_reads::diary_read(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.notebook.recent" => {
                let request: agentic_gpt_protocol::RoomNotebookRecentRequest =
                    from_value(arguments)?;
                let response = crate::room_reads::notebook_recent(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.notebook.search" => {
                let request: agentic_gpt_protocol::RoomNotebookSearchRequest =
                    from_value(arguments)?;
                let response = crate::room_reads::notebook_search(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.notebook.read" => {
                let request: agentic_gpt_protocol::RoomNotebookReadRequest = from_value(arguments)?;
                let response = crate::room_reads::notebook_read(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.state.list" => {
                let request: agentic_gpt_protocol::RoomStateListRequest = from_value(arguments)?;
                let response = crate::room_reads::state_list(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            "room.maintenance.status" => {
                let request: agentic_gpt_protocol::RoomMaintenanceStatusRequest =
                    from_value(arguments)?;
                match crate::room_maintenance::status(&self.state, request).await {
                    Ok(response) => Ok(serde_json::to_value(response)?),
                    Err(error) => Ok(local_service::room_maintenance_error("status", error)),
                }
            }
            "room.maintenance.submit" => {
                let request: agentic_gpt_protocol::RoomMaintenanceSubmitRequest =
                    from_value(arguments)?;
                match crate::room_maintenance::submit(&self.state, request).await {
                    Ok(response) => Ok(serde_json::to_value(response)?),
                    Err(error) => Ok(local_service::room_maintenance_error("submit", error)),
                }
            }
            "room.state.read" => {
                let request: agentic_gpt_protocol::RoomStateReadRequest = from_value(arguments)?;
                let response = crate::room_reads::state_read(&self.state, request).await?;
                Ok(serde_json::to_value(response)?)
            }
            _ => Err(anyhow::anyhow!("unknown agent tool: {name}")),
        };
        result.map(|value| (value, file_read_result))
    }

    async fn dispatch_process_exec(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let context = RequestContext::new(self.ingress, "process.exec");
        let request_source = context.source();
        let profile = self.state.runtime.profile;
        let mut snapshots = Vec::new();
        let response = local_service::dispatch_process(
            self.state.clone(),
            context,
            Some(&mut snapshots),
            move |config| {
                validate_stdio_arguments("process.exec", &arguments)?;
                let args: ProcessExecArgs = from_value(arguments)?;
                Ok(local_service::ProcessCall::Exec {
                    request: ProcessExecRequest {
                        agent_id: config.agent_id.clone(),
                        group: args.group,
                        command: args.command,
                        need_confirm: args.need_confirm,
                        confirm_method: None,
                        cwd: args.cwd,
                        wait_seconds: args.wait_seconds,
                    },
                    terminal_event_hook: Some(managed_terminal_event_hook(
                        profile,
                        request_source.clone(),
                        terminal_tracker,
                    )),
                })
            },
        )
        .await?;
        report_process_snapshots(&self.state, snapshots);
        Ok(response)
    }
    async fn dispatch_process_read(&self, arguments: Value) -> Result<Value> {
        let request: ProcessReadRequest = from_value(arguments)?;
        let mut snapshots = Vec::new();
        let response = local_service::dispatch_process_read(
            self.state.clone(),
            RequestContext::new(self.ingress, "process.read"),
            request,
            Some(&mut snapshots),
        )
        .await?;
        report_process_snapshots(&self.state, snapshots);
        Ok(response)
    }
    async fn dispatch_process_batch(
        &self,
        arguments: Value,
        terminal_tracker: Arc<HumanTerminalTracker>,
    ) -> Result<Value> {
        let context = RequestContext::new(self.ingress, "process.batch");
        let request_source = context.source();
        let profile = self.state.runtime.profile;
        let mut snapshots = Vec::new();
        let response = local_service::dispatch_process(
            self.state.clone(),
            context,
            Some(&mut snapshots),
            move |config| {
                validate_stdio_arguments("process.batch", &arguments)?;
                let args: ProcessBatchArgs = from_value(arguments)?;
                let request = ProcessBatchExecRequest {
                    agent_id: config.agent_id.clone(),
                    group: args.group,
                    elements: args
                        .elements
                        .into_iter()
                        .map(|element| ProcessExecElement {
                            command: element.command,
                            cwd: element.cwd,
                        })
                        .collect(),
                    need_confirm: args.need_confirm,
                    confirm_method: None,
                    cwd: args.cwd,
                    wait_seconds: args.wait_seconds,
                };
                Ok(local_service::ProcessCall::Batch {
                    request,
                    terminal_event_hook: Some(managed_terminal_event_hook(
                        profile,
                        request_source.clone(),
                        terminal_tracker,
                    )),
                })
            },
        )
        .await?;
        report_process_snapshots(&self.state, snapshots);
        Ok(response)
    }

    async fn dispatch_process_cancel(&self, arguments: Value) -> Result<Value> {
        let request: ProcessCancelRequest = from_value(arguments)?;
        match crate::process::cancel_process(&self.state, &request.process_id).await {
            Ok(response) => Ok(serde_json::to_value(response)?),
            Err(reason) => Ok(structured_error_from_reason(
                "process_cancel_failed",
                reason,
            )),
        }
    }

    async fn dispatch_process_list(&self, arguments: Value) -> Result<Value> {
        let mut request: ProcessListRequest = from_value(arguments)?;
        request.group = match normalize_stdio_group(request.group) {
            Ok(group) => group,
            Err(error) => return Ok(error),
        };
        match crate::process::get_process_list(&self.state, request).await {
            Ok(page) => Ok(serde_json::to_value(page)?),
            Err(reason) => Ok(structured_error_from_reason("process_list_failed", reason)),
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
        let request_source = self.ingress.source("skills.run");
        let terminal_event_hook = managed_terminal_event_hook(
            self.state.runtime.profile,
            request_source.clone(),
            terminal_tracker,
        );
        match crate::skills::run(
            self.state.clone(),
            request,
            &request_source,
            Some(terminal_event_hook),
            None,
        )
        .await
        {
            Ok(response) => {
                let mut snapshots = Vec::new();
                let value = slim_process_response(response, Some(&mut snapshots))?;
                report_process_snapshots(&self.state, snapshots);
                Ok(value)
            }
            Err(error) => Ok(crate::skills::skill_run_command_error(error)),
        }
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
        let mut skill_values = serde_json::to_value(skills)?;
        if let Some(skills) = skill_values.as_array_mut() {
            for skill in skills {
                remove_empty_warnings(skill);
            }
        }
        let mut response = json!({"skills": skill_values});
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
    async fn dispatch_browser_manual(&self, arguments: Value) -> Result<Value> {
        let args: BrowserManualArgs = from_value(arguments)?;
        let Some(runtime) = self.state.browser_runtime.clone() else {
            return Ok(browser_runtime_unavailable_value());
        };

        let value = match args.action.as_str() {
            "read" => match crate::browser_manual::read(
                &runtime.descriptor.docs_root,
                crate::browser_manual::ReadRequest {
                    path: args.path.expect("validated browser.manual read path"),
                    start_line: args.start_line,
                    end_line: args.end_line,
                },
            ) {
                Ok(response) => serde_json::to_value(response).unwrap_or_else(|error| {
                    browser_error_value(format!("browser_manual_serialize_failed:{error}"))
                }),
                Err(error) => browser_error_value(error),
            },
            "search" => match crate::browser_manual::search(
                &runtime.descriptor.docs_root,
                crate::browser_manual::SearchRequest {
                    query: args.query.expect("validated browser.manual search query"),
                    max_results: args.max_results,
                    context_lines: args.context_lines,
                },
            ) {
                Ok(response) => serde_json::to_value(response).unwrap_or_else(|error| {
                    browser_error_value(format!("browser_manual_serialize_failed:{error}"))
                }),
                Err(error) => browser_error_value(error),
            },
            _ => browser_error_value("browser_manual_action_invalid"),
        };
        Ok(value)
    }

    async fn dispatch_browser_acquire(&self, arguments: Value) -> Result<Value> {
        let args: BrowserAcquireArgs = from_value(arguments)?;
        let started = Instant::now();
        let name = args.name.clone();
        let idle_timeout_seconds = args.idle_timeout_seconds;
        let runtime = self.state.browser_runtime.clone();
        let runtime_app_version = runtime
            .as_ref()
            .map(|runtime| runtime.descriptor.app_version.clone());
        let value = match runtime {
            Some(runtime) => match runtime
                .manager
                .acquire(&name, Duration::from_secs(idle_timeout_seconds))
                .await
            {
                Ok(()) => json!({
                    "name": name,
                    "state": "ready",
                    "idleTimeoutSeconds": idle_timeout_seconds,
                    "appVersion": runtime.descriptor.app_version,
                    "channel": runtime.descriptor.channel,
                }),
                Err(error) => browser_error_value(error),
            },
            None => browser_runtime_unavailable_value(),
        };
        self.audit_browser(BrowserAuditContext {
            tool: "browser.acquire",
            lease_name: name,
            runtime_app_version,
            title: None,
            code_bytes: None,
            code_sha256: None,
            timeout_ms: None,
            idle_timeout_seconds: Some(idle_timeout_seconds),
            outcome: browser_outcome(&value),
            error_code: browser_error_code_from_value(&value),
            started,
        })
        .await;
        Ok(value)
    }

    async fn dispatch_browser_repl(&self, arguments: Value) -> Result<Value> {
        let args: BrowserReplArgs = from_value(arguments)?;
        let started = Instant::now();
        let name = args.name.clone();
        let code_bytes = args.code.len();
        let code_sha256 = Some(browser_sha256(&args.code));
        let timeout_ms = args.timeout_ms.unwrap_or(20_000);
        let title = args.title.as_deref().map(bounded_browser_title);
        let runtime = self.state.browser_runtime.clone();
        let runtime_app_version = runtime
            .as_ref()
            .map(|runtime| runtime.descriptor.app_version.clone());
        let (mut value, pending_result) = match runtime {
            Some(runtime) => match runtime.manager.repl(&name, &args.code, timeout_ms).await {
                Ok(result) => match serde_json::to_value(&result) {
                    Ok(value) if value.is_object() => (value, Some(result)),
                    Ok(_) => (browser_error_value("browser_repl_result_invalid"), None),
                    Err(error) => (
                        browser_error_value(format!(
                            "browser_repl_result_serialize_failed:{error}"
                        )),
                        None,
                    ),
                },
                Err(error) => (browser_error_value(error), None),
            },
            None => (browser_runtime_unavailable_value(), None),
        };
        self.audit_browser(BrowserAuditContext {
            tool: "browser.repl",
            lease_name: name,
            runtime_app_version,
            title,
            code_bytes: Some(code_bytes),
            code_sha256,
            timeout_ms: Some(timeout_ms),
            idle_timeout_seconds: None,
            outcome: browser_outcome_for_repl(&value),
            error_code: browser_error_code_from_value(&value),
            started,
        })
        .await;
        if let Some(result) = pending_result {
            let marker = self.stash_browser_repl_result(result);
            value
                .as_object_mut()
                .expect("serialized CallToolResult checked as object")
                .insert(
                    BROWSER_REPL_RESULT_MARKER.to_string(),
                    Value::String(marker),
                );
        }
        Ok(value)
    }

    async fn dispatch_browser_reset(&self, arguments: Value) -> Result<Value> {
        let args: BrowserLeaseArgs = from_value(arguments)?;
        let started = Instant::now();
        let name = args.name;
        let runtime = self.state.browser_runtime.clone();
        let runtime_app_version = runtime
            .as_ref()
            .map(|runtime| runtime.descriptor.app_version.clone());
        let value = match runtime {
            Some(runtime) => match runtime.manager.reset(&name).await {
                Ok(()) => json!({"name": name, "state": "ready"}),
                Err(error) => browser_error_value(error),
            },
            None => browser_runtime_unavailable_value(),
        };
        self.audit_browser(BrowserAuditContext {
            tool: "browser.reset",
            lease_name: name,
            runtime_app_version,
            title: None,
            code_bytes: None,
            code_sha256: None,
            timeout_ms: None,
            idle_timeout_seconds: None,
            outcome: browser_outcome(&value),
            error_code: browser_error_code_from_value(&value),
            started,
        })
        .await;
        Ok(value)
    }

    async fn dispatch_browser_release(&self, arguments: Value) -> Result<Value> {
        let args: BrowserLeaseArgs = from_value(arguments)?;
        let started = Instant::now();
        let name = args.name;
        let runtime = self.state.browser_runtime.clone();
        let runtime_app_version = runtime
            .as_ref()
            .map(|runtime| runtime.descriptor.app_version.clone());
        let value = match runtime {
            Some(runtime) => match runtime.manager.release(&name).await {
                Ok(released) => json!({"name": name, "released": released}),
                Err(error) => browser_error_value(error),
            },
            None => browser_runtime_unavailable_value(),
        };
        self.audit_browser(BrowserAuditContext {
            tool: "browser.release",
            lease_name: name,
            runtime_app_version,
            title: None,
            code_bytes: None,
            code_sha256: None,
            timeout_ms: None,
            idle_timeout_seconds: None,
            outcome: browser_outcome(&value),
            error_code: browser_error_code_from_value(&value),
            started,
        })
        .await;
        Ok(value)
    }

    async fn dispatch_browser_list(&self, arguments: Value) -> Result<Value> {
        let _: EmptyArgs = from_value(arguments)?;
        let Some(runtime) = self.state.browser_runtime.clone() else {
            return Ok(browser_runtime_unavailable_list_value());
        };
        let mut leases = runtime
            .manager
            .list()
            .await
            .into_iter()
            .map(browser_lease_value)
            .collect::<Vec<_>>();
        leases.sort_unstable_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        });
        Ok(json!({
            "runtimeAvailable": true,
            "appVersion": runtime.descriptor.app_version,
            "channel": runtime.descriptor.channel,
            "leases": leases,
        }))
    }

    async fn audit_browser(&self, context: BrowserAuditContext) {
        let BrowserAuditContext {
            tool,
            lease_name,
            runtime_app_version,
            title,
            code_bytes,
            code_sha256,
            timeout_ms,
            idle_timeout_seconds,
            outcome,
            error_code,
            started,
        } = context;
        let config = self.state.config.read().await.clone();
        let lease_name = bounded_browser_text(&lease_name, 128);
        let runtime_app_version = runtime_app_version
            .as_deref()
            .map(|value| bounded_browser_text(value, 128));
        let title = title.as_deref().map(bounded_browser_title);
        let _ = crate::audit::write_browser_audit(
            &config,
            crate::audit::BrowserAuditRecord {
                time: Utc::now(),
                tool: tool.to_string(),
                request_source: self.ingress.source(tool),
                lease_name,
                runtime_app_version,
                title,
                code_bytes,
                code_sha256,
                timeout_ms,
                idle_timeout_seconds,
                outcome,
                error_code,
                duration_ms: started.elapsed().as_millis(),
            },
        );
    }
}

impl ServerHandler for AgentMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: McpRequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let (value, special_result) = self.call_with_result(request).await?;
        if let Some(result) = special_result {
            return Ok(result);
        }
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
        _context: McpRequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: self.current_tools().await,
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if operation::is_event_api_operation(name)
            || (name == "privateevent.inject" && self.ingress == RequestIngress::LocalUnix)
        {
            return Some(tool_descriptor(name));
        }
        let namespace = tool_namespace(name)?;
        let config = self.state.config.try_read().ok()?;
        config
            .toolsets
            .is_enabled(namespace)
            .then(|| tool_descriptor(name))
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

fn ordered_file_read_result(
    value: Value,
    images_by_result: Vec<Vec<crate::file_ops::FileImageBlock>>,
    is_batch: bool,
) -> (Value, Option<CallToolResult>) {
    if is_batch {
        return ordered_file_read_batch_result(value, images_by_result);
    }
    if !images_by_result.iter().any(|images| !images.is_empty()) {
        return (value, None);
    }

    let mut content = vec![Content::text(value.to_string())];
    for images in images_by_result {
        for image in images {
            content.push(Content::image(
                base64::engine::general_purpose::STANDARD.encode(image.bytes),
                image.mime_type,
            ));
        }
    }

    let mut result = CallToolResult::structured(value.clone());
    result.content = content;
    let serialized_bytes = serde_json::to_vec(&result)
        .map(|serialized| serialized.len())
        .unwrap_or(usize::MAX);
    if serialized_bytes > crate::file_ops::MAX_IMAGE_RESPONSE_BYTES {
        return (
            json!({
                "error": {
                    "code": "file_image_response_too_large",
                    "message": "serialized MCP image response exceeds the 8 MiB bound"
                }
            }),
            None,
        );
    }
    (value, Some(result))
}

fn ordered_file_read_batch_result(
    mut value: Value,
    mut images_by_result: Vec<Vec<crate::file_ops::FileImageBlock>>,
) -> (Value, Option<CallToolResult>) {
    if !images_by_result.iter().any(|images| !images.is_empty()) {
        return (value, None);
    }
    let Some(result_count) = value["results"].as_array().map(Vec::len) else {
        return (value, None);
    };
    images_by_result.resize_with(result_count, Vec::new);
    images_by_result.truncate(result_count);

    let image_costs = batch_image_serialized_costs(&images_by_result);
    let mut include_images = vec![false; result_count];
    let mut image_bytes = 0usize;
    let mut serialized_bytes = batch_text_result_serialized_size(&value);
    for index in 0..result_count {
        if images_by_result[index].is_empty() {
            continue;
        }
        let cost = image_costs[index].unwrap_or(usize::MAX);
        let fits = serialized_bytes
            .checked_add(image_bytes)
            .and_then(|bytes| bytes.checked_add(cost))
            .is_some_and(|bytes| bytes <= crate::file_ops::MAX_IMAGE_RESPONSE_BYTES);
        if fits {
            include_images[index] = true;
            image_bytes += cost;
        } else {
            fail_batch_image_result(&mut value, index, &mut serialized_bytes);
        }
    }

    while serialized_bytes
        .checked_add(image_bytes)
        .is_none_or(|bytes| bytes > crate::file_ops::MAX_IMAGE_RESPONSE_BYTES)
    {
        let Some(index) = largest_included_image_group(&include_images, &image_costs) else {
            return (value, None);
        };
        include_images[index] = false;
        image_bytes = image_bytes.saturating_sub(image_costs[index].unwrap_or(usize::MAX));
        fail_batch_image_result(&mut value, index, &mut serialized_bytes);
    }
    if !include_images.iter().any(|include| *include) {
        return (value, None);
    }

    loop {
        let result = build_ordered_batch_image_result(&value, &images_by_result, &include_images);
        let actual_size = serde_json::to_vec(&result)
            .map(|serialized| serialized.len())
            .unwrap_or(usize::MAX);
        if actual_size <= crate::file_ops::MAX_IMAGE_RESPONSE_BYTES {
            return (value, Some(result));
        }
        let Some(index) = largest_included_image_group(&include_images, &image_costs) else {
            return (value, None);
        };
        include_images[index] = false;
        fail_batch_image_result(&mut value, index, &mut serialized_bytes);
    }
}

fn batch_text_result_serialized_size(value: &Value) -> usize {
    let Some(results) = value["results"].as_array() else {
        return usize::MAX;
    };
    let mut result = CallToolResult::structured(value.clone());
    result.content = results
        .iter()
        .map(|item| Content::text(item.to_string()))
        .collect();
    serde_json::to_vec(&result)
        .map(|serialized| serialized.len())
        .unwrap_or(usize::MAX)
}

fn batch_image_serialized_costs(
    images_by_result: &[Vec<crate::file_ops::FileImageBlock>],
) -> Vec<Option<usize>> {
    let mut block_overhead = HashMap::<&'static str, usize>::new();
    images_by_result
        .iter()
        .map(|images| {
            images.iter().try_fold(0usize, |total, image| {
                let encoded_bytes = image
                    .bytes
                    .len()
                    .checked_add(2)?
                    .checked_div(3)?
                    .checked_mul(4)?;
                let overhead = if let Some(overhead) = block_overhead.get(image.mime_type) {
                    *overhead
                } else {
                    let serialized =
                        serde_json::to_vec(&Content::image(String::new(), image.mime_type)).ok()?;
                    let overhead = serialized.len().checked_sub(2)?.checked_add(1)?;
                    block_overhead.insert(image.mime_type, overhead);
                    overhead
                };
                total.checked_add(encoded_bytes)?.checked_add(overhead)
            })
        })
        .collect()
}

fn fail_batch_image_result(value: &mut Value, index: usize, serialized_bytes: &mut usize) {
    let previous = value["results"][index].clone();
    let replacement = json!({
        "index": previous["index"],
        "status": "failed",
        "error": {
            "code": "file_image_response_too_large",
            "message": "image omitted to keep the ordered batch response within the 8 MiB bound"
        }
    });
    let previous_result_bytes = serialized_json_size(&previous);
    let replacement_result_bytes = serialized_json_size(&replacement);
    let previous_text_bytes = serialized_json_size(&Content::text(previous.to_string()));
    let replacement_text_bytes = serialized_json_size(&Content::text(replacement.to_string()));
    let mut adjusted_size = (*serialized_bytes)
        .checked_sub(previous_result_bytes)
        .and_then(|size| size.checked_sub(previous_text_bytes))
        .and_then(|size| size.checked_add(replacement_result_bytes))
        .and_then(|size| size.checked_add(replacement_text_bytes));
    let completed_with_errors = json!("completed_with_errors");
    if value["status"] != completed_with_errors {
        adjusted_size = adjusted_size
            .and_then(|size| size.checked_sub(serialized_json_size(&value["status"])))
            .and_then(|size| size.checked_add(serialized_json_size(&completed_with_errors)));
        value["status"] = completed_with_errors;
    }
    value["results"][index] = replacement;
    *serialized_bytes = adjusted_size.unwrap_or(usize::MAX);
}

fn serialized_json_size<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|serialized| serialized.len())
        .unwrap_or(usize::MAX)
}

fn largest_included_image_group(
    include_images: &[bool],
    image_costs: &[Option<usize>],
) -> Option<usize> {
    include_images
        .iter()
        .enumerate()
        .filter(|(_, include)| **include)
        .max_by_key(|(index, _)| image_costs[*index].unwrap_or(usize::MAX))
        .map(|(index, _)| index)
}

fn build_ordered_batch_image_result(
    value: &Value,
    images_by_result: &[Vec<crate::file_ops::FileImageBlock>],
    include_images: &[bool],
) -> CallToolResult {
    let results = value["results"].as_array().expect("batch result array");
    let mut content = Vec::new();
    for (index, item) in results.iter().enumerate() {
        content.push(Content::text(item.to_string()));
        if include_images.get(index).copied().unwrap_or(false) {
            for image in &images_by_result[index] {
                content.push(Content::image(
                    base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                    image.mime_type,
                ));
            }
        }
    }
    let mut result = CallToolResult::structured(value.clone());
    result.content = content;
    result
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessExecArgs {
    command: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchElementArgs {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessBatchArgs {
    elements: Vec<BatchElementArgs>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
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
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserManualArgs {
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    context_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserAcquireArgs {
    name: String,
    idle_timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserReplArgs {
    name: String,
    code: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserLeaseArgs {
    name: String,
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
    let operation_name = operation::hub_command_name(&command);
    let context = RequestContext::new(server.ingress, operation_name);
    local_service::dispatch(server.state.clone(), command, context, None).await
}
fn admission_error_value(error: AdmissionError) -> Value {
    json!({
        "error": {
            "code": error.code(),
            "message": error.message(),
        }
    })
}

fn structured_error_from_reason(default_code: &str, message: impl Into<String>) -> Value {
    let message = message.into();
    let mut error = rejection_error(&message);
    if error.code == "process_rejected" {
        error.code = default_code.to_string();
    }
    json!({"error": error})
}

fn normalize_stdio_group(group: Option<String>) -> std::result::Result<Option<String>, Value> {
    normalize_process_group(group.as_deref()).map_err(|error| {
        json!({
            "error": {
                "code": error.code(),
                "message": error.message()
            }
        })
    })
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
        "browser.manual" => {
            let args: BrowserManualArgs = from_value(arguments.clone())?;
            validate_browser_manual_args(&args, object)?;
        }
        "browser.acquire" => {
            let args: BrowserAcquireArgs = from_value(arguments.clone())?;
            validate_browser_acquire_args(&args)?;
        }
        "browser.repl" => {
            let args: BrowserReplArgs = from_value(arguments.clone())?;
            validate_browser_repl_args(&args)?;
        }
        "browser.reset" | "browser.release" => {
            let _: BrowserLeaseArgs = from_value(arguments.clone())?;
        }
        "browser.list" => {
            let _: EmptyArgs = from_value(arguments.clone())?;
        }
        "event.list" => {
            let _: agentic_gpt_protocol::EventListRequest = from_value(arguments.clone())?;
        }
        "event.get" => {
            let _: agentic_gpt_protocol::EventGetRequest = from_value(arguments.clone())?;
        }
        "event.mark" => {
            let _: agentic_gpt_protocol::EventMarkRequest = from_value(arguments.clone())?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_browser_manual_args(
    args: &BrowserManualArgs,
    object: &Map<String, Value>,
) -> Result<()> {
    let present = |name: &str| object.contains_key(name);
    match args.action.as_str() {
        "read" => {
            if args.path.is_none() {
                return Err(anyhow::anyhow!("browser.manual read requires path"));
            }
            if present("query") || present("maxResults") || present("contextLines") {
                return Err(anyhow::anyhow!("browser.manual read rejects search fields"));
            }
            if args.start_line == Some(0)
                || args.end_line == Some(0)
                || matches!(
                    (args.start_line, args.end_line),
                    (Some(start), Some(end)) if start > end
                )
            {
                return Err(anyhow::anyhow!("browser.manual invalid line range"));
            }
        }
        "search" => {
            if args.query.is_none() {
                return Err(anyhow::anyhow!("browser.manual search requires query"));
            }
            if present("path") || present("startLine") || present("endLine") {
                return Err(anyhow::anyhow!("browser.manual search rejects read fields"));
            }
            let query = args.query.as_deref().expect("query presence checked");
            if query.trim().is_empty() || query.len() > 4 * 1024 {
                return Err(anyhow::anyhow!("browser.manual invalid query"));
            }
            if args
                .max_results
                .is_some_and(|value| !(1..=100).contains(&value))
            {
                return Err(anyhow::anyhow!("browser.manual invalid maxResults"));
            }
            if args.context_lines.is_some_and(|value| value > 5) {
                return Err(anyhow::anyhow!("browser.manual invalid contextLines"));
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "browser.manual action must be read or search"
            ));
        }
    }
    Ok(())
}

fn validate_browser_acquire_args(args: &BrowserAcquireArgs) -> Result<()> {
    if !(1..=86_400).contains(&args.idle_timeout_seconds) {
        return Err(anyhow::anyhow!(
            "browser.acquire idleTimeoutSeconds must be 1..=86400"
        ));
    }
    Ok(())
}

fn validate_browser_repl_args(args: &BrowserReplArgs) -> Result<()> {
    if args.code.is_empty() {
        return Err(anyhow::anyhow!("browser.repl code must be non-empty"));
    }
    if args.code.len() > 256 * 1024 {
        return Err(anyhow::anyhow!(
            "browser.repl code exceeds 256 KiB UTF-8 bytes"
        ));
    }
    if args
        .timeout_ms
        .is_some_and(|value| !(1..=120_000).contains(&value))
    {
        return Err(anyhow::anyhow!("browser.repl timeoutMs must be 1..=120000"));
    }
    if args
        .title
        .as_deref()
        .is_some_and(|title| title.chars().count() > 128)
    {
        return Err(anyhow::anyhow!("browser.repl title exceeds 128 characters"));
    }
    Ok(())
}

fn browser_runtime_unavailable_value() -> Value {
    browser_error_value("browser_runtime_unavailable")
}

fn browser_runtime_unavailable_list_value() -> Value {
    json!({
        "runtimeAvailable": false,
        "leases": [],
    })
}

fn browser_error_value(reason: impl std::fmt::Display) -> Value {
    let code = browser_error_code(&reason.to_string());
    json!({
        "error": {
            "code": code.clone(),
            "message": code,
        }
    })
}

fn browser_error_code(reason: &str) -> String {
    let prefix = reason.split(':').next().unwrap_or(reason).trim();
    let code = prefix
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || *character == '_' || *character == '-'
        })
        .take(64)
        .collect::<String>();
    if code.is_empty() {
        "browser_error".to_string()
    } else {
        code
    }
}

fn browser_error_code_from_value(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(browser_error_code)
        .or_else(|| {
            value
                .get("structuredContent")
                .and_then(|content| content.get("error"))
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(browser_error_code)
        })
        .or_else(|| {
            value
                .get("isError")
                .and_then(Value::as_bool)
                .filter(|is_error| *is_error)
                .map(|_| "browser_repl_error".to_string())
        })
}

fn browser_outcome(value: &Value) -> String {
    if value.get("error").is_some() {
        "failed".to_string()
    } else {
        "completed".to_string()
    }
}

fn browser_outcome_for_repl(value: &Value) -> String {
    if value.get("error").is_some()
        || value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        "failed".to_string()
    } else {
        "completed".to_string()
    }
}

fn browser_sha256(code: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(code.as_bytes()))
}

fn bounded_browser_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

fn bounded_browser_title(title: &str) -> String {
    bounded_browser_text(title, 128)
}

fn browser_report_arguments(arguments: &Value) -> Value {
    let mut reported = Map::new();
    if let Some(name) = arguments.get("name").and_then(Value::as_str) {
        reported.insert(
            "name".to_string(),
            Value::String(bounded_browser_text(name, 128)),
        );
    }
    if let Some(code) = arguments.get("code").and_then(Value::as_str) {
        reported.insert("codeBytes".to_string(), json!(code.len()));
        reported.insert("codeSha256".to_string(), json!(browser_sha256(code)));
    }
    if let Some(timeout_ms) = arguments.get("timeoutMs").and_then(Value::as_u64) {
        reported.insert("timeoutMs".to_string(), json!(timeout_ms));
    }
    if let Some(title) = arguments.get("title").and_then(Value::as_str) {
        reported.insert(
            "title".to_string(),
            Value::String(bounded_browser_title(title)),
        );
    }
    Value::Object(reported)
}

fn browser_lease_value(snapshot: crate::browser_manager::BrowserLeaseSnapshot) -> Value {
    let mut value = json!({
        "name": snapshot.name,
        "state": browser_state_label(snapshot.state),
        "idleTimeoutSeconds": snapshot.idle_timeout.as_secs().min(86_400),
    });
    if let Some(remaining) = snapshot.remaining_idle {
        value["remainingIdleSeconds"] = json!(remaining.as_secs().min(86_400));
    }
    value
}

fn browser_state_label(state: crate::browser_manager::BrowserLeaseState) -> &'static str {
    match state {
        crate::browser_manager::BrowserLeaseState::Initializing => "initializing",
        crate::browser_manager::BrowserLeaseState::Ready => "ready",
        crate::browser_manager::BrowserLeaseState::Closing => "closing",
    }
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
fn settle_unreturned_response(state: &AppState, operation: &str, value: &Value) {
    let dispositions =
        match crate::event_notifications::initial_response_dispositions(operation, value) {
            Ok(dispositions) => dispositions,
            Err(error) => {
                crate::utils::log_warn(format!(
                    "unreturned event response metadata failed: {error}"
                ));
                return;
            }
        };
    for disposition in dispositions {
        match state
            .event_store
            .settle_response(&disposition.source, false)
        {
            Ok(()) => {}
            Err(error) if error.to_string() == "event_internal_source_not_registered" => {}
            Err(error) => {
                crate::utils::log_warn(format!(
                    "unreturned event response settlement failed: {error}"
                ));
            }
        }
    }
}

fn request_id() -> String {
    task_id("req")
}

fn task_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn value_has_active_process(value: &Value) -> bool {
    process_values(value).any(|process| {
        process
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(is_active_process_state)
    })
}

fn value_has_terminal_failure(value: &Value) -> bool {
    value.get("error").is_some()
        || process_values(value).any(|process| {
            process
                .get("state")
                .and_then(Value::as_str)
                .is_some_and(is_failure_process_state)
        })
}

fn human_failure_reason(value: &Value) -> Option<String> {
    process_values(value).find_map(|process| {
        process
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn process_values(value: &Value) -> impl Iterator<Item = &Value> {
    let wrapped = value.get("process").into_iter();
    let direct = value
        .get("processId")
        .and_then(|_| value.get("state"))
        .map(|_| value)
        .into_iter();
    let processes = value
        .get("processes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    wrapped.chain(direct).chain(processes).chain(results)
}

fn is_active_process_state(state: &str) -> bool {
    matches!(
        state,
        "queued" | "waiting_confirmation" | "starting" | "running" | "cancel_requested"
    )
}

fn is_failure_process_state(state: &str) -> bool {
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
) -> crate::process::TerminalEventHook {
    let source = source.into();
    let profile = profile.label();
    Arc::new(move |process| {
        tracker.record(profile, &source, process);
    })
}

fn report_process_snapshots(state: &AppState, snapshots: Vec<ProcessInfo>) {
    for process in snapshots {
        crate::hub::report_process(state, process);
    }
}

fn managed_terminal_event_message(profile: &str, source: &str, process: &ProcessInfo) -> String {
    let duration_ms = process
        .started_at
        .map(|started_at| (process.updated_at - started_at).num_milliseconds().max(0))
        .unwrap_or(0);
    let mut message = format!(
        "managed_process; source={source}; profile={profile}; status={}; process={}; durationMs={duration_ms}",
        process.state,
        crate::utils::compact_id(&process.process_id)
    );
    if let Some(exit_code) = process.exit_code {
        message.push_str(&format!("; exitCode={exit_code}"));
    }
    if let Some(reason) = process.reject_reason.as_deref() {
        message.push_str(&format!("; errorCode={}", bounded_error_code(reason)));
    }
    message
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
#[cfg(test)]
#[path = "stdio_server_tests.rs"]
mod tests;
