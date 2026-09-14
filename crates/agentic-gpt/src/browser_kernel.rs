use crate::browser_runtime::NodeReplLaunchSpec;
use anyhow::{anyhow, Result};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, Implementation,
        JsonObject, Meta, ProtocolVersion, RequestParamsMeta,
    },
    service::RunningService,
    transport::TokioChildProcess,
    RoleClient, ServiceExt,
};
use serde_json::{json, Map, Value};
use std::{path::Path, time::Duration};

fn node_repl_client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("agentic-browser-runtime", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2025_06_18)
}

fn command_from_launch_spec(spec: &NodeReplLaunchSpec) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&spec.program);
    command.current_dir(&spec.cwd);
    command.envs(&spec.env_overrides);
    command
}

fn validate_ids(session_id: &str, turn_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return Err(anyhow!("browser_runtime_session_id_invalid"));
    }
    if turn_id.trim().is_empty() {
        return Err(anyhow!("browser_runtime_turn_id_invalid"));
    }
    Ok(())
}

pub(crate) struct NodeReplKernel {
    client: RunningService<RoleClient, ClientInfo>,
    session_id: String,
    turn_id: String,
}

impl NodeReplKernel {
    fn from_initialized_client(
        client: RunningService<RoleClient, ClientInfo>,
        session_id: String,
        turn_id: String,
    ) -> Result<NodeReplKernel> {
        validate_ids(&session_id, &turn_id)?;
        Ok(Self {
            client,
            session_id,
            turn_id,
        })
    }

    pub(crate) async fn spawn(
        spec: &NodeReplLaunchSpec,
        session_id: String,
        turn_id: String,
    ) -> Result<NodeReplKernel> {
        validate_ids(&session_id, &turn_id)?;

        let command = command_from_launch_spec(spec);
        let transport = TokioChildProcess::new(command)
            .map_err(|error| anyhow!("browser_runtime_node_repl_spawn_failed:{error}"))?;
        let client = match tokio::time::timeout(
            Duration::from_secs(10),
            node_repl_client_info().serve(transport),
        )
        .await
        {
            Ok(Ok(client)) => client,
            Ok(Err(error)) => {
                return Err(anyhow!(
                    "browser_runtime_node_repl_initialize_failed:{error}"
                ));
            }
            Err(_) => return Err(anyhow!("browser_runtime_node_repl_initialize_timeout")),
        };

        Self::from_initialized_client(client, session_id, turn_id)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    pub(crate) async fn shutdown(self) -> Result<()> {
        match tokio::time::timeout(Duration::from_secs(6), self.client.cancel()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(anyhow!("browser_runtime_node_repl_shutdown_failed:{error}")),
            Err(_) => Err(anyhow!("browser_runtime_node_repl_shutdown_timeout")),
        }
    }

    pub(crate) async fn js(&mut self, code: &str, timeout_ms: u64) -> Result<CallToolResult> {
        let arguments: JsonObject = Map::from_iter([
            ("code".to_string(), Value::String(code.to_string())),
            ("timeout_ms".to_string(), json!(timeout_ms)),
        ]);
        let metadata = Meta(Map::from_iter([(
            "x-codex-turn-metadata".to_string(),
            json!({
                "session_id": self.session_id,
                "turn_id": self.turn_id,
            }),
        )]));
        let mut request = CallToolRequestParams::new("js").with_arguments(arguments);
        request.set_meta(metadata);

        self.client
            .call_tool(request)
            .await
            .map_err(|error| anyhow!("browser_runtime_node_repl_call_failed:{error}"))
    }

    pub(crate) async fn bootstrap_browser(&mut self, browser_client_path: &Path) -> Result<()> {
        let browser_client_path = browser_client_path
            .to_str()
            .ok_or_else(|| anyhow!("browser_runtime_path_not_utf8:browser_client_path"))?;
        let browser_client_path = serde_json::to_string(browser_client_path)
            .expect("serializing a Rust string as JSON cannot fail");
        let code = format!(
            "if (globalThis.agent == null) {{\n  const {{ setupBrowserRuntime }} = await import({browser_client_path});\n  globalThis.agent = await setupBrowserRuntime();\n}}\nif (globalThis.browser == null) {{\n  globalThis.browser = await globalThis.agent.browsers.get(\"chrome\");\n}}\nnodeRepl.write(JSON.stringify({{ browserId: globalThis.browser.browserId }}));",
            browser_client_path = browser_client_path,
        );
        let result = self.js(&code, 20_000).await?;
        if result.is_error == Some(true) {
            return Err(anyhow!("browser_runtime_browser_bootstrap_failed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{
        model::{
            CallToolRequestParams, ClientCapabilities, Content, InitializeRequestParams,
            ProtocolVersion, ServerCapabilities, ServerInfo,
        },
        service::{RequestContext, RunningService},
        RoleServer, ServerHandler, ServiceExt,
    };
    use std::{
        ffi::OsStr,
        future::Future,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    #[derive(Clone, Debug)]
    enum Behavior {
        Preserve,
        Success,
        ToolError,
        ServiceFailure,
    }

    #[derive(Clone, Debug)]
    struct RecordedCall {
        name: String,
        arguments: JsonObject,
        meta: Option<Meta>,
    }

    #[derive(Clone, Debug)]
    struct FakeNodeReplServer {
        behavior: Behavior,
        calls: Arc<Mutex<Vec<RecordedCall>>>,
        initialize_count: Arc<AtomicUsize>,
        initialize_protocol: Arc<Mutex<Option<ProtocolVersion>>>,
    }

    impl FakeNodeReplServer {
        fn new(behavior: Behavior) -> Self {
            Self {
                behavior,
                calls: Arc::new(Mutex::new(Vec::new())),
                initialize_count: Arc::new(AtomicUsize::new(0)),
                initialize_protocol: Arc::new(Mutex::new(None)),
            }
        }
    }

    impl ServerHandler for FakeNodeReplServer {
        fn initialize(
            &self,
            request: InitializeRequestParams,
            context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<rmcp::model::InitializeResult, rmcp::ErrorData>> + Send + '_
        {
            self.initialize_count.fetch_add(1, Ordering::SeqCst);
            *self.initialize_protocol.lock().unwrap() = Some(request.protocol_version.clone());
            if context.peer.peer_info().is_none() {
                context.peer.set_peer_info(request);
            }
            std::future::ready(Ok(self.get_info()))
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_ {
            self.calls.lock().unwrap().push(RecordedCall {
                name: request.name.to_string(),
                arguments: request.arguments.unwrap_or_default(),
                meta: Some(context.meta),
            });
            match self.behavior {
                Behavior::Preserve => std::future::ready(Ok(expected_result())),
                Behavior::Success => std::future::ready(Ok(CallToolResult::default())),
                Behavior::ToolError => std::future::ready(Ok(CallToolResult::structured_error(
                    json!({"message": "tool failed"}),
                ))),
                Behavior::ServiceFailure => std::future::ready(Err(
                    rmcp::ErrorData::internal_error("fake service failure", None),
                )),
            }
        }

        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
                .with_server_info(rmcp::model::Implementation::new("fake-node-repl", "test"))
        }
    }

    async fn connected(
        server: FakeNodeReplServer,
    ) -> (
        RunningService<RoleClient, ClientInfo>,
        tokio::task::JoinHandle<()>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            let running = server.serve(server_io).await.unwrap();
            running.waiting().await.unwrap();
        });
        let client = node_repl_client_info().serve(client_io).await.unwrap();
        (client, server_task)
    }

    fn expected_result() -> CallToolResult {
        let mut result = CallToolResult::default();
        result.content = vec![
            Content::text("hello"),
            Content::image("aW1hZ2U=", "image/png"),
        ];
        result.structured_content = Some(json!({"value": 42}));
        result.is_error = Some(true);
        result.meta = Some(Meta(Map::from_iter([(
            "result-key".to_string(),
            json!("result-value"),
        )])));
        result
    }

    fn metadata_values(meta: &Meta) -> &Value {
        &meta.0["x-codex-turn-metadata"]
    }

    #[test]
    fn node_repl_client_info_uses_the_frozen_initialize_contract() {
        let info = node_repl_client_info();

        assert_eq!(info.protocol_version, ProtocolVersion::V_2025_06_18);
        assert_eq!(info.capabilities, ClientCapabilities::default());
        assert_eq!(info.client_info.name, "agentic-browser-runtime");
        assert_eq!(info.client_info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn command_helper_uses_direct_program_cwd_and_env_overrides() {
        let spec = NodeReplLaunchSpec {
            program: PathBuf::from("/runtime/node-repl.mjs"),
            cwd: PathBuf::from("/runtime"),
            env_overrides: std::collections::BTreeMap::from([(
                "CUSTOM_SETTING".to_string(),
                "preserved".to_string(),
            )]),
        };
        let command = command_from_launch_spec(&spec);
        let command = command.as_std();

        assert_eq!(command.get_program(), OsStr::new("/runtime/node-repl.mjs"));
        assert_eq!(
            command.get_current_dir(),
            Some(PathBuf::from("/runtime").as_path())
        );
        assert!(command.get_args().next().is_none());
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == OsStr::new("CUSTOM_SETTING"))
                .and_then(|(_, value)| value),
            Some(OsStr::new("preserved"))
        );
    }

    #[tokio::test]
    async fn spawn_rejects_invalid_ids_before_attempting_process_creation() {
        let spec = NodeReplLaunchSpec {
            program: PathBuf::from("/definitely/missing/node-repl"),
            cwd: PathBuf::from("."),
            env_overrides: std::collections::BTreeMap::new(),
        };

        let error = match NodeReplKernel::spawn(&spec, " ".to_string(), "turn-1".to_string()).await
        {
            Ok(_) => panic!("invalid session id was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "browser_runtime_session_id_invalid");
    }

    #[tokio::test]
    async fn impossible_executable_is_a_spawn_failure() {
        let spec = NodeReplLaunchSpec {
            program: PathBuf::from("/definitely/missing/node-repl"),
            cwd: PathBuf::from("."),
            env_overrides: std::collections::BTreeMap::new(),
        };

        let error =
            match NodeReplKernel::spawn(&spec, "session-1".to_string(), "turn-1".to_string()).await
            {
                Ok(_) => panic!("missing executable unexpectedly spawned"),
                Err(error) => error,
            };
        assert!(error
            .to_string()
            .starts_with("browser_runtime_node_repl_spawn_failed:"));
    }

    #[tokio::test]
    async fn constructor_rejects_empty_or_whitespace_session_ids() {
        for session_id in ["", " ", "\n\t"] {
            let server = FakeNodeReplServer::new(Behavior::Preserve);
            let (client, server_task) = connected(server).await;
            let error = match NodeReplKernel::from_initialized_client(
                client,
                session_id.to_string(),
                "turn-1".to_string(),
            ) {
                Ok(_) => panic!("empty session id was accepted"),
                Err(error) => error,
            };
            assert_eq!(error.to_string(), "browser_runtime_session_id_invalid");
            server_task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn constructor_rejects_empty_or_whitespace_turn_ids() {
        for turn_id in ["", " ", "\n\t"] {
            let server = FakeNodeReplServer::new(Behavior::Preserve);
            let (client, server_task) = connected(server).await;
            let error = match NodeReplKernel::from_initialized_client(
                client,
                "session-1".to_string(),
                turn_id.to_string(),
            ) {
                Ok(_) => panic!("empty turn id was accepted"),
                Err(error) => error,
            };
            assert_eq!(error.to_string(), "browser_runtime_turn_id_invalid");
            server_task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn js_sends_exact_tool_name_and_arguments() {
        let server = FakeNodeReplServer::new(Behavior::Preserve);
        let calls = server.calls.clone();
        let (client, server_task) = connected(server).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        kernel.js("1 + 2", 750).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "js");
        assert_eq!(
            calls[0].arguments,
            Map::from_iter([
                ("code".to_string(), json!("1 + 2")),
                ("timeout_ms".to_string(), json!(750_u64)),
            ])
        );
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn js_attaches_exact_codex_turn_metadata() {
        let server = FakeNodeReplServer::new(Behavior::Preserve);
        let calls = server.calls.clone();
        let (client, server_task) = connected(server).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-abc".to_string(),
            "turn-xyz".to_string(),
        )
        .unwrap();

        kernel.js("console.log(1)", 100).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(
            metadata_values(calls[0].meta.as_ref().unwrap()),
            &json!({"session_id": "session-abc", "turn_id": "turn-xyz"})
        );
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn sequential_calls_reuse_initialized_service_and_metadata() {
        let server = FakeNodeReplServer::new(Behavior::Preserve);
        let calls = server.calls.clone();
        let initialize_count = server.initialize_count.clone();
        let initialize_protocol = server.initialize_protocol.clone();
        let (client, server_task) = connected(server).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-abc".to_string(),
            "turn-xyz".to_string(),
        )
        .unwrap();

        kernel.js("first", 10).await.unwrap();
        kernel.js("second", 20).await.unwrap();
        assert_eq!(initialize_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            initialize_protocol.lock().unwrap().as_ref(),
            Some(&ProtocolVersion::V_2025_06_18)
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert_eq!(
                metadata_values(call.meta.as_ref().unwrap()),
                &json!({"session_id": "session-abc", "turn_id": "turn-xyz"})
            );
        }
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cleanly_terminates_an_in_memory_service() {
        let (client, server_task) = connected(FakeNodeReplServer::new(Behavior::Preserve)).await;
        let kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        kernel.shutdown().await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn js_preserves_mixed_result_fields_unchanged() {
        let (client, server_task) = connected(FakeNodeReplServer::new(Behavior::Preserve)).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        let result = kernel.js("code", 1).await.unwrap();
        assert_eq!(result, expected_result());
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn js_keeps_tool_error_result_as_successful_transport_return() {
        let (client, server_task) = connected(FakeNodeReplServer::new(Behavior::ToolError)).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        let result = kernel.js("bad code", 1).await.unwrap();
        assert_eq!(result.is_error, Some(true));
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn js_prefixes_rmcp_service_failures() {
        let (client, server_task) =
            connected(FakeNodeReplServer::new(Behavior::ServiceFailure)).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        let error = kernel.js("code", 1).await.unwrap_err();
        assert!(error
            .to_string()
            .starts_with("browser_runtime_node_repl_call_failed:"));
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_sends_escaped_path_and_frozen_browser_setup_code() {
        let server = FakeNodeReplServer::new(Behavior::Success);
        let calls = server.calls.clone();
        let (client, server_task) = connected(server).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();
        let browser_client_path = PathBuf::from(r#"/runtime/browser"client\browser-client.mjs"#);
        let raw_path = browser_client_path.to_str().unwrap().to_string();

        kernel
            .bootstrap_browser(&browser_client_path)
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "js");
        assert_eq!(calls[0].arguments["timeout_ms"], json!(20_000_u64));
        let code = calls[0].arguments["code"].as_str().unwrap();
        let encoded_path = serde_json::to_string(&raw_path).unwrap();
        assert!(code.contains(&format!("await import({encoded_path})")));
        assert!(!code.contains(&raw_path));
        assert!(code.contains("if (globalThis.agent == null)"));
        assert!(code.contains("const { setupBrowserRuntime } = await import("));
        assert!(code.contains("globalThis.agent = await setupBrowserRuntime();"));
        assert!(code.contains("if (globalThis.browser == null)"));
        assert!(
            code.contains("globalThis.browser = await globalThis.agent.browsers.get(\"chrome\");")
        );
        assert!(code.contains(
            "nodeRepl.write(JSON.stringify({ browserId: globalThis.browser.browserId }));"
        ));
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_converts_tool_error_to_stable_failure() {
        let (client, server_task) = connected(FakeNodeReplServer::new(Behavior::ToolError)).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        let error = kernel
            .bootstrap_browser(Path::new("/runtime/browser-client.mjs"))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "browser_runtime_browser_bootstrap_failed"
        );
        drop(kernel);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn bootstrap_preserves_rmcp_service_failure_prefix() {
        let (client, server_task) =
            connected(FakeNodeReplServer::new(Behavior::ServiceFailure)).await;
        let mut kernel = NodeReplKernel::from_initialized_client(
            client,
            "session-1".to_string(),
            "turn-1".to_string(),
        )
        .unwrap();

        let error = kernel
            .bootstrap_browser(Path::new("/runtime/browser-client.mjs"))
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("browser_runtime_node_repl_call_failed:"));
        drop(kernel);
        server_task.await.unwrap();
    }
}
