#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli_i18n::UiLanguage;
    use crate::config_templates::{OptionalSection, RuntimeMode, SecretValue};
    use crate::WorkerProfile;

    use super::super::model::{SetupSeed, SetupSession};

    #[test]
    fn review_is_redacted_active_mode_only_and_reports_secret_write_intent() {
        let marker = "review-secret-marker-4f2e";
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Normal),
                tunnel_id: Some("tunnel-review".to_string()),
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
}

use std::path::PathBuf;

use crate::config_templates::{
    build_config, InitBuild, OptionalSection, PendingAction, RuntimeMode, TunnelSecretSource,
};
use crate::WorkerProfile;

use super::model::{
    ConfirmationDraft, HubReportingDraft, IdentityDraft, LimitsDraft, OptionalSectionDraft,
    RoomDraft, SandboxDraft, SectionStatus, SetupField, SetupSession, TunnelClientDraft,
    WorkspaceDraft,
};
use super::validation::{ValidationError, ValidationErrors};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewTarget {
    Basic,
    Connection,
    OptionalCenter,
    OptionalSection(OptionalSection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewItem {
    pub(crate) label_key: &'static str,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewGroup {
    pub(crate) target: ReviewTarget,
    pub(crate) status: SectionStatus,
    pub(crate) items: Vec<ReviewItem>,
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
    pub(crate) connection: ReviewGroup,
    pub(crate) optional_sections: Vec<ReviewGroup>,
    pub(crate) config_path: PathBuf,
    pub(crate) will_backup_existing_config: bool,
    pub(crate) pending_actions: Vec<PendingAction>,
    pub(crate) secret_write: Option<ReviewSecretWrite>,
}

pub(super) fn build_review_model(session: &SetupSession) -> Result<ReviewModel, ValidationErrors> {
    let input = session.build_active_input()?;
    let built = build_config(input).map_err(|_| {
        vec![ValidationError {
            field: SetupField::Mode,
            code: "config_init_build_invalid",
        }]
    })?;

    let connection = connection_group(session, &built);
    let optional_sections = [
        OptionalSection::Identity,
        OptionalSection::Workspace,
        OptionalSection::Confirmation,
        OptionalSection::Limits,
        OptionalSection::Sandbox,
        OptionalSection::Room,
        OptionalSection::TunnelClient,
        OptionalSection::HubReporting,
    ]
    .into_iter()
    .map(|section| optional_group(session, section))
    .collect();

    let mut pending_actions = built.pending.clone();
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
        connection,
        optional_sections,
        config_path: session.config_path().to_path_buf(),
        will_backup_existing_config: session.config_path().exists(),
        pending_actions,
        secret_write,
    })
}

fn connection_group(session: &SetupSession, built: &InitBuild) -> ReviewGroup {
    let mut items = vec![ReviewItem {
        label_key: "mode",
        value: format!("{:?}", built.mode),
    }];
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let draft = session.standalone();
            items.push(ReviewItem {
                label_key: "tunnel_id",
                value: draft.tunnel_id.clone(),
            });
            items.push(ReviewItem {
                label_key: "tunnel_secret_source",
                value: match draft.secret_source {
                    TunnelSecretSource::File => "file".to_string(),
                    TunnelSecretSource::Environment => "env".to_string(),
                },
            });
        }
        RuntimeMode::Hub => {
            let draft = session.hub();
            items.extend([
                ReviewItem {
                    label_key: "hub_url",
                    value: draft.hub_url.clone(),
                },
                ReviewItem {
                    label_key: "hub_transport",
                    value: draft.hub_transport.clone(),
                },
                ReviewItem {
                    label_key: "agent_id",
                    value: draft.agent_id.clone(),
                },
                ReviewItem {
                    label_key: "agent_secret",
                    value: "[REDACTED]".to_string(),
                },
            ]);
        }
        RuntimeMode::Local => items.push(ReviewItem {
            label_key: "connection",
            value: "none".to_string(),
        }),
    }
    ReviewGroup {
        target: ReviewTarget::Connection,
        status: SectionStatus::Configured,
        items,
    }
}

fn optional_group(session: &SetupSession, section: OptionalSection) -> ReviewGroup {
    let status = session.section_status(section);
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

fn optional_items(draft: OptionalSectionDraft) -> Vec<ReviewItem> {
    match draft {
        OptionalSectionDraft::Identity(IdentityDraft { display_name }) => vec![ReviewItem {
            label_key: "display_name",
            value: display_name,
        }],
        OptionalSectionDraft::Workspace(WorkspaceDraft {
            workspace_root,
            write_roots,
            read_only_roots,
            deny_roots,
        }) => vec![
            ReviewItem {
                label_key: "workspace_root",
                value: workspace_root,
            },
            ReviewItem {
                label_key: "write_roots",
                value: write_roots,
            },
            ReviewItem {
                label_key: "read_only_roots",
                value: read_only_roots,
            },
            ReviewItem {
                label_key: "deny_roots",
                value: deny_roots,
            },
        ],
        OptionalSectionDraft::Confirmation(ConfirmationDraft { provider, language }) => vec![
            ReviewItem {
                label_key: "confirmation_provider",
                value: provider,
            },
            ReviewItem {
                label_key: "confirmation_language",
                value: language,
            },
        ],
        OptionalSectionDraft::Limits(LimitsDraft {
            max_concurrent_tasks,
            max_active_jobs,
            max_file_search_context_lines,
        }) => vec![
            ReviewItem {
                label_key: "max_concurrent_tasks",
                value: max_concurrent_tasks,
            },
            ReviewItem {
                label_key: "max_active_jobs",
                value: max_active_jobs,
            },
            ReviewItem {
                label_key: "max_file_search_context_lines",
                value: max_file_search_context_lines,
            },
        ],
        OptionalSectionDraft::Sandbox(SandboxDraft {
            enabled,
            bubblewrap_path,
            required_runtime_paths,
        }) => vec![
            ReviewItem {
                label_key: "sandbox_enabled",
                value: enabled.to_string(),
            },
            ReviewItem {
                label_key: "bubblewrap_path",
                value: bubblewrap_path,
            },
            ReviewItem {
                label_key: "required_runtime_paths",
                value: required_runtime_paths,
            },
        ],
        OptionalSectionDraft::Room(RoomDraft {
            timezone,
            diary_boundary_hour,
            notebook_root,
        }) => vec![
            ReviewItem {
                label_key: "room_timezone",
                value: timezone,
            },
            ReviewItem {
                label_key: "diary_boundary_hour",
                value: diary_boundary_hour,
            },
            ReviewItem {
                label_key: "notebook_root",
                value: notebook_root,
            },
        ],
        OptionalSectionDraft::TunnelClient(TunnelClientDraft {
            version,
            cache_dir,
            auto_download,
            executable,
            download_url,
            sha256,
        }) => vec![
            ReviewItem {
                label_key: "tunnel_client_version",
                value: version,
            },
            ReviewItem {
                label_key: "tunnel_cache_dir",
                value: cache_dir,
            },
            ReviewItem {
                label_key: "tunnel_auto_download",
                value: auto_download.to_string(),
            },
            ReviewItem {
                label_key: "tunnel_executable",
                value: executable,
            },
            ReviewItem {
                label_key: "tunnel_download_url",
                value: download_url,
            },
            ReviewItem {
                label_key: "tunnel_sha256",
                value: sha256,
            },
        ],
        OptionalSectionDraft::HubReporting(HubReportingDraft { enabled, detail }) => vec![
            ReviewItem {
                label_key: "hub_reporting_enabled",
                value: enabled.to_string(),
            },
            ReviewItem {
                label_key: "hub_reporting_detail",
                value: detail,
            },
        ],
    }
}

impl SetupSession {
    pub(crate) fn review_model(&self) -> Result<ReviewModel, ValidationErrors> {
        build_review_model(self)
    }
}
