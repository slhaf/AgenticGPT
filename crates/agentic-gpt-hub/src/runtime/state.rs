use agentic_gpt_protocol::{
    AgentConnectionMode, AgentRole, JobInfo, JobState, NotificationChannel, SafeConfigSummary,
};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

use crate::{oauth, room, HubConfig};

pub(crate) const JOB_CACHE_CAPACITY: usize = 4096;
pub(crate) const JOB_CACHE_TTL_SECS: i64 = 15 * 60;
pub(crate) const JOB_CACHE_STALE_SECS: i64 = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JobFreshness {
    Live,
    Cached,
    Stale,
    Unknown,
}

impl JobFreshness {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Cached => "cached",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone)]
pub(crate) struct JobCacheSnapshot {
    pub(crate) job: JobInfo,
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) freshness: JobFreshness,
}

#[derive(Clone)]
struct JobCacheEntry {
    job: JobInfo,
    observed_at: DateTime<Utc>,
    connection_id: String,
    boot_generation: Option<String>,
    stale: bool,
    unknown: bool,
}

#[derive(Default)]
struct JobCacheInner {
    entries: HashMap<(String, String), JobCacheEntry>,
}

pub(crate) struct JobCache {
    inner: Mutex<JobCacheInner>,
}

impl JobCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(JobCacheInner::default()),
        }
    }

    pub(crate) async fn record(
        &self,
        agent_id: &str,
        connection_id: &str,
        boot_generation: Option<&str>,
        job: JobInfo,
    ) {
        self.record_at(agent_id, connection_id, boot_generation, job, Utc::now())
            .await;
    }

    async fn record_at(
        &self,
        agent_id: &str,
        connection_id: &str,
        boot_generation: Option<&str>,
        job: JobInfo,
        observed_at: DateTime<Utc>,
    ) {
        let key = (agent_id.to_string(), job.job_id.clone());
        let mut inner = self.inner.lock().await;
        Self::sweep_locked(&mut inner, observed_at);
        if let Some(existing) = inner.entries.get(&key) {
            let generation_changed = existing.boot_generation.as_deref() != boot_generation;
            if (existing.job.state.is_terminal() && job.state.is_active())
                || (generation_changed && existing.job.state.is_active() && job.state.is_active())
                || job.updated_at < existing.job.updated_at
            {
                return;
            }
        }
        if !inner.entries.contains_key(&key) && inner.entries.len() >= JOB_CACHE_CAPACITY {
            if let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.observed_at)
                .map(|(key, _)| key.clone())
            {
                inner.entries.remove(&oldest);
            }
        }
        inner.entries.insert(
            key,
            JobCacheEntry {
                job,
                observed_at,
                connection_id: connection_id.to_string(),
                boot_generation: boot_generation.map(str::to_string),
                stale: false,
                unknown: false,
            },
        );
    }

    pub(crate) async fn mark_connection_stale(&self, agent_id: &str, connection_id: &str) {
        let mut inner = self.inner.lock().await;
        for ((entry_agent_id, _), entry) in &mut inner.entries {
            if entry_agent_id == agent_id && entry.connection_id == connection_id {
                entry.stale = true;
            }
        }
    }

    pub(crate) async fn mark_unknown_after_restart(&self, agent_id: &str) {
        let mut inner = self.inner.lock().await;
        for ((entry_agent_id, _), entry) in &mut inner.entries {
            if entry_agent_id == agent_id && entry.job.state.is_active() {
                entry.job.state = JobState::UnknownAfterRestart;
                entry.job.reject_reason = Some("unknown_after_restart".to_string());
                entry.unknown = true;
                entry.stale = true;
            }
        }
    }

    pub(crate) async fn snapshot(&self, agent_id: &str, job_id: &str) -> Option<JobCacheSnapshot> {
        let now = Utc::now();
        self.snapshot_at(agent_id, job_id, now).await
    }

    async fn snapshot_at(
        &self,
        agent_id: &str,
        job_id: &str,
        now: DateTime<Utc>,
    ) -> Option<JobCacheSnapshot> {
        let mut inner = self.inner.lock().await;
        Self::sweep_locked(&mut inner, now);
        inner
            .entries
            .get(&(agent_id.to_string(), job_id.to_string()))
            .map(|entry| JobCacheSnapshot {
                job: entry.job.clone(),
                observed_at: entry.observed_at,
                freshness: Self::freshness(entry, now),
            })
    }

    pub(crate) async fn snapshots(&self, agent_id: &str) -> Vec<JobCacheSnapshot> {
        let now = Utc::now();
        self.snapshots_at(agent_id, now).await
    }

    async fn snapshots_at(&self, agent_id: &str, now: DateTime<Utc>) -> Vec<JobCacheSnapshot> {
        let mut inner = self.inner.lock().await;
        Self::sweep_locked(&mut inner, now);
        let mut snapshots = inner
            .entries
            .iter()
            .filter(|((entry_agent_id, _), _)| entry_agent_id == agent_id)
            .map(|(_, entry)| JobCacheSnapshot {
                job: entry.job.clone(),
                observed_at: entry.observed_at,
                freshness: Self::freshness(entry, now),
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            right
                .job
                .updated_at
                .cmp(&left.job.updated_at)
                .then_with(|| right.job.job_id.cmp(&left.job.job_id))
        });
        snapshots
    }

    pub(crate) async fn count(&self) -> usize {
        let mut inner = self.inner.lock().await;
        Self::sweep_locked(&mut inner, Utc::now());
        inner.entries.len()
    }

    pub(crate) async fn sweep(&self, now: DateTime<Utc>) {
        let mut inner = self.inner.lock().await;
        Self::sweep_locked(&mut inner, now);
    }

    fn freshness(entry: &JobCacheEntry, now: DateTime<Utc>) -> JobFreshness {
        if entry.unknown {
            JobFreshness::Unknown
        } else if entry.stale
            || now.signed_duration_since(entry.observed_at).num_seconds() >= JOB_CACHE_STALE_SECS
        {
            JobFreshness::Stale
        } else {
            JobFreshness::Cached
        }
    }

    fn sweep_locked(inner: &mut JobCacheInner, now: DateTime<Utc>) {
        inner.entries.retain(|_, entry| {
            now.signed_duration_since(entry.observed_at).num_seconds() < JOB_CACHE_TTL_SECS
        });
    }

    #[cfg(test)]
    pub(crate) async fn insert_for_test(
        &self,
        agent_id: &str,
        connection_id: &str,
        boot_generation: Option<&str>,
        job: JobInfo,
    ) {
        self.record_at(agent_id, connection_id, boot_generation, job, Utc::now())
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn lock_for_test(&self) -> impl Drop + '_ {
        self.inner.lock().await
    }
}

#[derive(Clone)]
pub(crate) struct HubState {
    pub(crate) api_key: String,
    pub(crate) db: Arc<StdMutex<Connection>>,
    pub(crate) config: Arc<HubConfig>,
    pub(crate) mcp_profile: McpProfile,
    pub(crate) agents: Arc<crate::agents::lifecycle::Connections>,
    pub(crate) dispatch: Arc<crate::agents::dispatch::Dispatch>,
    pub(crate) confirmations: Arc<crate::confirmation::Confirmations>,
    pub(crate) job_cache: Arc<JobCache>,
    pub(crate) boot_generations: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) active_room: Arc<Mutex<Option<room::control::ActiveRoomConnection>>>,
    pub(crate) http: reqwest::Client,
    pub(crate) public_base_url: Option<String>,
    pub(crate) oauth_codes: Arc<Mutex<HashMap<String, oauth::OAuthAuthorizationCode>>>,
    pub(crate) oauth_tokens: Arc<Mutex<HashMap<String, oauth::OAuthAccessToken>>>,
    pub(crate) ntfy_health: Arc<Mutex<Option<crate::notify::NtfyHealthCache>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub(crate) enum McpProfile {
    #[default]
    Full,
    Coordinator,
}

impl McpProfile {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Coordinator => "coordinator",
        }
    }
}

pub(crate) mod projection {
    use agentic_gpt_protocol::{
        HubInfoAgents, HubInfoCounts, HubInfoRemoteConfirmation, HubInfoResponse, JobInfo,
        JobListItem, JobListRequest,
    };
    use anyhow::Result;
    use chrono::Utc;
    use serde_json::Value;

    use super::{HubState, JobCacheSnapshot, JobFreshness};
    use crate::{notify, registry, MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS};

    pub(crate) fn job_list_item(job: JobInfo) -> JobListItem {
        JobListItem {
            job_id: job.job_id,
            group: job.group,
            kind: job.kind,
            state: job.state,
            created_at: job.created_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }

    pub(crate) fn filter_cached_jobs(
        snapshots: &mut Vec<JobCacheSnapshot>,
        request: &JobListRequest,
    ) {
        snapshots.retain(|snapshot| {
            request
                .group
                .as_ref()
                .is_none_or(|group| snapshot.job.group.as_deref() == Some(group.as_str()))
        });
        snapshots.retain(|snapshot| request.kind.is_none_or(|kind| snapshot.job.kind == kind));
        snapshots.retain(|snapshot| {
            request
                .state
                .is_none_or(|state| snapshot.job.state == state)
        });
        snapshots.sort_by(|left, right| {
            right
                .job
                .created_at
                .cmp(&left.job.created_at)
                .then_with(|| right.job.job_id.cmp(&left.job.job_id))
        });
        snapshots.truncate(request.effective_limit());
    }

    pub(crate) fn live_job_value(mut value: Value) -> Value {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "freshness".to_string(),
                Value::String(JobFreshness::Live.label().to_string()),
            );
            object.insert(
                "observedAt".to_string(),
                Value::String(Utc::now().to_rfc3339()),
            );
        }
        value
    }

    pub(crate) fn add_cache_metadata(value: &mut Value, snapshots: &[JobCacheSnapshot]) {
        let freshness = if snapshots.is_empty() {
            JobFreshness::Unknown
        } else if snapshots
            .iter()
            .any(|snapshot| snapshot.freshness == JobFreshness::Unknown)
        {
            JobFreshness::Unknown
        } else if snapshots
            .iter()
            .any(|snapshot| snapshot.freshness == JobFreshness::Stale)
        {
            JobFreshness::Stale
        } else {
            JobFreshness::Cached
        };
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "freshness".to_string(),
                Value::String(freshness.label().to_string()),
            );
            if let Some(observed_at) = snapshots.iter().map(|snapshot| snapshot.observed_at).max() {
                object.insert(
                    "observedAt".to_string(),
                    Value::String(observed_at.to_rfc3339()),
                );
            }
        }
    }

    pub(crate) async fn build_hub_info_response(state: &HubState) -> Result<HubInfoResponse> {
        let entries = registry::registry_entries(state)?;
        let registered_count = entries.len();
        let enabled_count = entries.iter().filter(|entry| entry.enabled).count();
        let online_count = state.agents.online_count().await;
        let pending_request_count = state.dispatch.pending_count().await;
        let pending_confirmation_count = state.confirmations.pending_count().await;
        let cached_job_count = state.job_cache.count().await;
        let remote = &state.config.remote_confirmation;
        let ntfy = &remote.ntfy;
        Ok(HubInfoResponse {
            service: "agentic-gpt-hub".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            public_base_url: state.public_base_url.clone(),
            request_timeout_seconds: REQUEST_TIMEOUT_SECS,
            max_wait_seconds: MAX_WAIT_SECONDS,
            remote_confirmation: HubInfoRemoteConfirmation {
                enabled: remote.enabled,
                provider: remote.provider.clone(),
                timeout_seconds: remote.timeout_seconds,
                ntfy_configured: !notify::ntfy_not_configured(ntfy)
                    && !ntfy.callback_base_url.trim().is_empty(),
            },
            agents: HubInfoAgents {
                registered_count,
                enabled_count,
                online_count,
            },
            counts: HubInfoCounts {
                pending_request_count,
                pending_confirmation_count,
                cached_job_count,
            },
            generated_at: Utc::now(),
        })
    }
}

#[derive(Clone)]
pub(crate) struct AgentConnection {
    pub(crate) connection_id: String,
    pub(crate) sender: mpsc::UnboundedSender<OutboundAgentMessage>,
    pub(crate) last_seen_at: DateTime<Utc>,
    pub(crate) role: AgentRole,
    pub(crate) connection_mode: AgentConnectionMode,
    pub(crate) hello_received: bool,
    pub(crate) boot_generation: Option<String>,
    pub(crate) transport: AgentTransport,
    pub(crate) config_summary: Option<SafeConfigSummary>,
    pub(crate) notification_channels: Vec<NotificationChannel>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentTransport {
    WebSocket,
    Sse,
}

#[derive(Clone, Debug)]
pub(crate) enum OutboundAgentMessage {
    Text(String),
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::{JobKind, JobState};

    fn job(job_id: &str, state: JobState, updated_at: DateTime<Utc>) -> JobInfo {
        JobInfo {
            agent_id: "agent".to_string(),
            job_id: job_id.to_string(),
            group: None,
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state,
            created_at: updated_at,
            started_at: None,
            updated_at,
            finished_at: None,
            program: None,
            args: Vec::new(),
            working_directory: None,
            command_preview: None,
            exit_code: None,
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

    #[tokio::test]
    async fn job_cache_age_generation_and_ordering_are_truthful() {
        let cache = JobCache::new();
        let observed_at = Utc::now();
        let updated_at = observed_at - chrono::Duration::seconds(10);
        cache
            .record_at(
                "agent",
                "connection",
                Some("boot-a"),
                job("job", JobState::Running, updated_at),
                observed_at,
            )
            .await;
        let snapshot = cache
            .snapshot_at(
                "agent",
                "job",
                observed_at + chrono::Duration::seconds(JOB_CACHE_STALE_SECS),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.freshness, JobFreshness::Stale);
        assert_eq!(snapshot.job.updated_at, updated_at);

        cache.mark_unknown_after_restart("agent").await;
        let snapshot = cache.snapshot("agent", "job").await.unwrap();
        assert_eq!(snapshot.freshness, JobFreshness::Unknown);
        assert_eq!(snapshot.job.state, JobState::UnknownAfterRestart);
        assert_eq!(snapshot.job.updated_at, updated_at);
        assert!(snapshot.job.finished_at.is_none());

        cache
            .record(
                "agent",
                "connection",
                Some("boot-a"),
                job(
                    "job",
                    JobState::Running,
                    Utc::now() + chrono::Duration::seconds(1),
                ),
            )
            .await;
        assert_eq!(
            cache.snapshot("agent", "job").await.unwrap().job.state,
            JobState::UnknownAfterRestart
        );
    }

    #[tokio::test]
    async fn job_cache_capacity_is_bounded() {
        let cache = JobCache::new();
        let start = Utc::now();
        for index in 0..=JOB_CACHE_CAPACITY {
            cache
                .record_at(
                    "agent",
                    "connection",
                    None,
                    job(
                        &format!("job-{index}"),
                        JobState::Completed,
                        start + chrono::Duration::seconds(index as i64),
                    ),
                    start,
                )
                .await;
        }
        assert_eq!(cache.count().await, JOB_CACHE_CAPACITY);
    }
}
