use anyhow::{anyhow, Result};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ClientInfo, JsonObject, Meta, RequestParamsMeta,
    },
    service::RunningService,
    RoleClient,
};
use serde_json::{json, Map, Value};

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
        if session_id.trim().is_empty() {
            return Err(anyhow!("browser_runtime_session_id_invalid"));
        }
        if turn_id.trim().is_empty() {
            return Err(anyhow!("browser_runtime_turn_id_invalid"));
        }
        Ok(Self {
            client,
            session_id,
            turn_id,
        })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{
        model::{
            CallToolRequestParams, Content, InitializeRequestParams, ServerCapabilities, ServerInfo,
        },
        service::{RequestContext, RunningService},
        RoleServer, ServerHandler, ServiceExt,
    };
    use std::{
        future::Future,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    #[derive(Clone, Debug)]
    enum Behavior {
        Preserve,
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
    }

    impl FakeNodeReplServer {
        fn new(behavior: Behavior) -> Self {
            Self {
                behavior,
                calls: Arc::new(Mutex::new(Vec::new())),
                initialize_count: Arc::new(AtomicUsize::new(0)),
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
        let client = ClientInfo::default().serve(client_io).await.unwrap();
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
}
