#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli_i18n::UiLanguage;
    use crate::config_templates::{OptionalSection, PendingAction, RuntimeMode, SecretValue};
    use crate::WorkerProfile;

    use super::super::model::{
        IdentityDraft, McpServerDraft, McpServersDraft, OptionalSectionDraft, SetupField,
        SetupSeed, SetupSession,
    };
    use super::{optional_items, ReviewEditorKind, ReviewItemTarget};

    #[test]
    fn review_is_redacted_active_mode_only_and_reports_secret_write_intent() {
        let marker = "review-secret-marker-4f2e";
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Normal),
                imported_base: None,
                tunnel_id: Some("review-tunnel".to_string()),
                tunnel_api_key: Some("file:/tmp/review-secret".to_string()),
                hub_url: Some("https://inactive-hub.example.com".to_string()),
                hub_transport: Some("sse".to_string()),
                agent_id: Some("inactive-agent".to_string()),
                agent_secret: Some(SecretValue::new("inactive-hub-secret")),
            },
            UiLanguage::En,
            PathBuf::from("/tmp/review-config.json"),
        );
        session.standalone_mut().provision_secret_now = true;
        session.standalone_mut().secret_value = Some(SecretValue::new(marker));

        let review = session.review_model().unwrap();
        let rendered = format!("{review:?}");
        assert!(!rendered.contains(marker));
        assert!(!rendered.contains("inactive-hub-secret"));
        assert!(review.secret_write.is_some());
        assert_eq!(review.mode, RuntimeMode::Standalone);
        assert_eq!(
            review.basic.target,
            super::super::review::ReviewTarget::Basic
        );
        assert!(review
            .connection
            .items
            .iter()
            .all(|item| !item.value.contains("inactive-hub.example.com")));
        assert_eq!(
            review
                .optional_sections
                .iter()
                .find(|group| group.target
                    == super::super::review::ReviewTarget::OptionalSection(OptionalSection::Room))
                .unwrap()
                .status,
            super::super::model::SectionStatus::NotApplicable
        );
    }

    #[test]
    fn review_reports_default_and_configured_optional_statuses() {
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                profile: Some(WorkerProfile::Normal),
                ..SetupSeed::default()
            },
            UiLanguage::ZhCn,
            PathBuf::from("/tmp/review-config.json"),
        );
        let default_review = session.review_model().unwrap();
        assert_eq!(
            default_review
                .optional_sections
                .iter()
                .find(|group| group.target
                    == super::super::review::ReviewTarget::OptionalSection(
                        OptionalSection::Identity
                    ))
                .unwrap()
                .status,
            super::super::model::SectionStatus::Default
        );

        let draft = session.optional_draft(OptionalSection::Identity);
        session.save_optional_section(draft).unwrap();
        let still_default = session.review_model().unwrap();
        assert_eq!(
            still_default
                .optional_sections
                .iter()
                .find(|group| group.target
                    == super::super::review::ReviewTarget::OptionalSection(
                        OptionalSection::Identity
                    ))
                .unwrap()
                .status,
            super::super::model::SectionStatus::Default
        );

        session
            .save_optional_section(OptionalSectionDraft::Identity(IdentityDraft {
                display_name: "Configured agent".into(),
            }))
            .unwrap();
        let configured_review = session.review_model().unwrap();
        assert_eq!(
            configured_review
                .optional_sections
                .iter()
                .find(|group| group.target
                    == super::super::review::ReviewTarget::OptionalSection(
                        OptionalSection::Identity
                    ))
                .unwrap()
                .status,
            super::super::model::SectionStatus::Configured
        );
    }

    #[test]
    fn review_rows_expose_stable_edit_contract_without_secret_material() {
        let marker = "review-row-secret-marker";
        let hub = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                hub_url: Some("https://hub.example.com".into()),
                agent_id: Some("review-agent".into()),
                agent_secret: Some(SecretValue::new(marker)),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/review-contract.json"),
        );
        let review = hub.review_model().unwrap();
        let agent_secret = review
            .connection
            .items
            .iter()
            .find(|item| item.field == Some(SetupField::AgentSecret))
            .unwrap();
        assert_eq!(agent_secret.editor, ReviewEditorKind::Secret);
        assert_eq!(agent_secret.target, ReviewItemTarget::Static);
        assert_eq!(agent_secret.value, "[REDACTED]");
        assert!(!format!("{review:?}").contains(marker));

        let mcp_items = optional_items(OptionalSectionDraft::McpServers(McpServersDraft {
            servers: vec![McpServerDraft {
                id: "docs".into(),
                enabled: true,
                transport: "stdio".into(),
                endpoint: "node server.mjs".into(),
            }],
        }));
        assert_eq!(mcp_items.len(), 1);
        assert_eq!(mcp_items[0].field, None);
        assert_eq!(mcp_items[0].editor, ReviewEditorKind::Compound);
        assert_eq!(
            mcp_items[0].target,
            ReviewItemTarget::McpServer { index: 0 }
        );
    }

    #[test]
    fn review_preserves_pending_actions_and_redacted_standalone_reference() {
        let default_secret = crate::utils::agentic_home()
            .unwrap()
            .join("secrets/tunnel-api-key");
        let deferred = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_id: Some("tunnel_replace-me".to_string()),
                tunnel_api_key: Some(format!("file:{}", default_secret.display())),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/review-config.json"),
        );
        let deferred_review = deferred.review_model().unwrap();
        assert!(deferred_review
            .pending_actions
            .contains(&PendingAction::ReplaceTunnelId));
        assert!(deferred_review
            .pending_actions
            .contains(&PendingAction::ProvisionTunnelSecret));
        assert!(deferred_review.secret_write.is_none());
        assert!(deferred_review.connection.items.iter().any(|item| {
            item.label_key == "tunnel_secret_reference"
                && item.value == format!("file:{}", default_secret.display())
        }));

        let mut immediate = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_id: Some("immediate-tunnel".into()),
                tunnel_api_key: Some("file:/tmp/immediate-tunnel-secret".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/review-config.json"),
        );
        immediate.standalone_mut().provision_secret_now = true;
        immediate.standalone_mut().secret_value = Some(SecretValue::new("review-secret"));
        let immediate_review = immediate.review_model().unwrap();
        assert!(!immediate_review
            .pending_actions
            .contains(&PendingAction::ProvisionTunnelSecret));
        assert!(immediate_review.secret_write.is_some());

        let hub = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                hub_url: Some("https://hub.replace-me".into()),
                agent_secret: Some(SecretValue::new("change-me")),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/review-config.json"),
        );
        let hub_review = hub.review_model().unwrap();
        assert!(hub_review
            .pending_actions
            .contains(&PendingAction::ConfigureHubUrl));
        assert!(hub_review
            .pending_actions
            .contains(&PendingAction::ReplaceAgentSecret));
    }
}

use std::path::PathBuf;

use crate::config_templates::{
    build_config, OptionalSection, PendingAction, RuntimeMode, TunnelSecretSource,
};
use crate::WorkerProfile;

use super::model::{
    default_optional_draft, ConfirmationDraft, HubReportingDraft, IdentityDraft, LimitsDraft,
    McpServersDraft, OptionalSectionDraft, RoomDraft, SandboxDraft, SectionStatus, SetupField,
    SetupSession, TunnelClientDraft, WorkspaceDraft,
};
use super::validation::ValidationErrors;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewTarget {
    Basic,
    Connection,
    #[allow(dead_code)]
    OptionalCenter,
    OptionalSection(OptionalSection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewEditorKind {
    ReadOnly,
    Text,
    Secret,
    Choice,
    MultiSelect,
    List,
    Compound,
    AutoCustom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewItemTarget {
    Static,
    McpServer { index: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewItem {
    pub(crate) field: Option<SetupField>,
    pub(crate) label_key: &'static str,
    pub(crate) value: String,
    pub(crate) editor: ReviewEditorKind,
    pub(crate) target: ReviewItemTarget,
}

impl ReviewItem {
    fn field(
        field: SetupField,
        label_key: &'static str,
        value: impl Into<String>,
        editor: ReviewEditorKind,
    ) -> Self {
        Self {
            field: Some(field),
            label_key,
            value: value.into(),
            editor,
            target: ReviewItemTarget::Static,
        }
    }

    fn read_only(
        field: Option<SetupField>,
        label_key: &'static str,
        value: impl Into<String>,
    ) -> Self {
        Self {
            field,
            label_key,
            value: value.into(),
            editor: ReviewEditorKind::ReadOnly,
            target: ReviewItemTarget::Static,
        }
    }

    fn mcp_server(index: usize, value: impl Into<String>) -> Self {
        Self {
            field: None,
            label_key: "mcp_server",
            value: value.into(),
            editor: ReviewEditorKind::Compound,
            target: ReviewItemTarget::McpServer { index },
        }
    }

    pub(crate) fn choice_values(&self) -> &'static [&'static str] {
        match self.field {
            Some(SetupField::Mode) => &["standalone", "hub", "local"],
            Some(SetupField::Profile) => &["normal", "room"],
            Some(SetupField::TunnelSecretSource) => &["file", "env"],
            Some(SetupField::HubTransport) => &["websocket", "sse"],
            Some(SetupField::ConfirmationLanguage) => &["zh-CN", "en"],
            Some(
                SetupField::ProvisionTunnelSecret
                | SetupField::SandboxEnabled
                | SetupField::TunnelAutoDownload
                | SetupField::HubReportingEnabled,
            ) => &["false", "true"],
            Some(SetupField::HubReportingDetail) => &["metadata", "full"],
            _ => &[],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewGroup {
    pub(crate) target: ReviewTarget,
    pub(crate) status: SectionStatus,
    pub(crate) items: Vec<ReviewItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReviewRowKey {
    pub(crate) group: ReviewTarget,
    pub(crate) field: Option<SetupField>,
    pub(crate) target: ReviewItemTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewSecretWrite {
    pub(crate) path: PathBuf,
    pub(crate) will_write: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewModel {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) basic: ReviewGroup,
    pub(crate) connection: ReviewGroup,
    pub(crate) optional_sections: Vec<ReviewGroup>,
    pub(crate) config_path: PathBuf,
    pub(crate) will_backup_existing_config: bool,
    pub(crate) pending_actions: Vec<PendingAction>,
    pub(crate) secret_write: Option<ReviewSecretWrite>,
}

impl ReviewModel {
    pub(crate) fn groups(&self) -> Vec<&ReviewGroup> {
        let mut groups = vec![&self.basic];
        if self.mode != RuntimeMode::Local {
            groups.push(&self.connection);
        }
        groups.extend(
            self.optional_sections
                .iter()
                .filter(|group| group.status != SectionStatus::NotApplicable),
        );
        groups
    }

    pub(crate) fn row_count(&self) -> usize {
        self.groups().iter().map(|group| group.items.len()).sum()
    }

    pub(crate) fn row(&self, index: usize) -> Option<(&ReviewGroup, &ReviewItem)> {
        let mut offset = index;
        for group in self.groups() {
            if offset < group.items.len() {
                return Some((group, &group.items[offset]));
            }
            offset = offset.saturating_sub(group.items.len());
        }
        None
    }

    pub(crate) fn row_key(&self, index: usize) -> Option<ReviewRowKey> {
        let (group, item) = self.row(index)?;
        Some(ReviewRowKey {
            group: group.target,
            field: item.field,
            target: item.target,
        })
    }

    pub(crate) fn find_row(&self, key: ReviewRowKey) -> Option<usize> {
        (0..self.row_count()).find(|index| self.row_key(*index) == Some(key))
    }
}

pub(super) fn build_review_model(session: &SetupSession) -> Result<ReviewModel, ValidationErrors> {
    let pending_actions = super::validation::build_active_input_unchecked(session)
        .ok()
        .and_then(|input| build_config(input).ok())
        .map(|built| built.pending)
        .unwrap_or_default();

    let basic = basic_group(session);
    let connection = connection_group(session);
    let optional_sections = [
        OptionalSection::Identity,
        OptionalSection::Workspace,
        OptionalSection::Confirmation,
        OptionalSection::Limits,
        OptionalSection::Sandbox,
        OptionalSection::McpServers,
        OptionalSection::Room,
        OptionalSection::TunnelClient,
        OptionalSection::HubReporting,
    ]
    .into_iter()
    .map(|section| optional_group(session, section))
    .collect();

    let mut pending_actions = pending_actions;
    let secret_write = match session.selected_mode() {
        RuntimeMode::Standalone
            if session.standalone().provision_secret_now
                && session.standalone().secret_value.is_some() =>
        {
            pending_actions.retain(|action| *action != PendingAction::ProvisionTunnelSecret);
            Some(ReviewSecretWrite {
                path: PathBuf::from(
                    session
                        .standalone()
                        .secret_path
                        .trim()
                        .strip_prefix("file:")
                        .unwrap_or_else(|| session.standalone().secret_path.trim()),
                ),
                will_write: true,
            })
        }
        _ => None,
    };

    Ok(ReviewModel {
        mode: session.selected_mode(),
        profile: session.selected_profile(),
        basic,
        connection,
        optional_sections,
        config_path: session.config_path().to_path_buf(),
        will_backup_existing_config: session.config_path().exists(),
        pending_actions,
        secret_write,
    })
}

fn basic_group(session: &SetupSession) -> ReviewGroup {
    ReviewGroup {
        target: ReviewTarget::Basic,
        status: SectionStatus::Configured,
        items: vec![
            ReviewItem::field(
                SetupField::Mode,
                "mode",
                format!("{:?}", session.selected_mode()),
                ReviewEditorKind::Choice,
            ),
            ReviewItem::field(
                SetupField::Profile,
                "profile",
                format!("{:?}", session.selected_profile()).to_lowercase(),
                ReviewEditorKind::Choice,
            ),
        ],
    }
}

fn connection_group(session: &SetupSession) -> ReviewGroup {
    let mut items = Vec::new();
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let draft = session.standalone();
            items.push(ReviewItem::field(
                SetupField::TunnelId,
                "tunnel_id",
                draft.tunnel_id.clone(),
                ReviewEditorKind::Text,
            ));
            items.push(ReviewItem::field(
                SetupField::TunnelSecretSource,
                "tunnel_secret_source",
                match draft.secret_source {
                    TunnelSecretSource::File => "file".to_string(),
                    TunnelSecretSource::Environment => "env".to_string(),
                },
                ReviewEditorKind::Choice,
            ));
            let reference_field = match draft.secret_source {
                TunnelSecretSource::File => SetupField::TunnelSecretPath,
                TunnelSecretSource::Environment => SetupField::TunnelSecretEnvironment,
            };
            items.push(ReviewItem::field(
                reference_field,
                "tunnel_secret_reference",
                tunnel_secret_reference(draft),
                ReviewEditorKind::Text,
            ));
            if draft.secret_source == TunnelSecretSource::File {
                items.push(ReviewItem::field(
                    SetupField::ProvisionTunnelSecret,
                    "provision_tunnel_secret",
                    draft.provision_secret_now.to_string(),
                    ReviewEditorKind::Choice,
                ));
                if draft.provision_secret_now {
                    items.push(ReviewItem::field(
                        SetupField::TunnelSecretValue,
                        "tunnel_secret_value",
                        if draft.secret_value.is_some() {
                            "[REDACTED]"
                        } else {
                            ""
                        },
                        ReviewEditorKind::Secret,
                    ));
                }
            }
        }
        RuntimeMode::Hub => {
            let draft = session.hub();
            items.extend([
                ReviewItem::field(
                    SetupField::HubUrl,
                    "hub_url",
                    draft.hub_url.clone(),
                    ReviewEditorKind::Text,
                ),
                ReviewItem::field(
                    SetupField::HubTransport,
                    "hub_transport",
                    draft.hub_transport.clone(),
                    ReviewEditorKind::Choice,
                ),
                ReviewItem::field(
                    SetupField::AgentId,
                    "agent_id",
                    draft.agent_id.clone(),
                    ReviewEditorKind::Text,
                ),
                ReviewItem::field(
                    SetupField::AgentSecret,
                    "agent_secret",
                    if draft.agent_secret.is_some() {
                        "[REDACTED]"
                    } else {
                        ""
                    },
                    ReviewEditorKind::Secret,
                ),
            ]);
        }
        RuntimeMode::Local => items.push(ReviewItem::read_only(None, "connection", "none")),
    }
    ReviewGroup {
        target: ReviewTarget::Connection,
        status: SectionStatus::Configured,
        items,
    }
}

fn tunnel_secret_reference(draft: &super::model::StandaloneDraft) -> String {
    match draft.secret_source {
        TunnelSecretSource::File => format!(
            "file:{}",
            draft
                .secret_path
                .trim()
                .strip_prefix("file:")
                .unwrap_or_else(|| draft.secret_path.trim())
        ),
        TunnelSecretSource::Environment => format!(
            "env:{}",
            draft
                .secret_environment
                .trim()
                .strip_prefix("env:")
                .unwrap_or_else(|| draft.secret_environment.trim())
        ),
    }
}

fn optional_group(session: &SetupSession, section: OptionalSection) -> ReviewGroup {
    let status = if session.section_status(section) == SectionStatus::NotApplicable {
        SectionStatus::NotApplicable
    } else {
        let draft = session.optional_draft(section);
        if draft == default_optional_draft(session.language(), section) {
            SectionStatus::Default
        } else {
            SectionStatus::Configured
        }
    };
    let items = if status == SectionStatus::NotApplicable {
        Vec::new()
    } else {
        optional_items(session.optional_draft(section))
    };
    ReviewGroup {
        target: ReviewTarget::OptionalSection(section),
        status,
        items,
    }
}

fn confirmation_channels_summary(raw: &str) -> String {
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(channels) if channels.is_empty() => "none".to_string(),
        Ok(channels) => channels.join(" → "),
        Err(_) => raw.to_string(),
    }
}

fn optional_items(draft: OptionalSectionDraft) -> Vec<ReviewItem> {
    match draft {
        OptionalSectionDraft::Identity(IdentityDraft { display_name }) => vec![ReviewItem::field(
            SetupField::DisplayName,
            "display_name",
            display_name,
            ReviewEditorKind::Text,
        )],
        OptionalSectionDraft::Workspace(WorkspaceDraft {
            workspace_root,
            write_roots,
            read_only_roots,
            deny_roots,
        }) => vec![
            ReviewItem::field(
                SetupField::WorkspaceRoot,
                "workspace_root",
                workspace_root,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::WriteRoots,
                "write_roots",
                write_roots,
                ReviewEditorKind::List,
            ),
            ReviewItem::field(
                SetupField::ReadOnlyRoots,
                "read_only_roots",
                read_only_roots,
                ReviewEditorKind::List,
            ),
            ReviewItem::field(
                SetupField::DenyRoots,
                "deny_roots",
                deny_roots,
                ReviewEditorKind::List,
            ),
        ],
        OptionalSectionDraft::Confirmation(ConfirmationDraft { channels, language }) => vec![
            ReviewItem::field(
                SetupField::ConfirmationChannels,
                "confirmation_channels",
                confirmation_channels_summary(&channels),
                ReviewEditorKind::MultiSelect,
            ),
            ReviewItem::field(
                SetupField::ConfirmationLanguage,
                "confirmation_language",
                language,
                ReviewEditorKind::Choice,
            ),
        ],
        OptionalSectionDraft::Limits(LimitsDraft {
            max_concurrent_tasks,
            max_active_jobs,
            max_file_search_context_lines,
        }) => vec![
            ReviewItem::field(
                SetupField::MaxConcurrentTasks,
                "max_concurrent_tasks",
                max_concurrent_tasks,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::MaxActiveJobs,
                "max_active_jobs",
                max_active_jobs,
                ReviewEditorKind::AutoCustom,
            ),
            ReviewItem::field(
                SetupField::MaxFileSearchContextLines,
                "max_file_search_context_lines",
                max_file_search_context_lines,
                ReviewEditorKind::Text,
            ),
        ],
        OptionalSectionDraft::Sandbox(SandboxDraft {
            enabled,
            bubblewrap_path,
            required_runtime_paths,
        }) => vec![
            ReviewItem::field(
                SetupField::SandboxEnabled,
                "sandbox_enabled",
                enabled.to_string(),
                ReviewEditorKind::Choice,
            ),
            ReviewItem::field(
                SetupField::BubblewrapPath,
                "bubblewrap_path",
                bubblewrap_path,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::RequiredRuntimePaths,
                "required_runtime_paths",
                required_runtime_paths,
                ReviewEditorKind::List,
            ),
        ],
        OptionalSectionDraft::McpServers(McpServersDraft { servers }) => {
            let items: Vec<_> = servers
                .into_iter()
                .enumerate()
                .map(|(index, server)| {
                    ReviewItem::mcp_server(
                        index,
                        format!(
                            "{} · {} · {} · {}",
                            server.id,
                            if server.enabled {
                                "enabled"
                            } else {
                                "disabled"
                            },
                            server.transport,
                            server.endpoint
                        ),
                    )
                })
                .collect();
            items
        }
        OptionalSectionDraft::Room(RoomDraft {
            timezone,
            diary_boundary_hour,
            notebook_root,
        }) => vec![
            ReviewItem::field(
                SetupField::RoomTimezone,
                "room_timezone",
                timezone,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::DiaryBoundaryHour,
                "diary_boundary_hour",
                diary_boundary_hour,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::NotebookRoot,
                "notebook_root",
                notebook_root,
                ReviewEditorKind::Text,
            ),
        ],
        OptionalSectionDraft::TunnelClient(TunnelClientDraft {
            version,
            cache_dir,
            auto_download,
            executable,
            download_url,
            sha256,
        }) => vec![
            ReviewItem::field(
                SetupField::TunnelClientVersion,
                "tunnel_client_version",
                version,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::TunnelCacheDir,
                "tunnel_cache_dir",
                cache_dir,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::TunnelAutoDownload,
                "tunnel_auto_download",
                auto_download.to_string(),
                ReviewEditorKind::Choice,
            ),
            ReviewItem::field(
                SetupField::TunnelExecutable,
                "tunnel_executable",
                executable,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::TunnelDownloadUrl,
                "tunnel_download_url",
                download_url,
                ReviewEditorKind::Text,
            ),
            ReviewItem::field(
                SetupField::TunnelSha256,
                "tunnel_sha256",
                sha256,
                ReviewEditorKind::Text,
            ),
        ],
        OptionalSectionDraft::HubReporting(HubReportingDraft { enabled, detail }) => vec![
            ReviewItem::field(
                SetupField::HubReportingEnabled,
                "hub_reporting_enabled",
                enabled.to_string(),
                ReviewEditorKind::Choice,
            ),
            ReviewItem::field(
                SetupField::HubReportingDetail,
                "hub_reporting_detail",
                detail,
                ReviewEditorKind::Choice,
            ),
        ],
    }
}

impl SetupSession {
    pub(crate) fn review_model(&self) -> Result<ReviewModel, ValidationErrors> {
        build_review_model(self)
    }
}
