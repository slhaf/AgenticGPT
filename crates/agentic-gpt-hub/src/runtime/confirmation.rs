use agentic_gpt_protocol::{ConfirmationDecision, ConfirmationPayload, HubMessage};
use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::routes::api_error;
use crate::state::{HubState, OutboundAgentMessage};
use crate::utils::{constant_time_equal, random_id, random_token, sha256_hex};

const MAX_COMMAND_PREVIEW_CHARS: usize = 1000;

struct PendingConfirmation {
    request_id: String,
    agent_id: String,
    connection_id: String,
    sender: Option<mpsc::UnboundedSender<OutboundAgentMessage>>,
    token_hash: String,
    expires_at: DateTime<Utc>,
    resolved: bool,
    decision: Option<ConfirmationDecision>,
}

pub(crate) struct ConfirmationPublication {
    pub(crate) confirmation_id: String,
    pub(crate) token: String,
    pub(crate) agent_id: String,
    pub(crate) request_id: String,
    pub(crate) payload: ConfirmationPayload,
    pub(crate) command_preview: String,
}

pub(crate) struct ConfirmationAdmission<'a> {
    pub(crate) agent_id: &'a str,
    pub(crate) connection_id: &'a str,
    pub(crate) request_id: String,
    pub(crate) sender: mpsc::UnboundedSender<OutboundAgentMessage>,
    pub(crate) timeout_seconds: u64,
    pub(crate) provider_timeout_seconds: u64,
    pub(crate) payload: ConfirmationPayload,
}

#[cfg(test)]
pub(crate) struct TestConfirmation {
    pub(crate) confirmation_id: String,
    pub(crate) request_id: String,
    pub(crate) agent_id: String,
    pub(crate) connection_id: String,
    pub(crate) token: String,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) sender: mpsc::UnboundedSender<OutboundAgentMessage>,
}

struct ClaimedConfirmation {
    request_id: String,
    sender: Option<mpsc::UnboundedSender<OutboundAgentMessage>>,
}
pub(crate) struct Confirmations {
    pending: Mutex<HashMap<String, PendingConfirmation>>,
    #[cfg(test)]
    retire_gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Confirmations {
    pub(crate) async fn admit(
        &self,
        admission: ConfirmationAdmission<'_>,
    ) -> ConfirmationPublication {
        let ConfirmationAdmission {
            agent_id,
            connection_id,
            request_id,
            sender,
            timeout_seconds,
            provider_timeout_seconds,
            payload,
        } = admission;
        let confirmation_id = random_id("confirm");
        let token = random_token();
        let created_at = Utc::now();
        let timeout_seconds = timeout_seconds.max(1).min(provider_timeout_seconds.max(1));
        let expires_at = created_at + chrono::Duration::seconds(timeout_seconds as i64);
        let command_preview = truncate_chars(&payload.command_preview, MAX_COMMAND_PREVIEW_CHARS);
        let pending = PendingConfirmation {
            request_id: request_id.clone(),
            agent_id: agent_id.to_string(),
            connection_id: connection_id.to_string(),
            sender: Some(sender),
            token_hash: sha256_hex(&token),
            expires_at,
            resolved: false,
            decision: None,
        };
        self.pending
            .lock()
            .await
            .insert(confirmation_id.clone(), pending);
        ConfirmationPublication {
            confirmation_id,
            token,
            agent_id: agent_id.to_string(),
            request_id,
            payload,
            command_preview,
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            #[cfg(test)]
            retire_gate: Mutex::new(None),
        }
    }

    pub(crate) async fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .await
            .values()
            .filter(|pending| !pending.resolved)
            .count()
    }

    async fn is_pending(&self, confirmation_id: &str) -> bool {
        self.pending
            .lock()
            .await
            .get(confirmation_id)
            .is_some_and(|pending| !pending.resolved)
    }
    #[cfg(test)]
    pub(crate) async fn pause_next_retirement(&self) -> oneshot::Sender<()> {
        let (release, wait) = oneshot::channel();
        *self.retire_gate.lock().await = Some(wait);
        release
    }

    #[cfg(test)]
    async fn wait_retire_gate(&self) {
        let wait = self.retire_gate.lock().await.take();
        if let Some(wait) = wait {
            let _ = wait.await;
        }
    }

    async fn claim(
        &self,
        confirmation_id: &str,
        decision: ConfirmationDecision,
    ) -> Option<ClaimedConfirmation> {
        let mut pending = self.pending.lock().await;
        let pending = pending.get_mut(confirmation_id)?;
        if pending.resolved {
            return None;
        }
        pending.resolved = true;
        pending.decision = Some(decision);
        Some(ClaimedConfirmation {
            request_id: pending.request_id.clone(),
            sender: pending.sender.take(),
        })
    }

    async fn claim_generation(
        &self,
        agent_id: &str,
        connection_id: &str,
    ) -> Vec<ClaimedConfirmation> {
        let mut pending = self.pending.lock().await;
        pending
            .values_mut()
            .filter(|pending| {
                pending.agent_id == agent_id
                    && pending.connection_id == connection_id
                    && !pending.resolved
            })
            .map(|pending| {
                pending.resolved = true;
                pending.decision = Some(ConfirmationDecision::ProviderUnavailable);
                ClaimedConfirmation {
                    request_id: pending.request_id.clone(),
                    sender: pending.sender.take(),
                }
            })
            .collect()
    }

    async fn claim_expired(&self, now: DateTime<Utc>) -> Vec<ClaimedConfirmation> {
        let mut pending = self.pending.lock().await;
        pending
            .values_mut()
            .filter(|pending| !pending.resolved && now >= pending.expires_at)
            .map(|pending| {
                pending.resolved = true;
                pending.decision = Some(ConfirmationDecision::Timeout);
                ClaimedConfirmation {
                    request_id: pending.request_id.clone(),
                    sender: pending.sender.take(),
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) async fn insert_for_test(&self, confirmation: TestConfirmation) {
        self.pending.lock().await.insert(
            confirmation.confirmation_id,
            PendingConfirmation {
                request_id: confirmation.request_id,
                agent_id: confirmation.agent_id,
                connection_id: confirmation.connection_id,
                sender: Some(confirmation.sender),
                token_hash: sha256_hex(&confirmation.token),
                expires_at: confirmation.expires_at,
                resolved: false,
                decision: None,
            },
        );
    }

    #[cfg(test)]
    pub(crate) async fn snapshot_for_test(
        &self,
        confirmation_id: &str,
    ) -> Option<ConfirmationSnapshot> {
        self.pending
            .lock()
            .await
            .get(confirmation_id)
            .map(|pending| ConfirmationSnapshot {
                resolved: pending.resolved,
                decision: pending.decision.clone(),
            })
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct ConfirmationSnapshot {
    pub(crate) resolved: bool,
    pub(crate) decision: Option<ConfirmationDecision>,
}

pub(crate) async fn handle_confirmation_request(
    state: HubState,
    publication: ConfirmationPublication,
) -> Result<()> {
    let remote = &state.config.remote_confirmation;
    if !remote.enabled || remote.provider != "ntfy" {
        resolve_and_send(
            &state,
            &publication.confirmation_id,
            ConfirmationDecision::ProviderUnavailable,
            "remote_confirmation_disabled",
        )
        .await;
        return Ok(());
    }
    if remote.ntfy.topic.trim().is_empty()
        || remote.ntfy.server_url.trim().is_empty()
        || remote.ntfy.callback_base_url.trim().is_empty()
    {
        resolve_and_send(
            &state,
            &publication.confirmation_id,
            ConfirmationDecision::ProviderUnavailable,
            "ntfy_not_configured",
        )
        .await;
        return Ok(());
    }
    if !state
        .confirmations
        .is_pending(&publication.confirmation_id)
        .await
    {
        return Ok(());
    }

    match publish_ntfy(
        &state,
        &publication.confirmation_id,
        &publication.token,
        &publication.agent_id,
        &publication.payload,
        &publication.command_preview,
    )
    .await
    {
        Ok(()) => {
            info!(
                agent_id = %publication.agent_id,
                confirmation_id = %publication.confirmation_id,
                request_id = %publication.request_id,
                "remote confirmation notification sent"
            );
        }
        Err(error) => {
            warn!(
                agent_id = %publication.agent_id,
                confirmation_id = %publication.confirmation_id,
                request_id = %publication.request_id,
                %error,
                "remote confirmation notification failed"
            );
            resolve_and_send(
                &state,
                &publication.confirmation_id,
                ConfirmationDecision::ProviderUnavailable,
                "ntfy_publish_failed",
            )
            .await;
        }
    }
    Ok(())
}

pub(crate) async fn retire_generation(state: &HubState, agent_id: &str, connection_id: &str) {
    #[cfg(test)]
    state.confirmations.wait_retire_gate().await;
    let claimed = state
        .confirmations
        .claim_generation(agent_id, connection_id)
        .await;
    for claimed in claimed {
        send_claimed(
            claimed,
            ConfirmationDecision::ProviderUnavailable,
            "provider_unavailable",
        )
        .await;
    }
}

async fn resolve_and_send(
    state: &HubState,
    confirmation_id: &str,
    decision: ConfirmationDecision,
    reason: &str,
) {
    let Some(claimed) = state
        .confirmations
        .claim(confirmation_id, decision.clone())
        .await
    else {
        return;
    };
    send_claimed(claimed, decision, reason).await;
}

async fn send_claimed(claimed: ClaimedConfirmation, decision: ConfirmationDecision, reason: &str) {
    let Some(sender) = claimed.sender else {
        return;
    };
    let message = HubMessage::ConfirmationResponse {
        request_id: claimed.request_id,
        decision,
        reason: reason.to_string(),
    };
    let Ok(text) = serde_json::to_string(&message) else {
        return;
    };
    let _ = sender.send(OutboundAgentMessage::Text(text));
}

#[derive(Deserialize)]
pub(crate) struct ConfirmationCallbackQuery {
    pub(crate) token: String,
}

pub(crate) async fn callback(
    State(state): State<HubState>,
    Path((confirmation_id, decision)): Path<(String, String)>,
    Query(query): Query<ConfirmationCallbackQuery>,
) -> Response {
    let decision = match decision.as_str() {
        "allow" => ConfirmationDecision::AllowOnce,
        "allow-mcp-server-15m" | "allow_mcp_server_15m" => ConfirmationDecision::AllowMcpServer15m,
        "allow-mcp-server-30m" | "allow_mcp_server_30m" => ConfirmationDecision::AllowMcpServer30m,
        "deny" => ConfirmationDecision::Deny,
        _ => {
            return api_error(
                StatusCode::NOT_FOUND,
                "confirmation_not_found",
                "Unknown confirmation callback action",
            )
        }
    };
    let token_hash = sha256_hex(&query.token);
    let token_status = {
        let mut pending = state.confirmations.pending.lock().await;
        let Some(pending) = pending.get_mut(&confirmation_id) else {
            return api_error(
                StatusCode::NOT_FOUND,
                "confirmation_not_found",
                "Confirmation was not found",
            );
        };
        if pending.resolved && Utc::now() >= pending.expires_at {
            return api_error(
                StatusCode::GONE,
                "confirmation_expired",
                "Confirmation has expired",
            );
        }
        if pending.resolved {
            return api_error(
                StatusCode::CONFLICT,
                "confirmation_resolved",
                "Confirmation has already been resolved",
            );
        }
        if Utc::now() >= pending.expires_at {
            pending.resolved = true;
            pending.decision = Some(ConfirmationDecision::Expired);
            let claimed = ClaimedConfirmation {
                request_id: pending.request_id.clone(),
                sender: pending.sender.take(),
            };
            Some((claimed, ConfirmationDecision::Expired, "expired"))
        } else if !constant_time_equal(&token_hash, &pending.token_hash) {
            return api_error(
                StatusCode::FORBIDDEN,
                "callback_token_invalid",
                "Invalid confirmation token",
            );
        } else {
            pending.resolved = true;
            pending.decision = Some(decision.clone());
            let reason = match &decision {
                ConfirmationDecision::AllowOnce => "user_allowed",
                ConfirmationDecision::AllowMcpServer15m => "user_allowed_mcp_server_15m",
                ConfirmationDecision::AllowMcpServer30m => "user_allowed_mcp_server_30m",
                ConfirmationDecision::Deny => "user_denied",
                _ => "resolved",
            };
            let claimed = ClaimedConfirmation {
                request_id: pending.request_id.clone(),
                sender: pending.sender.take(),
            };
            Some((claimed, decision.clone(), reason))
        }
    };

    let Some((claimed, resolved_decision, reason)) = token_status else {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "confirmation_resolution_failed",
            "Confirmation resolution failed",
        );
    };
    if matches!(&resolved_decision, ConfirmationDecision::Expired) {
        send_claimed(claimed, resolved_decision, reason).await;
        return api_error(
            StatusCode::GONE,
            "confirmation_expired",
            "Confirmation has expired",
        );
    }
    send_claimed(claimed, resolved_decision.clone(), reason).await;
    Json(json!({
        "status": "accepted",
        "decision": decision_wire_value(&resolved_decision)
    }))
    .into_response()
}

pub(crate) async fn cleanup(state: HubState) {
    loop {
        sleep(Duration::from_secs(2)).await;
        let expired = state.confirmations.claim_expired(Utc::now()).await;
        for claimed in expired {
            info!(request_id = %claimed.request_id, "confirmation timed out");
            send_claimed(claimed, ConfirmationDecision::Timeout, "timeout").await;
        }
    }
}

async fn publish_ntfy(
    state: &HubState,
    confirmation_id: &str,
    token: &str,
    agent_id: &str,
    payload: &ConfirmationPayload,
    command_preview: &str,
) -> Result<()> {
    let remote = &state.config.remote_confirmation;
    let ntfy = &remote.ntfy;
    let server_url = ntfy.server_url.trim_end_matches('/');
    let callback_base = ntfy.callback_base_url.trim_end_matches('/');
    let message = format!(
        "Agent {agent_id} wants to run:\n{command_preview}\n\nReason: {}\nRisk: {}",
        payload.reason, payload.risk_level
    );
    let actions = ntfy_confirmation_actions(
        callback_base,
        confirmation_id,
        token,
        payload.kind.as_deref(),
    );
    let body = json!({
        "topic": ntfy.topic,
        "title": "AgenticGPT confirmation",
        "message": message,
        "priority": 5,
        "tags": ["warning"],
        "actions": actions
    });
    let response = state.http.post(server_url).json(&body).send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("ntfy returned {}", response.status()))
    }
}

pub(crate) fn ntfy_confirmation_actions(
    callback_base: &str,
    confirmation_id: &str,
    token: &str,
    kind: Option<&str>,
) -> serde_json::Value {
    let allow_url =
        format!("{callback_base}/v1/confirmations/{confirmation_id}/allow?token={token}");
    let allow_mcp_30m_url = format!(
        "{callback_base}/v1/confirmations/{confirmation_id}/allow-mcp-server-30m?token={token}"
    );
    let deny_url = format!("{callback_base}/v1/confirmations/{confirmation_id}/deny?token={token}");
    if matches!(kind, Some("mcpTool" | "mcpBatchSingleServer")) {
        json!([
            {
                "action": "http",
                "label": "Allow once",
                "url": allow_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Allow MCP 30m",
                "url": allow_mcp_30m_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Deny",
                "url": deny_url,
                "method": "POST",
                "clear": true
            }
        ])
    } else {
        json!([
            {
                "action": "http",
                "label": "Allow",
                "url": allow_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Deny",
                "url": deny_url,
                "method": "POST",
                "clear": true
            }
        ])
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn decision_wire_value(decision: &ConfirmationDecision) -> &'static str {
    match decision {
        ConfirmationDecision::AllowOnce => "allow_once",
        ConfirmationDecision::AllowMcpServer15m => "allow_mcp_server_15m",
        ConfirmationDecision::AllowMcpServer30m => "allow_mcp_server_30m",
        ConfirmationDecision::Deny => "deny",
        ConfirmationDecision::Timeout => "timeout",
        ConfirmationDecision::ProviderUnavailable => "provider_unavailable",
        ConfirmationDecision::CallbackTokenInvalid => "callback_token_invalid",
        ConfirmationDecision::Expired => "expired",
    }
}
