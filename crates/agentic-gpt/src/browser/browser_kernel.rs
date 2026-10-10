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

const BROWSER_UNAVAILABLE_SENTINEL: &str = "agentic_browser_runtime_browser_unavailable";

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

    pub(crate) async fn turn_ended(&mut self) -> Result<()> {
        let arguments: JsonObject = Map::from_iter([
            ("hook_event_name".to_string(), json!("Stop")),
            ("session_id".to_string(), json!(self.session_id)),
            ("turn_id".to_string(), json!(self.turn_id)),
        ]);
        let result = self
            .client
            .call_tool(CallToolRequestParams::new("turn_ended").with_arguments(arguments))
            .await
            .map_err(|error| anyhow!("browser_runtime_node_repl_call_failed:{error}"))?;
        if result.is_error == Some(true) {
            return Err(anyhow!("browser_runtime_turn_ended_failed"));
        }
        Ok(())
    }

    pub(crate) async fn reset_js(&mut self) -> Result<()> {
        let result = self
            .client
            .call_tool(CallToolRequestParams::new("js_reset").with_arguments(Map::new()))
            .await
            .map_err(|error| anyhow!("browser_runtime_node_repl_call_failed:{error}"))?;
        if result.is_error == Some(true) {
            return Err(anyhow!("browser_runtime_js_reset_failed"));
        }
        Ok(())
    }

    pub(crate) async fn bootstrap_browser(&mut self, browser_client_path: &Path) -> Result<()> {
        let browser_client_path = browser_client_path
            .to_str()
            .ok_or_else(|| anyhow!("browser_runtime_path_not_utf8:browser_client_path"))?;
        let browser_client_path = serde_json::to_string(browser_client_path)
            .expect("serializing a Rust string as JSON cannot fail");
        let code = format!(
            "if (globalThis.agent == null) {{\n  const {{ setupBrowserRuntime }} = await import({browser_client_path});\n  globalThis.agent = await setupBrowserRuntime();\n}}\nif (globalThis.browser == null) {{\n  const __agentic_browsers = await globalThis.agent.browsers.list();\n  if (!__agentic_browsers.some((browser) => browser.family === \"chrome\")) {{\n    nodeRepl.write({sentinel});\n  }} else {{\n    globalThis.browser = await globalThis.agent.browsers.get(\"chrome\");\n  }}\n}}\nif (globalThis.browser != null) {{\n  nodeRepl.write(JSON.stringify({{ browserId: globalThis.browser.browserId }}));\n}}",
            browser_client_path = browser_client_path,
            sentinel = serde_json::to_string(BROWSER_UNAVAILABLE_SENTINEL)
                .expect("serializing a Rust string as JSON cannot fail"),
        );
        let result = self.js(&code, 20_000).await?;
        if result.is_error == Some(true) {
            return Err(anyhow!("browser_runtime_browser_bootstrap_failed"));
        }
        if result.content.iter().any(|content| {
            content
                .as_text()
                .is_some_and(|text| text.text == BROWSER_UNAVAILABLE_SENTINEL)
        }) {
            return Err(anyhow!("browser_runtime_browser_unavailable"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use rmcp::{
        model::{
            CallToolRequestParams, Content, InitializeRequestParams, ProtocolVersion,
            ServerCapabilities, ServerInfo,
        },
        service::{RequestContext, RunningService},
        RoleServer, ServerHandler, ServiceExt,
    };
    use std::{
        future::Future,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[derive(Clone, Debug)]
    enum Behavior {
        Preserve,
        Success,
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
            *self.initialize_protocol.lock() = Some(request.protocol_version.clone());
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
            self.calls.lock().push(RecordedCall {
                name: request.name.to_string(),
                arguments: request.arguments.unwrap_or_default(),
                meta: Some(context.meta),
            });
            match self.behavior {
                Behavior::Preserve => std::future::ready(Ok(expected_result())),
                Behavior::Success => std::future::ready(Ok(CallToolResult::default())),
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
            initialize_protocol.lock().as_ref(),
            Some(&ProtocolVersion::V_2025_06_18)
        );
        {
            let calls = calls.lock();
            assert_eq!(calls.len(), 2);
            for call in calls.iter() {
                assert_eq!(
                    metadata_values(call.meta.as_ref().unwrap()),
                    &json!({"session_id": "session-abc", "turn_id": "turn-xyz"})
                );
            }
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
    async fn bootstrap_escapes_import_path_and_initializes_browser_in_order() {
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

        {
            let calls = calls.lock();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "js");
            assert_eq!(calls[0].arguments["timeout_ms"], json!(20_000_u64));
            let code = calls[0].arguments["code"].as_str().unwrap();
            let encoded_path = serde_json::to_string(&raw_path).unwrap();
            let import_path = format!("import({encoded_path})");
            assert!(code.contains(&import_path));
            assert!(!code.contains(&raw_path));

            let agent_guard = code.find("globalThis.agent == null").unwrap();
            let setup_import = code.find(&import_path).unwrap();
            let setup_agent = code
                .find("globalThis.agent = await setupBrowserRuntime()")
                .unwrap();
            let browser_guard = code.find("globalThis.browser == null").unwrap();
            let list_browsers = code.find("browsers.list()").unwrap();
            let select_chrome = code.find("browser.family === \"chrome\"").unwrap();
            let get_browser = code.find("browsers.get(\"chrome\")").unwrap();
            let result_guard = code.find("if (globalThis.browser != null)").unwrap();
            let result_write = code.find("nodeRepl.write(JSON.stringify").unwrap();
            let browser_id = code
                .find("browserId: globalThis.browser.browserId")
                .unwrap();

            assert!(
                agent_guard < setup_import
                    && setup_import < setup_agent
                    && setup_agent < browser_guard
                    && browser_guard < list_browsers
                    && list_browsers < select_chrome
                    && select_chrome < get_browser
                    && get_browser < result_guard
                    && result_guard < result_write
                    && result_write < browser_id
            );
            assert!(code.contains(BROWSER_UNAVAILABLE_SENTINEL));
            assert!(code.contains("JSON.stringify"));
        }
        drop(kernel);
        server_task.await.unwrap();
    }
}
