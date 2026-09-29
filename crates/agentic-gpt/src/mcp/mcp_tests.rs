use super::*;
use crate::config::mcp_servers::{
    mutate_servers, server_config_revision, validate_server_configs, McpConfigCommand,
};
use crate::config::Config;
use agentic_gpt_protocol::{McpBatchMode, McpBatchRequest, McpBatchStatus};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

fn server(transport: &str, endpoint: Option<&str>) -> McpServerConfig {
    McpServerConfig {
        enabled: true,
        transport: transport.to_string(),
        url: endpoint.map(str::to_string),
        auth: None,
    }
}

#[derive(Clone)]
enum FakeBehavior {
    Fast,
    Delayed(u64),
    ToolError,
    Large,
    MediumLarge,
    WaitForCancel,
    IgnoreCancel,
}

#[derive(Default)]
struct FakeConcurrency {
    active: AtomicUsize,
    max_active: AtomicUsize,
}

impl FakeConcurrency {
    fn enter(self: &Arc<Self>) -> FakeConcurrencyGuard {
        let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self
            .max_active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (active > current).then_some(active)
            });
        FakeConcurrencyGuard {
            tracker: self.clone(),
        }
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::Acquire)
    }
}

struct FakeConcurrencyGuard {
    tracker: Arc<FakeConcurrency>,
}

impl Drop for FakeConcurrencyGuard {
    fn drop(&mut self) {
        self.tracker.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
struct FakeMcpServer {
    behavior: FakeBehavior,
    calls: Arc<std::sync::Mutex<Vec<Value>>>,
    request_ids: Arc<std::sync::Mutex<Vec<String>>>,
    cancelled_ids: Arc<std::sync::Mutex<Vec<String>>>,
    context_cancelled: Arc<AtomicBool>,
    concurrency: Arc<FakeConcurrency>,
    events: Arc<std::sync::Mutex<Vec<String>>>,
}

impl FakeMcpServer {
    fn new(behavior: FakeBehavior) -> Self {
        Self::with_concurrency(behavior, Arc::new(FakeConcurrency::default()))
    }

    fn with_concurrency(behavior: FakeBehavior, concurrency: Arc<FakeConcurrency>) -> Self {
        Self {
            behavior,
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            request_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
            cancelled_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
            context_cancelled: Arc::new(AtomicBool::new(false)),
            concurrency,
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl rmcp::ServerHandler for FakeMcpServer {
    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        let behavior = self.behavior.clone();
        let calls = self.calls.clone();
        let request_ids = self.request_ids.clone();
        let context_cancelled = self.context_cancelled.clone();
        let concurrency = self.concurrency.clone();
        let events = self.events.clone();
        async move {
            let arguments = Value::Object(request.arguments.unwrap_or_default());
            let label = arguments
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string();
            calls.lock().unwrap().push(arguments);
            request_ids.lock().unwrap().push(context.id.to_string());
            events.lock().unwrap().push(format!("start:{label}"));
            let _guard = concurrency.enter();
            let result = match behavior {
                FakeBehavior::Fast => {
                    Ok(rmcp::model::CallToolResult::structured(json!({"ok": true})))
                }
                FakeBehavior::Delayed(milliseconds) => {
                    sleep(Duration::from_millis(milliseconds)).await;
                    Ok(rmcp::model::CallToolResult::structured(json!({
                        "delayed": true
                    })))
                }
                FakeBehavior::ToolError => {
                    Ok(rmcp::model::CallToolResult::structured_error(json!({
                        "code": "fake_error",
                        "message": "fake tool error"
                    })))
                }
                FakeBehavior::Large => Ok(rmcp::model::CallToolResult::structured(json!({
                    "blob": "大".repeat(220_000)
                }))),
                FakeBehavior::MediumLarge => Ok(rmcp::model::CallToolResult::structured(json!({
                    "blob": "x".repeat(240_000)
                }))),
                FakeBehavior::WaitForCancel => {
                    context.ct.cancelled().await;
                    context_cancelled.store(true, Ordering::Release);
                    Err(rmcp::ErrorData::internal_error(
                        "cancelled by client".to_string(),
                        None,
                    ))
                }
                FakeBehavior::IgnoreCancel => {
                    sleep(Duration::from_secs(10)).await;
                    Ok(rmcp::model::CallToolResult::structured(
                        json!({"late": true}),
                    ))
                }
            };
            events.lock().unwrap().push(format!("end:{label}"));
            result
        }
    }

    fn on_cancelled(
        &self,
        notification: rmcp::model::CancelledNotificationParam,
        _context: rmcp::service::NotificationContext<rmcp::RoleServer>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.cancelled_ids
            .lock()
            .unwrap()
            .push(notification.request_id.to_string());
        std::future::ready(())
    }

    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new("fake-mcp", "test"))
    }
}

fn fake_factory(server: FakeMcpServer) -> McpClientFactory {
    Arc::new(move |_config| {
        let server = server.clone();
        Box::pin(async move {
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                if let Ok(running) = server.serve(server_io).await {
                    let _ = running.waiting().await;
                }
            });
            Ok(ClientInfo::default().serve(client_io).await?)
        })
    })
}

fn routing_factory(servers: std::collections::HashMap<String, FakeMcpServer>) -> McpClientFactory {
    Arc::new(move |config| {
        let key = config.url.unwrap_or_default();
        let server = servers.get(&key).cloned();
        Box::pin(async move {
            let server = server.ok_or_else(|| anyhow!("fake_server_not_found: {key}"))?;
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                if let Ok(running) = server.serve(server_io).await {
                    let _ = running.waiting().await;
                }
            });
            Ok(ClientInfo::default().serve(client_io).await?)
        })
    })
}

async fn managed_test_state(max_active_processes: usize) -> (AppState, PathBuf) {
    use std::collections::HashMap;
    use tokio::sync::{Mutex, RwLock};

    let root = std::env::temp_dir().join(format!(
        "agentic-managed-mcp-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config::default_config().unwrap();
    config.workspace_root = workspace;
    config.limits.max_active_processes =
        crate::config::MaxActiveProcesses::Explicit(max_active_processes);
    config.mcp_servers.insert(
        "fake".to_string(),
        McpServerConfig {
            enabled: true,
            transport: "stdio".to_string(),
            url: Some("fake-command".to_string()),
            auth: None,
        },
    );
    let private_state =
        crate::private_state::PrivateStatePaths::for_test(root.join("private-state"));
    let state = AppState {
        config_path: root.join("config.json"),
        config: Arc::new(RwLock::new(config)),
        private_state: private_state.clone(),
        process_history: crate::process_history::ProcessHistoryStore::open(&private_state),
        browser_runtime: None,
        runtime: crate::state::RuntimeModel::local(crate::state::CapabilityProfile::Normal),
        started_at: chrono::Utc::now(),
        boot_generation: "mcpboot00001".to_string(),
        supervised: false,
        file_locks: Arc::new(Mutex::new(HashMap::new())),
        processes: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        reporting_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
        room_repository_writes: Arc::new(Mutex::new(())),
        skills_writes: Arc::new(Mutex::new(())),
        skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
        skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
    };
    crate::confirmation::allow_mcp_server_for_test(&state, "fake").await;
    (state, root)
}

async fn add_fake_server(state: &AppState, server_id: &str, temporary_allow: bool) {
    state.config.write().await.mcp_servers.insert(
        server_id.to_string(),
        McpServerConfig {
            enabled: true,
            transport: "stdio".to_string(),
            url: Some(server_id.to_string()),
            auth: None,
        },
    );
    if temporary_allow {
        crate::confirmation::allow_mcp_server_for_test(state, server_id).await;
    }
}

fn batch_call(
    id: Option<&str>,
    server_id: &str,
    label: &str,
) -> agentic_gpt_protocol::McpBatchCall {
    agentic_gpt_protocol::McpBatchCall {
        id: id.map(str::to_string),
        server_id: server_id.to_string(),
        tool_name: "fake.tool".to_string(),
        arguments: json!({"label": label}),
    }
}

fn batch_request(
    calls: Vec<agentic_gpt_protocol::McpBatchCall>,
    mode: McpBatchMode,
    fail_fast: bool,
    wait_seconds: u64,
) -> McpBatchRequest {
    McpBatchRequest {
        agent_id: "test-agent".to_string(),
        group: None,
        calls,
        mode,
        fail_fast,
        wait_seconds: Some(wait_seconds),
        timeout_seconds: Some(30),
    }
}

fn managed_request(arguments: Value, wait_seconds: u64) -> McpCallToolRequest {
    managed_request_with_timeout(arguments, wait_seconds, 30)
}

fn managed_request_with_timeout(
    arguments: Value,
    wait_seconds: u64,
    timeout_seconds: u64,
) -> McpCallToolRequest {
    McpCallToolRequest {
        agent_id: "test-agent".to_string(),
        group: None,
        server_id: "fake".to_string(),
        tool_name: "fake.tool".to_string(),
        arguments,
        wait_seconds: Some(wait_seconds),
        timeout_seconds: Some(timeout_seconds),
    }
}

async fn wait_for_fake_request(server: &FakeMcpServer) {
    for _ in 0..200 {
        if !server.request_ids.lock().unwrap().is_empty() {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("fake downstream request did not start");
}

#[tokio::test]
async fn managed_mcp_fast_result_uses_real_rmcp_transport() {
    let (state, root) = managed_test_state(2).await;
    let fake = FakeMcpServer::new(FakeBehavior::Fast);
    let response = start_managed_call_with_factory(
        &state,
        managed_request(json!({"value": 1, "secret": "super-secret-value"}), 5),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    assert!(response.completed_inline);
    assert_eq!(response.status, ProcessState::Completed);
    assert_eq!(
        response.process.kind,
        agentic_gpt_protocol::ProcessKind::Mcp
    );
    assert_eq!(
        response.result.as_ref().unwrap()["structuredContent"]["ok"],
        true
    );
    assert_eq!(
        fake.calls.lock().unwrap().as_slice(),
        &[json!({"value": 1, "secret": "super-secret-value"})]
    );
    let audit =
        std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl")).unwrap();
    assert!(audit.contains("\"program\":\"mcp.callTool\""));
    assert!(audit.contains("\"mcpServerId\":\"fake\""));
    assert!(audit.contains("\"mcpToolName\":\"fake.tool\""));
    assert!(audit.contains("\"argumentKeys\":[\"secret\",\"value\"]"));
    assert!(audit.contains("\"argumentKeyCount\":2"));
    assert!(audit.contains("\"argumentKeysTruncated\":false"));
    assert!(audit.contains("\"argumentSha256\":\"sha256:"));
    assert!(audit.contains("\"resultSha256\":\"sha256:"));
    assert!(!audit.contains("super-secret-value"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_deferred_result_is_retained_for_process_get() {
    let (state, root) = managed_test_state(2).await;
    let response = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(FakeMcpServer::new(FakeBehavior::Delayed(150))),
    )
    .await
    .unwrap();
    assert!(!response.completed_inline);
    assert!(response.status.is_active());
    let detail = crate::process::get_process_detail(&state, &response.process.process_id, 2)
        .await
        .unwrap();
    assert_eq!(detail.process.state, ProcessState::Completed);
    assert!(detail.result_available);
    assert_eq!(detail.result.unwrap()["structuredContent"]["delayed"], true);
    let retrieved = crate::process::get_process_result(
        &state,
        agentic_gpt_protocol::ProcessResultRequest {
            process_id: response.process.process_id.clone(),
            max_bytes: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        &retrieved.status,
        &agentic_gpt_protocol::ProcessResultStatus::Complete
    ));
    assert!(retrieved.result_available);
    assert_eq!(
        retrieved.result.unwrap()["structuredContent"]["delayed"],
        true
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_cancel_while_waiting_for_hub_confirmation_cleans_pending_sender() {
    let (state, root) = managed_test_state(1).await;
    state.temporary_mcp_allows.lock().await.clear();
    state.config.write().await.confirmation_provider =
        crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    *state.hub_sender.lock().await = Some(sender);
    let fake = FakeMcpServer::new(FakeBehavior::Fast);
    let response = start_managed_call_with_factory(
        &state,
        managed_request(json!({"secret": "must-not-appear-in-confirmation"}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    match message {
        agentic_gpt_protocol::AgentMessage::ConfirmationRequest { payload, .. } => {
            assert!(payload
                .command_preview
                .contains("Argument keys (showing 1 of 1, truncated=false): [secret]"));
            assert!(payload
                .command_preview
                .contains("Argument SHA-256: sha256:"));
            assert!(!payload
                .command_preview
                .contains("must-not-appear-in-confirmation"));
        }
        other => panic!("unexpected message: {other:?}"),
    }
    assert_eq!(state.pending_confirmations.lock().await.len(), 1);
    let cancelled = crate::process::cancel_process(&state, &response.process.process_id)
        .await
        .unwrap();
    assert_eq!(cancelled.state, ProcessState::Cancelled);
    for _ in 0..100 {
        if state.pending_confirmations.lock().await.is_empty() {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(state.pending_confirmations.lock().await.is_empty());
    assert!(fake.calls.lock().unwrap().is_empty());
    let detail = crate::process::get_process_detail(&state, &response.process.process_id, 1)
        .await
        .unwrap();
    assert!(!detail.result_available);
    let result = crate::process::get_process_result(
        &state,
        agentic_gpt_protocol::ProcessResultRequest {
            process_id: response.process.process_id,
            max_bytes: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        &result.status,
        &agentic_gpt_protocol::ProcessResultStatus::Unavailable
    ));
    assert!(!result.result_available);
    assert!(result.result.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_tool_error_and_large_result_are_truthful() {
    let (state, root) = managed_test_state(2).await;
    let error = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 5),
        "local:mcp.callTool",
        None,
        fake_factory(FakeMcpServer::new(FakeBehavior::ToolError)),
    )
    .await
    .unwrap();
    assert_eq!(error.status, ProcessState::Failed);
    assert_eq!(error.error.as_ref().unwrap().code, "mcp_tool_error");
    assert_eq!(error.result.as_ref().unwrap()["isError"], true);

    let large = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 5),
        "local:mcp.callTool",
        None,
        fake_factory(FakeMcpServer::new(FakeBehavior::Large)),
    )
    .await
    .unwrap();
    assert_eq!(large.status, ProcessState::Completed);
    assert!(large.result.is_none());
    assert!(matches!(
        large.result_status.as_ref(),
        Some(agentic_gpt_protocol::ProcessResultStatus::TooLarge)
    ));
    assert!(!large.result_available);
    assert!(large.result_bytes.unwrap() > crate::process::MAX_MCP_RESULT_BYTES);
    assert!(large
        .result_sha256
        .as_deref()
        .unwrap()
        .starts_with("sha256:"));
    assert!(large.result_preview.as_deref().unwrap().contains("blob"));
    let unavailable = crate::process::get_process_result(
        &state,
        agentic_gpt_protocol::ProcessResultRequest {
            process_id: large.process.process_id.clone(),
            max_bytes: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        &unavailable.status,
        &agentic_gpt_protocol::ProcessResultStatus::TooLarge
    ));
    assert!(!unavailable.result_available);
    assert!(unavailable.result.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_timeout_sends_exact_cancel_notification() {
    let (state, root) = managed_test_state(1).await;
    let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
    let response = start_managed_call_with_factory(
        &state,
        managed_request_with_timeout(json!({}), 3, 1),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    assert_eq!(response.status, ProcessState::TimedOut);
    assert_eq!(
        response.process.termination_evidence.as_deref(),
        Some("mcp_timeout_cancel_notification_sent")
    );
    assert!(fake.context_cancelled.load(Ordering::Acquire));
    assert_eq!(
        fake.cancelled_ids.lock().unwrap().as_slice(),
        fake.request_ids.lock().unwrap().as_slice()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_user_cancel_observes_remote_cancellation() {
    let (state, root) = managed_test_state(1).await;
    let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
    let response = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    wait_for_fake_request(&fake).await;
    let cancelling = crate::process::cancel_process(&state, &response.process.process_id)
        .await
        .unwrap();
    assert!(matches!(
        cancelling.state,
        ProcessState::CancelRequested | ProcessState::Detached
    ));
    let detail = crate::process::get_process_detail(&state, &response.process.process_id, 3)
        .await
        .unwrap();
    assert_eq!(detail.process.state, ProcessState::Detached);
    assert_eq!(
        detail.process.cancel_outcome.as_deref(),
        Some("notification_sent")
    );
    assert_eq!(
        detail.process.termination_evidence.as_deref(),
        Some("transport_or_remote_error_after_cancel")
    );
    assert!(fake.context_cancelled.load(Ordering::Acquire));
    assert_eq!(
        fake.cancelled_ids.lock().unwrap().as_slice(),
        fake.request_ids.lock().unwrap().as_slice()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_cancel_without_terminal_evidence_becomes_detached() {
    let (state, root) = managed_test_state(1).await;
    let fake = FakeMcpServer::new(FakeBehavior::IgnoreCancel);
    let response = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    wait_for_fake_request(&fake).await;
    let cancelling = crate::process::cancel_process(&state, &response.process.process_id)
        .await
        .unwrap();
    assert_eq!(cancelling.cancel_outcome, "notification_sent");
    let detail = crate::process::get_process_detail(&state, &response.process.process_id, 4)
        .await
        .unwrap();
    assert_eq!(detail.process.state, ProcessState::Detached);
    assert_eq!(
        detail.process.termination_evidence.as_deref(),
        Some("transport_or_remote_error_after_cancel")
    );
    assert_eq!(fake.cancelled_ids.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn managed_mcp_shares_capacity_and_rejects_oversized_arguments() {
    let (state, root) = managed_test_state(1).await;
    let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
    let first = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(fake.clone()),
    )
    .await
    .unwrap();
    wait_for_fake_request(&fake).await;
    let capacity = start_managed_call_with_factory(
        &state,
        managed_request(json!({}), 0),
        "local:mcp.callTool",
        None,
        fake_factory(FakeMcpServer::new(FakeBehavior::Fast)),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(capacity.starts_with("max_active_processes_reached"));
    let oversized = start_managed_call_with_factory(
        &state,
        managed_request(
            json!({"blob": "x".repeat(crate::process::MAX_MCP_ARGUMENT_BYTES + 1)}),
            0,
        ),
        "local:mcp.callTool",
        None,
        fake_factory(FakeMcpServer::new(FakeBehavior::Fast)),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(oversized.starts_with("mcp_tool_arguments_too_large"));
    assert_eq!(
        crate::process::list_processes(
            &state,
            agentic_gpt_protocol::ProcessListRequest::default(),
        )
        .await
        .len(),
        1
    );
    let _ = crate::process::cancel_process(&state, &first.process.process_id).await;
    let _ = crate::process::get_process_detail(&state, &first.process.process_id, 3).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_batch_preflight_and_capacity_fail_atomically_before_confirmation() {
    let (state, root) = managed_test_state(20).await;
    let fake = FakeMcpServer::new(FakeBehavior::Fast);
    let factory = fake_factory(fake.clone());

    let duplicate = batch_request(
        vec![
            batch_call(Some("dup"), "fake", "a"),
            batch_call(Some("dup"), "fake", "b"),
        ],
        McpBatchMode::Parallel,
        false,
        0,
    );
    assert!(batch::start_managed_batch_with_factory(
        &state,
        duplicate,
        "local:mcp.batch",
        None,
        factory.clone(),
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("mcp_batch_call_id_duplicate"));

    let empty = batch_request(Vec::new(), McpBatchMode::Parallel, false, 0);
    assert!(batch::start_managed_batch_with_factory(
        &state,
        empty,
        "local:mcp.batch",
        None,
        factory.clone(),
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("mcp_batch_call_count_invalid"));

    let too_many = batch_request(
        (0..17)
            .map(|index| batch_call(Some(&format!("call-{index}")), "fake", "x"))
            .collect(),
        McpBatchMode::Parallel,
        false,
        0,
    );
    assert!(batch::start_managed_batch_with_factory(
        &state,
        too_many,
        "local:mcp.batch",
        None,
        factory.clone(),
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("mcp_batch_call_count_invalid"));

    let non_object = McpBatchRequest {
        calls: vec![agentic_gpt_protocol::McpBatchCall {
            id: Some("non-object".to_string()),
            server_id: "fake".to_string(),
            tool_name: "fake.tool".to_string(),
            arguments: json!([1, 2, 3]),
        }],
        ..batch_request(Vec::new(), McpBatchMode::Parallel, false, 0)
    };
    assert!(batch::start_managed_batch_with_factory(
        &state,
        non_object,
        "local:mcp.batch",
        None,
        factory.clone(),
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("mcp_tool_arguments_must_be_object"));

    let aggregate = McpBatchRequest {
        calls: (0..9)
            .map(|index| agentic_gpt_protocol::McpBatchCall {
                id: Some(format!("large-{index}")),
                server_id: "fake".to_string(),
                tool_name: "fake.tool".to_string(),
                arguments: json!({"blob": "x".repeat(245_000)}),
            })
            .collect(),
        ..batch_request(Vec::new(), McpBatchMode::Parallel, false, 0)
    };
    assert!(batch::start_managed_batch_with_factory(
        &state,
        aggregate,
        "local:mcp.batch",
        None,
        factory,
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("mcp_batch_arguments_too_large"));

    assert!(crate::process::list_processes(
        &state,
        agentic_gpt_protocol::ProcessListRequest::default(),
    )
    .await
    .is_empty());
    assert!(state.pending_confirmations.lock().await.is_empty());
    assert!(fake.calls.lock().unwrap().is_empty());

    let rejection_audit =
        std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl")).unwrap();
    let rejection_records = rejection_audit
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rejection_records.len(), 5);
    assert!(rejection_records.iter().all(|record| {
        record["tool"] == "mcp.batch"
            && record["outcome"] == "validation_rejected"
            && record["childProcessIds"] == json!([])
    }));
    assert!(!rejection_audit.contains("\"program\":\"mcp.callTool\""));

    let (capacity_state, capacity_root) = managed_test_state(1).await;
    let capacity_fake = FakeMcpServer::new(FakeBehavior::Fast);
    let capacity = batch_request(
        vec![
            batch_call(Some("a"), "fake", "a"),
            batch_call(Some("b"), "fake", "b"),
        ],
        McpBatchMode::Parallel,
        false,
        0,
    );
    assert!(batch::start_managed_batch_with_factory(
        &capacity_state,
        capacity,
        "local:mcp.batch",
        None,
        fake_factory(capacity_fake.clone()),
    )
    .await
    .unwrap_err()
    .to_string()
    .starts_with("max_active_processes_reached"));
    assert!(crate::process::list_processes(
        &capacity_state,
        agentic_gpt_protocol::ProcessListRequest::default(),
    )
    .await
    .is_empty());
    assert!(capacity_state.pending_confirmations.lock().await.is_empty());
    assert!(capacity_fake.calls.lock().unwrap().is_empty());
    let capacity_audit = std::fs::read_to_string(
        capacity_root
            .join("workspace")
            .join(".agentic-gpt-audit.jsonl"),
    )
    .unwrap();
    let capacity_records = capacity_audit
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(capacity_records.len(), 1);
    assert_eq!(capacity_records[0]["tool"], "mcp.batch");
    assert_eq!(capacity_records[0]["outcome"], "capacity_rejected");
    assert_eq!(
        capacity_records[0]["errorCode"],
        "max_active_processes_reached"
    );
    assert_eq!(capacity_records[0]["childProcessIds"], json!([]));
    assert!(!capacity_audit.contains("\"program\":\"mcp.callTool\""));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(capacity_root);
}

#[tokio::test]
async fn mcp_batch_sequential_fail_fast_preserves_order_and_audit_correlation() {
    let (state, root) = managed_test_state(10).await;
    add_fake_server(&state, "error", true).await;
    add_fake_server(&state, "fast", true).await;
    let error = FakeMcpServer::new(FakeBehavior::ToolError);
    let fast = FakeMcpServer::new(FakeBehavior::Fast);
    let response = batch::start_managed_batch_with_factory(
        &state,
        batch_request(
            vec![
                batch_call(Some("first"), "error", "first"),
                batch_call(Some("second"), "fast", "second"),
                batch_call(Some("third"), "fast", "third"),
            ],
            McpBatchMode::Sequential,
            true,
            5,
        ),
        "local:mcp.batch",
        None,
        routing_factory(std::collections::HashMap::from([
            ("error".to_string(), error.clone()),
            ("fast".to_string(), fast.clone()),
        ])),
    )
    .await
    .unwrap();

    assert!(response.completed_inline);
    assert_eq!(response.status, McpBatchStatus::CompletedWithErrors);
    assert_eq!(
        response
            .results
            .iter()
            .map(|result| result.id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("first"), Some("second"), Some("third")]
    );
    assert_eq!(
        response.results[0].process.process.state,
        ProcessState::Failed
    );
    assert_eq!(
        response.results[1].process.process.state,
        ProcessState::Skipped
    );
    assert_eq!(
        response.results[2].process.process.state,
        ProcessState::Skipped
    );
    for (index, result) in response.results.iter().enumerate() {
        assert_eq!(
            result.process.process.batch_id.as_deref(),
            Some(response.batch_id.as_str())
        );
        assert_eq!(result.process.process.batch_index, Some(index));
        assert_eq!(result.process.process.batch_call_id, result.id);
    }
    assert_eq!(error.calls.lock().unwrap().len(), 1);
    assert!(fast.calls.lock().unwrap().is_empty());

    let audit_path = root.join("workspace").join(".agentic-gpt-audit.jsonl");
    let audit = loop {
        if let Ok(audit) = std::fs::read_to_string(&audit_path) {
            if audit.contains("\"tool\":\"mcp.batch\"") {
                break audit;
            }
        }
        sleep(Duration::from_millis(10)).await;
    };
    let records = audit
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .filter(|record| record["tool"] == "mcp.batch")
            .count(),
        1
    );
    let child_records = records
        .iter()
        .filter(|record| record["program"] == "mcp.callTool")
        .collect::<Vec<_>>();
    assert_eq!(child_records.len(), 3);
    assert!(child_records
        .iter()
        .all(|record| record["batchId"] == response.batch_id));
    assert_eq!(
        child_records
            .iter()
            .map(|record| record["batchIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(!audit.contains("first\":\""));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_batch_parallel_enforces_per_server_and_global_concurrency() {
    let (state, root) = managed_test_state(20).await;
    let same_server = FakeMcpServer::new(FakeBehavior::Delayed(120));
    let response = batch::start_managed_batch_with_factory(
        &state,
        batch_request(
            (0..5)
                .map(|index| {
                    batch_call(
                        Some(&format!("same-{index}")),
                        "fake",
                        &format!("same-{index}"),
                    )
                })
                .collect(),
            McpBatchMode::Parallel,
            false,
            5,
        ),
        "local:mcp.batch",
        None,
        fake_factory(same_server.clone()),
    )
    .await
    .unwrap();
    assert_eq!(response.status, McpBatchStatus::Completed);
    assert_eq!(same_server.concurrency.max_active(), 2);

    let (global_state, global_root) = managed_test_state(20).await;
    let tracker = Arc::new(FakeConcurrency::default());
    let mut routes = std::collections::HashMap::new();
    let mut calls = Vec::new();
    for index in 0..10 {
        let server_id = format!("server-{index}");
        add_fake_server(&global_state, &server_id, true).await;
        routes.insert(
            server_id.clone(),
            FakeMcpServer::with_concurrency(FakeBehavior::Delayed(120), tracker.clone()),
        );
        calls.push(batch_call(
            Some(&format!("global-{index}")),
            &server_id,
            &format!("global-{index}"),
        ));
    }
    let response = batch::start_managed_batch_with_factory(
        &global_state,
        batch_request(calls, McpBatchMode::Parallel, false, 5),
        "local:mcp.batch",
        None,
        routing_factory(routes),
    )
    .await
    .unwrap();
    assert_eq!(response.status, McpBatchStatus::Completed);
    assert_eq!(tracker.max_active(), crate::process::MCP_GLOBAL_CONCURRENCY);
    assert_eq!(global_state.mcp_concurrency.active(), 0);
    assert_eq!(global_state.mcp_concurrency.queued(), 0);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(global_root);
}

#[tokio::test]
async fn mcp_batch_clips_late_results_to_the_aggregate_budget() {
    let (state, root) = managed_test_state(20).await;
    let fake = FakeMcpServer::new(FakeBehavior::MediumLarge);
    let response = batch::start_managed_batch_with_factory(
        &state,
        batch_request(
            (0..6)
                .map(|index| {
                    batch_call(
                        Some(&format!("large-{index}")),
                        "fake",
                        &format!("large-{index}"),
                    )
                })
                .collect(),
            McpBatchMode::Parallel,
            false,
            10,
        ),
        "local:mcp.batch",
        None,
        fake_factory(fake),
    )
    .await
    .unwrap();
    assert_eq!(response.status, McpBatchStatus::Completed);
    assert!(response.aggregate_truncated);
    assert!(response.aggregate_bytes.unwrap() <= McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES);
    assert_eq!(
        serde_json::to_vec(&response).unwrap().len(),
        response.aggregate_bytes.unwrap()
    );
    let retained = response
        .results
        .iter()
        .filter(|result| result.process.result.is_some())
        .count();
    let clipped = response
        .results
        .iter()
        .filter(|result| result.process.result.is_none() && result.result_omitted)
        .count();
    assert!(retained > 0);
    assert!(clipped > 0);
    assert!(response.results.last().unwrap().process.result.is_none());
    assert!(response
        .results
        .last()
        .unwrap()
        .process
        .result_sha256
        .as_deref()
        .unwrap()
        .starts_with("sha256:"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_batch_single_server_uses_one_confirmation_and_can_grant_temporary_allow() {
    let (state, root) = managed_test_state(10).await;
    state.temporary_mcp_allows.lock().await.clear();
    state.config.write().await.confirmation_provider =
        crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    *state.hub_sender.lock().await = Some(sender);
    let fake = FakeMcpServer::new(FakeBehavior::Fast);
    let task_state = state.clone();
    let task_fake = fake.clone();
    let task = tokio::spawn(async move {
        batch::start_managed_batch_with_factory(
            &task_state,
            batch_request(
                vec![
                    batch_call(Some("first"), "fake", "secret-first-value"),
                    batch_call(Some("second"), "fake", "secret-second-value"),
                ],
                McpBatchMode::Parallel,
                false,
                5,
            ),
            "local:mcp.batch",
            None,
            fake_factory(task_fake),
        )
        .await
    });
    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let request_id = match message {
        agentic_gpt_protocol::AgentMessage::ConfirmationRequest {
            request_id,
            payload,
            ..
        } => {
            assert_eq!(payload.program, "mcp.batch");
            assert_eq!(payload.kind.as_deref(), Some("mcpBatchSingleServer"));
            assert_eq!(payload.server_id.as_deref(), Some("fake"));
            assert_eq!(payload.tool_name, None);
            assert!(payload
                .command_preview
                .contains("Calls requiring confirmation: 2"));
            assert!(payload.command_preview.contains("#0 id=first"));
            assert!(payload.command_preview.contains("#1 id=second"));
            assert!(!payload.command_preview.contains("secret-first-value"));
            assert!(!payload.command_preview.contains("secret-second-value"));
            request_id
        }
        other => panic!("unexpected message: {other:?}"),
    };
    assert!(receiver.try_recv().is_err());
    state
        .pending_confirmations
        .lock()
        .await
        .remove(&request_id)
        .unwrap()
        .send("allow_mcp_server_15m".to_string())
        .unwrap();
    let response = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status, McpBatchStatus::Completed);
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    assert!(confirmation::temporary_mcp_allowed(&state, "fake").await);
    assert!(state.pending_confirmations.lock().await.is_empty());
    assert!(receiver.try_recv().is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_batch_multi_server_uses_one_non_scoped_confirmation_and_rejects_all() {
    let (state, root) = managed_test_state(10).await;
    state.temporary_mcp_allows.lock().await.clear();
    add_fake_server(&state, "second", false).await;
    state.config.write().await.confirmation_provider =
        crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    *state.hub_sender.lock().await = Some(sender);
    let first = FakeMcpServer::new(FakeBehavior::Fast);
    let second = FakeMcpServer::new(FakeBehavior::Fast);
    let task_state = state.clone();
    let task_first = first.clone();
    let task_second = second.clone();
    let task = tokio::spawn(async move {
        batch::start_managed_batch_with_factory(
            &task_state,
            batch_request(
                vec![
                    batch_call(Some("first"), "fake", "first-value"),
                    batch_call(Some("second"), "second", "second-value"),
                ],
                McpBatchMode::Parallel,
                false,
                5,
            ),
            "local:mcp.batch",
            None,
            routing_factory(std::collections::HashMap::from([
                ("fake-command".to_string(), task_first),
                ("second".to_string(), task_second),
            ])),
        )
        .await
    });
    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let request_id = match message {
        agentic_gpt_protocol::AgentMessage::ConfirmationRequest {
            request_id,
            payload,
            ..
        } => {
            assert_eq!(payload.kind.as_deref(), Some("mcpBatch"));
            assert_eq!(payload.server_id, None);
            assert_eq!(payload.tool_name, None);
            assert!(payload
                .command_preview
                .contains("Calls requiring confirmation: 2"));
            assert!(!payload.command_preview.contains("first-value"));
            assert!(!payload.command_preview.contains("second-value"));
            request_id
        }
        other => panic!("unexpected message: {other:?}"),
    };
    assert!(receiver.try_recv().is_err());
    state
        .pending_confirmations
        .lock()
        .await
        .remove(&request_id)
        .unwrap()
        .send("deny".to_string())
        .unwrap();
    let response = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status, McpBatchStatus::Rejected);
    assert!(response.completed_inline);
    assert_eq!(response.error.as_ref().unwrap().code, "mcp_batch_rejected");
    assert!(response
        .results
        .iter()
        .all(|result| result.process.process.state == ProcessState::Rejected));
    assert!(first.calls.lock().unwrap().is_empty());
    assert!(second.calls.lock().unwrap().is_empty());
    assert!(!confirmation::temporary_mcp_allowed(&state, "fake").await);
    assert!(!confirmation::temporary_mcp_allowed(&state, "second").await);
    assert!(state.pending_confirmations.lock().await.is_empty());
    assert!(receiver.try_recv().is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_batch_child_cancel_during_aggregate_confirmation_cancels_all_before_start() {
    let (state, root) = managed_test_state(10).await;
    state.temporary_mcp_allows.lock().await.clear();
    state.config.write().await.confirmation_provider =
        crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    *state.hub_sender.lock().await = Some(sender);
    let fake = FakeMcpServer::new(FakeBehavior::Fast);
    let task_state = state.clone();
    let task_fake = fake.clone();
    let task = tokio::spawn(async move {
        batch::start_managed_batch_with_factory(
            &task_state,
            batch_request(
                vec![
                    batch_call(Some("first"), "fake", "first"),
                    batch_call(Some("second"), "fake", "second"),
                ],
                McpBatchMode::Parallel,
                false,
                5,
            ),
            "local:mcp.batch",
            None,
            fake_factory(task_fake),
        )
        .await
    });
    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    match message {
        agentic_gpt_protocol::AgentMessage::ConfirmationRequest { payload, .. } => {
            assert_eq!(payload.kind.as_deref(), Some("mcpBatchSingleServer"));
        }
        other => panic!("unexpected message: {other:?}"),
    }
    let processes =
        crate::process::list_processes(&state, agentic_gpt_protocol::ProcessListRequest::default())
            .await;
    assert_eq!(processes.len(), 2);
    assert!(processes
        .iter()
        .all(|process| process.state == ProcessState::WaitingConfirmation));
    let cancelled_process_id = processes[0].process_id.clone();
    let cancelled = crate::process::cancel_process(&state, &cancelled_process_id)
        .await
        .unwrap();
    assert_eq!(cancelled.state, ProcessState::Cancelled);

    let response = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status, McpBatchStatus::Rejected);
    assert!(response.completed_inline);
    assert_eq!(response.error.as_ref().unwrap().code, "mcp_batch_rejected");
    assert!(response
        .results
        .iter()
        .all(|result| result.process.process.state == ProcessState::Cancelled));
    let directly_cancelled = response
        .results
        .iter()
        .find(|result| result.process.process.process_id == cancelled_process_id)
        .unwrap();
    assert_eq!(
        directly_cancelled
            .process
            .process
            .termination_evidence
            .as_deref(),
        Some("local_cancel_before_downstream_request")
    );
    assert!(response
        .results
        .iter()
        .filter(|result| result.process.process.process_id != cancelled_process_id)
        .all(|result| {
            result.process.process.termination_evidence.as_deref()
                == Some("aggregate_authorization_decision")
        }));
    for _ in 0..100 {
        if state.pending_confirmations.lock().await.is_empty() {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(state.pending_confirmations.lock().await.is_empty());
    assert!(fake.calls.lock().unwrap().is_empty());
    assert!(receiver.try_recv().is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn server_config_validation_is_complete_and_typed() {
    let mut servers = BTreeMap::new();
    servers.insert(
        "http-main".to_string(),
        server("streamable-http", Some("http://127.0.0.1:3000/mcp")),
    );
    servers.insert(
        "stdio_main".to_string(),
        server("stdio", Some("node ./server.mjs")),
    );
    assert!(validate_server_configs(&servers).is_ok());

    for (id, config, code) in [
        (
            "bad id",
            server("stdio", Some("echo ok")),
            "mcp_server_id_invalid",
        ),
        (
            "missing-http",
            server("streamable-http", None),
            "mcp_server_url_missing",
        ),
        (
            "bad-http",
            server("streamable-http", Some("file:///tmp/mcp.sock")),
            "mcp_server_url_invalid",
        ),
        (
            "spaced-http",
            server("streamable-http", Some(" https://example.test/mcp")),
            "mcp_server_url_invalid",
        ),
        (
            "missing-command",
            server("stdio", Some("  ")),
            "mcp_server_command_missing",
        ),
        (
            "bad-command",
            server("stdio", Some("echo\0bad")),
            "mcp_server_command_invalid",
        ),
        (
            "spaced-command",
            server("stdio", Some(" echo ok")),
            "mcp_server_command_invalid",
        ),
        (
            "unsupported",
            server("sse", Some("https://example.test/mcp")),
            "unsupported_mcp_transport",
        ),
    ] {
        let mut candidate = BTreeMap::new();
        candidate.insert(id.to_string(), config);
        let error = validate_server_configs(&candidate).unwrap_err().to_string();
        assert!(error.starts_with(code), "error={error}");
    }
}

#[test]
fn config_cli_rejects_invalid_server_without_writing_and_accepts_valid_server() {
    let root = std::env::temp_dir().join(format!(
        "agentic-mcp-config-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("config.json");
    let config = Config::default_config().unwrap();
    std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();

    let error = mutate_servers(
        path.clone(),
        McpConfigCommand::Add {
            server_id: "invalid".to_string(),
            url: "https://example.test/mcp".to_string(),
            transport: "sse".to_string(),
            enabled: true,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("unsupported_mcp_transport"));
    assert_eq!(std::fs::read(&path).unwrap(), before);

    mutate_servers(
        path.clone(),
        McpConfigCommand::Add {
            server_id: "valid-http".to_string(),
            url: "https://example.test/mcp".to_string(),
            transport: "streamable-http".to_string(),
            enabled: true,
        },
    )
    .unwrap();
    let written = Config::load(&path).unwrap();
    assert_eq!(
        written.mcp_servers["valid-http"].url.as_deref(),
        Some("https://example.test/mcp")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn server_config_revision_is_deterministic_and_content_sensitive() {
    let mut first = BTreeMap::new();
    first.insert(
        "b".to_string(),
        server("streamable-http", Some("https://b.example/mcp")),
    );
    first.insert("a".to_string(), server("stdio", Some("node a.mjs")));
    let mut second = BTreeMap::new();
    second.insert("a".to_string(), server("stdio", Some("node a.mjs")));
    second.insert(
        "b".to_string(),
        server("streamable-http", Some("https://b.example/mcp")),
    );
    assert_eq!(
        server_config_revision(&first),
        server_config_revision(&second)
    );
    second.get_mut("b").unwrap().enabled = false;
    assert_ne!(
        server_config_revision(&first),
        server_config_revision(&second)
    );
}

#[test]
fn streamable_http_bearer_auth_is_validated_and_injected() {
    let mut http = server("streamable-http", Some("https://example.test/mcp"));
    http.auth = Some(McpServerAuthConfig::Bearer {
        token: "secret-token".to_string(),
    });
    let mut servers = BTreeMap::new();
    servers.insert("secured".to_string(), http.clone());
    validate_server_configs(&servers).unwrap();
    assert_eq!(
        streamable_http_transport_config(&http)
            .unwrap()
            .auth_header
            .as_deref(),
        Some("secret-token")
    );
    assert!(!format!("{http:?}").contains("secret-token"));

    let mut stdio = server("stdio", Some("node server.mjs"));
    stdio.auth = http.auth;
    servers.clear();
    servers.insert("local".to_string(), stdio);
    assert!(validate_server_configs(&servers)
        .unwrap_err()
        .to_string()
        .starts_with("mcp_server_auth_unsupported"));
}
