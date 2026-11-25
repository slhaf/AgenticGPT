use super::model::SetupField;

impl SetupField {
    pub(crate) fn static_choices(self) -> &'static [&'static str] {
        match self {
            Self::Mode => &["standalone", "hub", "local"],
            Self::Profile => &["normal", "room"],
            Self::TunnelSecretSource => &["file", "env"],
            Self::HubTransport => &["websocket", "sse"],
            Self::McpServerEnabled | Self::McpServerBearerAuth => &["false", "true"],
            Self::McpServerTransport => &["streamable-http", "stdio"],
            Self::ShellInitFileMode => &["default", "disabled", "path"],
            Self::ConfirmationLanguage => &["zh-CN", "en"],
            Self::PathDefaultAction => &["allow", "deny"],
            Self::EmailImapTls | Self::EmailSmtpTls => &["implicit", "starttls"],
            Self::EmailSendPolicy | Self::EmailTrashPolicy => &["allow", "confirm", "deny"],
            Self::RoomSecretGetPolicy
            | Self::RoomSecretUpsertPolicy
            | Self::RoomSecretDeletePolicy => &["allow", "confirm"],
            Self::RoomMaintenanceMode => &["local", "workflow"],
            Self::ProvisionTunnelSecret
            | Self::HttpMcpEnabled
            | Self::HubEnabled
            | Self::Bubblewrap
            | Self::HubReportingEnabled
            | Self::RoomMaintenanceAutoPush => &["false", "true"],
            Self::HubReportingDetail => &["metadata", "full"],
            Self::EventProcessCompletedLevel
            | Self::EventProcessFailedLevel
            | Self::EventProcessRejectedLevel
            | Self::EventProcessCancelledLevel
            | Self::EventProcessTimedOutLevel
            | Self::EventProcessDetachedLevel
            | Self::EventProcessUnknownAfterRestartLevel
            | Self::EventProcessSkippedLevel
            | Self::EventSkillInstallCompletedLevel
            | Self::EventSkillInstallFailedLevel
            | Self::EventSkillInstallCancelledLevel => &["inherit", "low", "medium", "high", "off"],
            _ => &[],
        }
    }

    /// Values suitable for a selection control, derived from the same catalog
    /// used by configuration editing wherever the field is catalog-backed.
    pub(crate) fn choices(self) -> Vec<String> {
        match self {
            Self::Catalog(index) => {
                let entries = super::model::catalog_entries();
                let Some(entry) = entries.get(index) else {
                    return Vec::new();
                };
                if !entry.choices.is_empty() {
                    return entry.choices.clone();
                }
                if entry.kind == "boolean" {
                    return vec!["false".into(), "true".into()];
                }
                Vec::new()
            }
            _ => self
                .static_choices()
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    /// Whether the value is an integer or port and should use a numeric editor.
    pub(crate) fn is_numeric(self) -> bool {
        match self {
            Self::HttpMcpPort
            | Self::EmailImapPort
            | Self::EmailSmtpPort
            | Self::EmailSyncInterval
            | Self::EmailRetentionDays
            | Self::EmailMaxStorageBytes
            | Self::MaxConcurrentTasks
            | Self::MaxActiveProcesses
            | Self::MaxFileSearchContextLines
            | Self::ProcessResponseBytes
            | Self::RoomSecretSyncWaitTimeoutMs
            | Self::DiaryBoundaryHour
            | Self::EventsLowTtlSeconds => true,
            Self::Catalog(index) => {
                super::model::catalog_entries()
                    .get(index)
                    .is_some_and(|entry| {
                        matches!(
                            entry.kind.as_str(),
                            "port" | "non-negative-integer" | "auto-or-non-negative-integer"
                        )
                    })
            }
            _ => false,
        }
    }

    /// Fields containing secret values or references and shown with masked input.
    pub(crate) fn is_secret_reference(self) -> bool {
        if let Self::Catalog(index) = self {
            return super::model::catalog_entries()
                .get(index)
                .is_some_and(|entry| entry.sensitive);
        }
        matches!(
            self,
            Self::HttpMcpBearerToken
                | Self::EmailPassword
                | Self::RoomSecretClientId
                | Self::RoomSecretClientSecret
                | Self::RoomSecretMasterPassword
        )
    }
}
