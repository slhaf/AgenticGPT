use std::future::Future;

use rmcp::{
    model::{
        ClientCapabilities, ClientJsonRpcMessage, ClientRequest, Implementation, InitializeRequest,
        InitializeRequestParams, ProtocolVersion, RequestId, ServerJsonRpcMessage,
    },
    service::RoleServer,
    transport::Transport,
};
use uuid::Uuid;

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
pub(super) struct ResumableStdioTransport<T> {
    inner: T,
    state: StdioInitializationState,
    pending: Option<ClientJsonRpcMessage>,
}

impl<T> ResumableStdioTransport<T> {
    pub(super) fn new(inner: T) -> Self {
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
