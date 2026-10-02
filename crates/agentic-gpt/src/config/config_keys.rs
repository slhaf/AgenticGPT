use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::{
    cli_i18n::{self, UiLanguage},
    config::{
        self, normalize_confirmation_language, Config, EventNotificationLevel, ReportingDetail,
        RoomMaintenanceMode, RuntimeMode, INTERNAL_EVENT_TYPES,
    },
    policy, WorkerProfile,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigValueKind {
    String,
    Path,
    Boolean,
    Port,
    NonNegativeInteger,
    AutoOrNonNegativeInteger,
    EventNotificationLevel,
    JsonStringArray,
    JsonStringArrayOrNull,
    JsonPathArray,
    NullableString,
    NullablePath,
    ConfirmationChannels,
    Language,
    HubTransport,
    ReportingDetail,
    RuntimeMode,
    WorkerProfile,
    RoomMaintenanceMode,
}

impl ConfigValueKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Path => "path",
            Self::Boolean => "boolean",
            Self::Port => "port",
            Self::NonNegativeInteger => "non-negative-integer",
            Self::AutoOrNonNegativeInteger => "auto-or-non-negative-integer",
            Self::JsonStringArray => "json-string-array",
            Self::JsonStringArrayOrNull => "json-string-array-or-null",
            Self::JsonPathArray => "json-path-array",
            Self::NullableString => "nullable-string",
            Self::NullablePath => "nullable-path",
            Self::ConfirmationChannels => "ordered-string-array",
            Self::Language => "language",
            Self::HubTransport => "hub-transport",
            Self::ReportingDetail => "reporting-detail",
            Self::RuntimeMode => "runtime-mode",
            Self::WorkerProfile => "worker-profile",
            Self::RoomMaintenanceMode => "room-maintenance-mode",
            Self::EventNotificationLevel => "event-notification-level",
        }
    }

    fn choices(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Boolean => Some(&["true", "false"]),
            Self::ConfirmationChannels => Some(&["freedesktop", "ntfy"]),
            Self::HubTransport => Some(&["websocket", "sse"]),
            Self::ReportingDetail => Some(&["metadata", "full"]),
            Self::RuntimeMode => Some(&["standalone", "hub", "local"]),
            Self::WorkerProfile => Some(&["normal", "room"]),
            Self::RoomMaintenanceMode => Some(&["local", "workflow"]),
            Self::EventNotificationLevel => Some(&["low", "medium", "high", "off"]),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum ConfigSection {
    Runtime,
    Identity,
    Hub,
    HttpMcp,
    Confirmation,
    Sandbox,
    Limits,
    Skills,
    Room,
    Tunnel,
    Events,
}

impl ConfigSection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::HttpMcp => "http-mcp",
            Self::Identity => "identity",
            Self::Hub => "hub",
            Self::Confirmation => "confirmation",
            Self::Sandbox => "sandbox",
            Self::Limits => "limits",
            Self::Skills => "skills",
            Self::Room => "room",
            Self::Tunnel => "tunnel",
            Self::Events => "events",
        }
    }

    fn label(self, language: UiLanguage) -> &'static str {
        match (self, language) {
            (Self::Runtime, UiLanguage::En) => "Runtime",
            (Self::Runtime, UiLanguage::ZhCn) => "运行时",
            (Self::Identity, UiLanguage::En) => "Identity",
            (Self::Identity, UiLanguage::ZhCn) => "身份",
            (Self::Hub, UiLanguage::En) => "Hub",
            (Self::HttpMcp, UiLanguage::En) => "HTTP MCP",
            (Self::HttpMcp, UiLanguage::ZhCn) => "HTTP MCP",
            (Self::Hub, UiLanguage::ZhCn) => "Hub",
            (Self::Confirmation, UiLanguage::En) => "Confirmation",
            (Self::Confirmation, UiLanguage::ZhCn) => "确认",
            (Self::Sandbox, UiLanguage::En) => "Sandbox",
            (Self::Sandbox, UiLanguage::ZhCn) => "沙箱",
            (Self::Limits, UiLanguage::En) => "Limits",
            (Self::Limits, UiLanguage::ZhCn) => "限制",
            (Self::Skills, UiLanguage::En) => "Skills",
            (Self::Skills, UiLanguage::ZhCn) => "技能",
            (Self::Room, UiLanguage::En) => "Room",
            (Self::Room, UiLanguage::ZhCn) => "Room",
            (Self::Tunnel, UiLanguage::En) => "Tunnel",
            (Self::Tunnel, UiLanguage::ZhCn) => "隧道",
            (Self::Events, UiLanguage::En) => "Events",
            (Self::Events, UiLanguage::ZhCn) => "事件",
        }
    }
}

pub(crate) struct LocalizedText {
    pub(crate) en: &'static str,
    pub(crate) zh_cn: &'static str,
}

pub(crate) struct ConfigKeySpec {
    pub(crate) key: &'static str,
    pub(crate) section: ConfigSection,
    pub(crate) kind: ConfigValueKind,
    pub(crate) nullable: bool,
    pub(crate) description: LocalizedText,
    pub(crate) example: &'static str,
    pub(crate) alias_of: Option<&'static str>,
    apply: fn(&mut Config, &str) -> Result<()>,
}

macro_rules! config_key {
    ($key:literal, $section:ident, $kind:ident, $nullable:expr, $en:expr, $zh_cn:expr, $example:expr, $apply:ident) => {
        ConfigKeySpec {
            key: $key,
            section: ConfigSection::$section,
            kind: ConfigValueKind::$kind,
            nullable: $nullable,
            description: LocalizedText {
                en: $en,
                zh_cn: $zh_cn,
            },
            example: $example,
            alias_of: None,
            apply: $apply,
        }
    };
    ($key:literal, $section:ident, $kind:ident, $nullable:expr, $en:expr, $zh_cn:expr, $example:expr, $apply:ident, $alias_of:literal) => {
        ConfigKeySpec {
            key: $key,
            section: ConfigSection::$section,
            kind: ConfigValueKind::$kind,
            nullable: $nullable,
            description: LocalizedText {
                en: $en,
                zh_cn: $zh_cn,
            },
            example: $example,
            alias_of: Some($alias_of),
            apply: $apply,
        }
    };
}

pub(crate) static CONFIG_KEYS: &[ConfigKeySpec] = &[
    config_key!(
        "mode",
        Runtime,
        RuntimeMode,
        false,
        "Runtime mode selected by the configuration.",
        "由配置选择的运行模式。",
        "standalone",
        set_mode
    ),
    config_key!(
        "profile",
        Runtime,
        WorkerProfile,
        false,
        "Capability profile selected by the configuration.",
        "由配置选择的能力配置。",
        "normal",
        set_profile
    ),
    config_key!(
        "agentId",
        Identity,
        String,
        false,
        "Stable identifier reported by the agent.",
        "代理上报的稳定标识符。",
        "laptop",
        set_agent_id
    ),
    config_key!(
        "displayName",
        Identity,
        String,
        false,
        "Human-readable name shown for this agent.",
        "此代理显示给用户的名称。",
        "Desk Agent",
        set_display_name
    ),
    config_key!(
        "workspaceRoot",
        Identity,
        Path,
        false,
        "Root directory used as the agent workspace.",
        "代理工作区使用的根目录。",
        "/home/user/workspace",
        set_workspace_root
    ),
    config_key!(
        "hub.url",
        Hub,
        String,
        false,
        "Hub URL used for agent communication.",
        "代理通信使用的 Hub URL。",
        "http://localhost:8787",
        set_hub_url
    ),
    config_key!(
        "hub.transport",
        Hub,
        HubTransport,
        false,
        "Hub transport; websocket or sse.",
        "Hub 传输方式：websocket 或 sse。",
        "websocket",
        set_hub_transport
    ),
    config_key!(
        "hub.agentSecret",
        Hub,
        String,
        false,
        "Agent authentication secret or reference.",
        "代理认证密钥或密钥引用。",
        "env:AGENT_SECRET",
        set_agent_secret
    ),
    config_key!(
        "httpMcp.enabled",
        HttpMcp,
        Boolean,
        false,
        "Enable the standalone inbound HTTP MCP endpoint.",
        "启用 Standalone 入站 HTTP MCP 端点。",
        "false",
        set_http_mcp_enabled
    ),
    config_key!(
        "httpMcp.host",
        HttpMcp,
        String,
        false,
        "Host address for the standalone HTTP MCP listener.",
        "Standalone HTTP MCP 监听器使用的主机地址。",
        "127.0.0.1",
        set_http_mcp_host
    ),
    config_key!(
        "httpMcp.port",
        HttpMcp,
        Port,
        false,
        "TCP port for the standalone HTTP MCP listener.",
        "Standalone HTTP MCP 监听器使用的 TCP 端口。",
        "8765",
        set_http_mcp_port
    ),
    config_key!(
        "httpMcp.publicUrl",
        HttpMcp,
        NullableString,
        true,
        "Optional external HTTPS origin advertised for standalone ChatGPT OAuth.",
        "可选的外部 HTTPS 来源，用于 Standalone ChatGPT OAuth。",
        "https://mcp.example.com",
        set_http_mcp_public_url
    ),
    config_key!(
        "httpMcp.bearerToken",
        HttpMcp,
        String,
        false,
        "Bearer token reference; use file:/absolute/path or env:NAME, never plaintext.",
        "Bearer token 引用；使用 file:/absolute/path 或 env:NAME，不能使用明文。",
        "env:HTTP_MCP_TOKEN",
        set_http_mcp_bearer_token
    ),
    config_key!(
        "httpMcp.allowHosts",
        HttpMcp,
        JsonStringArrayOrNull,
        true,
        "JSON authority array, null, or [\"*\"] for unrestricted Host validation.",
        "JSON authority 数组、null 或 [\"*\"]（不限制 Host 校验）。",
        r#"["localhost","127.0.0.1","::1"]"#,
        set_http_mcp_allow_hosts
    ),
    config_key!(
        "confirmationProvider.channels",
        Confirmation,
        ConfirmationChannels,
        false,
        "Ordered confirmation fallback channels; the first available channel handles the request.",
        "有序确认降级通道；按列表顺序尝试，首个可用通道处理请求。",
        r#"["freedesktop","ntfy"]"#,
        set_confirmation_channels
    ),
    config_key!(
        "confirmationLanguage",
        Confirmation,
        Language,
        false,
        "Language used for confirmation prompts.",
        "确认提示使用的语言。",
        "en",
        set_confirmation_language
    ),
    config_key!(
        "sandbox.enabled",
        Sandbox,
        Boolean,
        false,
        "Enable bubblewrap sandbox execution.",
        "启用 bubblewrap 沙箱执行。",
        "true",
        set_sandbox_enabled
    ),
    config_key!(
        "sandbox.bubblewrapPath",
        Sandbox,
        Path,
        false,
        "Path or command name used to invoke bubblewrap.",
        "调用 bubblewrap 使用的路径或命令名。",
        "bwrap",
        set_bubblewrap_path
    ),
    config_key!(
        "sandbox.requiredRuntimePaths",
        Sandbox,
        JsonPathArray,
        false,
        "JSON array of runtime paths exposed to the sandbox.",
        "以 JSON 数组表示的沙箱运行时路径。",
        r#"["/usr","/opt/runtime"]"#,
        set_required_runtime_paths
    ),
    config_key!(
        "backupLimit",
        Limits,
        NonNegativeInteger,
        false,
        "Maximum number of configuration backups to retain.",
        "保留的配置备份文件最大数量。",
        "7",
        set_backup_limit
    ),
    config_key!(
        "limits.maxConcurrentTasks",
        Limits,
        NonNegativeInteger,
        false,
        "Maximum concurrently running child processes within one process.batch call.",
        "单次 process.batch 中同时运行的子进程最大数量。",
        "4",
        set_max_concurrent_tasks
    ),
    config_key!(
        "limits.maxActiveProcesses",
        Limits,
        AutoOrNonNegativeInteger,
        false,
        "Total active Process capacity: auto or a non-negative integer.",
        "活动 Process 总容量：auto 或非负整数。",
        "auto",
        set_max_active_processes
    ),
    config_key!(
        "limits.maxFileSearchContextLines",
        Limits,
        NonNegativeInteger,
        false,
        "File-search context lines, from 0 through 100.",
        "文件搜索上下文行数，范围为 0 到 100。",
        "12",
        set_max_file_search_context_lines
    ),
    config_key!(
        "limits.processResponseBytes",
        Limits,
        NonNegativeInteger,
        false,
        "Default serialized Process response budget shared by process.exec, process.batch, skills.run, mcp.callTool, and process.read. If process.read omits maxBytes, it uses this value; an explicit maxBytes overrides it for that read only. Configured range: 4096–1048576 bytes.",
        "process.exec、process.batch、skills.run、mcp.callTool 和 process.read 共用的序列化 Process 响应默认预算。process.read 省略 maxBytes 时使用此值；显式 maxBytes 仅覆盖本次 read。配置范围：4096–1048576 字节。",
        "8192",
        set_process_response_bytes
    ),
    config_key!(
        "skills.maxFiles",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum files accepted in a skill package.",
        "技能包允许的最大文件数。",
        "200",
        set_skills_max_files
    ),
    config_key!(
        "skills.maxFileBytes",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum size of one skill file in bytes.",
        "单个技能文件的最大字节数。",
        "1048576",
        set_skills_max_file_bytes
    ),
    config_key!(
        "skills.maxPackageBytes",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum uncompressed skill package size in bytes.",
        "解压后技能包的最大字节数。",
        "10485760",
        set_skills_max_package_bytes
    ),
    config_key!(
        "skills.maxSkillMdBytes",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum SKILL.md size in bytes.",
        "SKILL.md 的最大字节数。",
        "262144",
        set_skills_max_skill_md_bytes
    ),
    config_key!(
        "skills.maxInlineBytes",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum inline skill content size in bytes.",
        "内联技能内容的最大字节数。",
        "65536",
        set_skills_max_inline_bytes
    ),
    config_key!(
        "skills.connectTimeoutSecs",
        Skills,
        NonNegativeInteger,
        false,
        "Skill connection timeout in seconds.",
        "技能连接超时时间（秒）。",
        "10",
        set_skills_connect_timeout_secs
    ),
    config_key!(
        "skills.requestTimeoutSecs",
        Skills,
        NonNegativeInteger,
        false,
        "Skill request timeout in seconds.",
        "技能请求超时时间（秒）。",
        "30",
        set_skills_request_timeout_secs
    ),
    config_key!(
        "skills.idleTimeoutSecs",
        Skills,
        NonNegativeInteger,
        false,
        "Idle skill connection timeout in seconds.",
        "技能空闲连接超时时间（秒）。",
        "30",
        set_skills_idle_timeout_secs
    ),
    config_key!(
        "skills.maxRedirects",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum redirects followed for skill downloads.",
        "技能下载允许跟随的最大重定向次数。",
        "5",
        set_skills_max_redirects
    ),
    config_key!(
        "skills.maxConcurrentInstalls",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum concurrent skill installations.",
        "并发技能安装的最大数量。",
        "2",
        set_skills_max_concurrent_installs
    ),
    config_key!(
        "skills.maxParallelDownloads",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum parallel skill downloads.",
        "并行技能下载的最大数量。",
        "4",
        set_skills_max_parallel_downloads
    ),
    config_key!(
        "skills.maxAttempts",
        Skills,
        NonNegativeInteger,
        false,
        "Maximum attempts for one skill operation.",
        "单次技能操作的最大尝试次数。",
        "3",
        set_skills_max_attempts
    ),
    config_key!(
        "skills.totalDeadlineSecs",
        Skills,
        NonNegativeInteger,
        false,
        "Total deadline for a skill operation in seconds.",
        "技能操作的总截止时间（秒）。",
        "600",
        set_skills_total_deadline_secs
    ),
    config_key!(
        "skills.allowedHosts",
        Skills,
        JsonStringArray,
        false,
        "JSON array of hosts allowed for skill downloads.",
        "允许技能下载的主机 JSON 数组。",
        r#"["skills.example.com"]"#,
        set_skills_allowed_hosts
    ),
    config_key!(
        "room.repositoryRoot",
        Room,
        NullablePath,
        true,
        "Room repository root path, or null to use the workspace default.",
        "Room 仓库根目录；使用 null 可恢复工作区默认值。",
        "null",
        set_repository_root
    ),
    config_key!(
        "room.timezone",
        Room,
        String,
        false,
        "IANA timezone used by room diary operations.",
        "房间日记操作使用的 IANA 时区。",
        "Asia/Shanghai",
        set_room_timezone
    ),
    config_key!(
        "room.diaryDayBoundaryHour",
        Room,
        NonNegativeInteger,
        false,
        "Hour at which a diary day starts, from 0 through 23.",
        "日记日期开始的小时，范围为 0 到 23。",
        "5",
        set_diary_day_boundary_hour
    ),
    config_key!(
        "room.maintenance.mode",
        Room,
        RoomMaintenanceMode,
        false,
        "Room maintenance execution mode: local or workflow.",
        "Room 维护执行模式：local 或 workflow。",
        "local",
        set_room_maintenance_mode
    ),
    config_key!(
        "room.maintenance.autoPush",
        Room,
        Boolean,
        false,
        "Synchronize successful local Room maintenance to the remote when possible.",
        "本地 Room 维护成功后，尽可能同步到远端。",
        "false",
        set_room_maintenance_auto_push
    ),
    config_key!(
        "tunnel.tunnelId",
        Tunnel,
        String,
        false,
        "Identifier of the configured tunnel.",
        "已配置隧道的标识符。",
        "tunnel-id",
        set_tunnel_id
    ),
    config_key!(
        "tunnel.apiKey",
        Tunnel,
        String,
        false,
        "Tunnel API key or secret reference.",
        "隧道 API 密钥或密钥引用。",
        "env:TUNNEL_API_KEY",
        set_tunnel_api_key
    ),
    config_key!(
        "tunnel.client.version",
        Tunnel,
        NullableString,
        true,
        "Managed tunnel client version, or null for the default.",
        "托管隧道客户端版本；使用 null 可恢复默认值。",
        "null",
        set_tunnel_client_version
    ),
    config_key!(
        "tunnel.client.cacheDir",
        Tunnel,
        Path,
        false,
        "Directory used to cache the tunnel client.",
        "隧道客户端缓存目录。",
        "~/.cache/agentic-gpt/tunnel-client",
        set_tunnel_client_cache_dir
    ),
    config_key!(
        "tunnel.client.autoDownload",
        Tunnel,
        Boolean,
        false,
        "Allow automatic tunnel client downloads.",
        "允许自动下载隧道客户端。",
        "true",
        set_tunnel_client_auto_download
    ),
    config_key!(
        "tunnel.client.executable",
        Tunnel,
        NullablePath,
        true,
        "Explicit tunnel client executable, or null for managed mode.",
        "显式隧道客户端可执行文件；使用 null 可恢复托管模式。",
        "null",
        set_tunnel_client_executable
    ),
    config_key!(
        "tunnel.client.downloadUrl",
        Tunnel,
        NullableString,
        true,
        "Custom tunnel client download URL, or null.",
        "自定义隧道客户端下载 URL；也可使用 null。",
        "null",
        set_tunnel_client_download_url
    ),
    config_key!(
        "tunnel.client.sha256",
        Tunnel,
        NullableString,
        true,
        "SHA-256 for a custom tunnel client, or null.",
        "自定义隧道客户端的 SHA-256；也可使用 null。",
        "null",
        set_tunnel_client_sha256
    ),
    config_key!(
        "tunnel.hubReporting.enabled",
        Tunnel,
        Boolean,
        false,
        "Enable tunnel hub reporting.",
        "启用隧道 Hub 上报。",
        "false",
        set_tunnel_hub_reporting_enabled
    ),
    config_key!(
        "tunnel.hubReporting.detail",
        Tunnel,
        ReportingDetail,
        false,
        "Hub reporting detail: metadata or full.",
        "Hub 上报详细程度：metadata 或 full。",
        "metadata",
        set_tunnel_hub_reporting_detail
    ),
    config_key!(
        "events.lowTtlSeconds",
        Events,
        NonNegativeInteger,
        false,
        "Low-severity event lifetime in seconds; zero expires immediately.",
        "低等级事件保留秒数；0 表示立即过期。",
        "86400",
        set_events_low_ttl_seconds
    ),
    config_key!(
        "events.internalOverrides.process.completed",
        Events,
        EventNotificationLevel,
        true,
        "Override process.completed notification level; null restores the default.",
        "覆写 process.completed 通知等级；null 恢复默认值。",
        "low",
        set_process_completed_level
    ),
    config_key!(
        "events.internalOverrides.process.failed",
        Events,
        EventNotificationLevel,
        true,
        "Override process.failed notification level; null restores the default.",
        "覆写 process.failed 通知等级；null 恢复默认值。",
        "low",
        set_process_failed_level
    ),
    config_key!(
        "events.internalOverrides.process.rejected",
        Events,
        EventNotificationLevel,
        true,
        "Override process.rejected notification level; null restores the default.",
        "覆写 process.rejected 通知等级；null 恢复默认值。",
        "low",
        set_process_rejected_level
    ),
    config_key!(
        "events.internalOverrides.process.cancelled",
        Events,
        EventNotificationLevel,
        true,
        "Override process.cancelled notification level; null restores the default.",
        "覆写 process.cancelled 通知等级；null 恢复默认值。",
        "low",
        set_process_cancelled_level
    ),
    config_key!(
        "events.internalOverrides.process.timed_out",
        Events,
        EventNotificationLevel,
        true,
        "Override process.timed_out notification level; null restores the default.",
        "覆写 process.timed_out 通知等级；null 恢复默认值。",
        "low",
        set_process_timed_out_level
    ),
    config_key!(
        "events.internalOverrides.process.detached",
        Events,
        EventNotificationLevel,
        true,
        "Override process.detached notification level; null restores the default.",
        "覆写 process.detached 通知等级；null 恢复默认值。",
        "low",
        set_process_detached_level
    ),
    config_key!(
        "events.internalOverrides.process.unknown_after_restart",
        Events,
        EventNotificationLevel,
        true,
        "Override process.unknown_after_restart notification level; null restores the default.",
        "覆写 process.unknown_after_restart 通知等级；null 恢复默认值。",
        "low",
        set_process_unknown_after_restart_level
    ),
    config_key!(
        "events.internalOverrides.process.skipped",
        Events,
        EventNotificationLevel,
        true,
        "Override process.skipped notification level; null restores the default.",
        "覆写 process.skipped 通知等级；null 恢复默认值。",
        "low",
        set_process_skipped_level
    ),
    config_key!(
        "events.internalOverrides.skill_install.completed",
        Events,
        EventNotificationLevel,
        true,
        "Override skill_install.completed notification level; null restores the default.",
        "覆写 skill_install.completed 通知等级；null 恢复默认值。",
        "low",
        set_skill_install_completed_level
    ),
    config_key!(
        "events.internalOverrides.skill_install.failed",
        Events,
        EventNotificationLevel,
        true,
        "Override skill_install.failed notification level; null restores the default.",
        "覆写 skill_install.failed 通知等级；null 恢复默认值。",
        "low",
        set_skill_install_failed_level
    ),
    config_key!(
        "events.internalOverrides.skill_install.cancelled",
        Events,
        EventNotificationLevel,
        true,
        "Override skill_install.cancelled notification level; null restores the default.",
        "覆写 skill_install.cancelled 通知等级；null 恢复默认值。",
        "low",
        set_skill_install_cancelled_level
    ),
];

pub(crate) fn apply_config_key(config: &mut Config, key: &str, value: &str) -> Result<()> {
    let spec = CONFIG_KEYS
        .iter()
        .find(|spec| spec.key == key)
        .ok_or_else(|| anyhow!("unsupported config key: {key}"))?;
    (spec.apply)(config, value)?;
    if spec.section == ConfigSection::HttpMcp {
        config.validate_http_mcp()?;
    }
    Ok(())
}

#[derive(Serialize)]
struct ConfigKeysOutput {
    keys: Vec<ConfigKeyOutput>,
}

#[derive(Serialize)]
struct ConfigKeyOutput {
    key: &'static str,
    section: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    choices: Option<&'static [&'static str]>,
    example: &'static str,
    description: ConfigDescriptionOutput,
    #[serde(rename = "aliasOf", skip_serializing_if = "Option::is_none")]
    alias_of: Option<&'static str>,
}

#[derive(Serialize)]
struct ConfigDescriptionOutput {
    en: &'static str,
    #[serde(rename = "zhCN")]
    zh_cn: &'static str,
}

const CONFIG_SECTION_ORDER: [ConfigSection; 11] = [
    ConfigSection::Runtime,
    ConfigSection::Identity,
    ConfigSection::Hub,
    ConfigSection::HttpMcp,
    ConfigSection::Confirmation,
    ConfigSection::Sandbox,
    ConfigSection::Limits,
    ConfigSection::Skills,
    ConfigSection::Room,
    ConfigSection::Tunnel,
    ConfigSection::Events,
];

pub(super) fn print_config_keys(
    section: Option<ConfigSection>,
    json: bool,
    language: UiLanguage,
) -> Result<()> {
    if json {
        let output = ConfigKeysOutput {
            keys: CONFIG_KEYS
                .iter()
                .filter(|spec| section.is_none_or(|section| spec.section == section))
                .map(|spec| ConfigKeyOutput {
                    key: spec.key,
                    section: spec.section.as_str(),
                    kind: spec.kind.as_str(),
                    nullable: spec.nullable,
                    choices: spec.kind.choices(),
                    example: spec.example,
                    description: ConfigDescriptionOutput {
                        en: spec.description.en,
                        zh_cn: spec.description.zh_cn,
                    },
                    alias_of: spec.alias_of,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("{}", cli_i18n::text(language).config_keys_about);
    let sections = section.map_or_else(|| CONFIG_SECTION_ORDER.to_vec(), |section| vec![section]);
    for section in sections {
        let specs = CONFIG_KEYS
            .iter()
            .filter(|spec| spec.section == section)
            .collect::<Vec<_>>();
        if specs.is_empty() {
            continue;
        }
        println!();
        println!("{}:", section.label(language));
        for spec in specs {
            let alias = spec
                .alias_of
                .map(|canonical| match language {
                    UiLanguage::En => format!("; alias of {canonical}"),
                    UiLanguage::ZhCn => format!("；{canonical} 的别名"),
                })
                .unwrap_or_default();
            let (description_label, example_label) = match language {
                UiLanguage::En => ("description", "example"),
                UiLanguage::ZhCn => ("说明", "示例"),
            };
            println!("  {} [{}]{}", spec.key, spec.kind.as_str(), alias);
            println!(
                "    ├─ {description_label}: {}",
                localized_description(spec, language)
            );
            if let Some(choices) = spec.kind.choices() {
                let choices_label = match language {
                    UiLanguage::En => "choices",
                    UiLanguage::ZhCn => "可选值",
                };
                println!("    ├─ {example_label}: {}", spec.example);
                println!("    └─ {choices_label}: {}", choices.join(" | "));
            } else {
                println!("    └─ {example_label}: {}", spec.example);
            }
        }
    }
    Ok(())
}

fn localized_description(spec: &ConfigKeySpec, language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => spec.description.en,
        UiLanguage::ZhCn => spec.description.zh_cn,
    }
}

fn set_mode(config: &mut Config, value: &str) -> Result<()> {
    config.mode = match value.to_ascii_lowercase().as_str() {
        "standalone" => RuntimeMode::Standalone,
        "hub" => RuntimeMode::Hub,
        "local" => RuntimeMode::Local,
        _ => return Err(anyhow!("mode must be standalone, hub, or local")),
    };
    Ok(())
}

fn set_profile(config: &mut Config, value: &str) -> Result<()> {
    config.profile = match value.to_ascii_lowercase().as_str() {
        "normal" => WorkerProfile::Normal,
        "room" => WorkerProfile::Room,
        _ => return Err(anyhow!("profile must be normal or room")),
    };
    Ok(())
}

fn set_agent_id(config: &mut Config, value: &str) -> Result<()> {
    config.agent_id = value.to_string();
    Ok(())
}

fn set_display_name(config: &mut Config, value: &str) -> Result<()> {
    config.display_name = value.to_string();
    Ok(())
}

fn set_agent_secret(config: &mut Config, value: &str) -> Result<()> {
    config.hub.agent_secret = value.to_string();
    Ok(())
}
fn set_http_mcp_enabled(config: &mut Config, value: &str) -> Result<()> {
    config.http_mcp.enabled = value.parse::<bool>()?;
    Ok(())
}

fn set_http_mcp_host(config: &mut Config, value: &str) -> Result<()> {
    config.http_mcp.host = value.to_string();
    Ok(())
}

fn set_http_mcp_port(config: &mut Config, value: &str) -> Result<()> {
    let port = value.parse::<u16>()?;
    if port == 0 {
        return Err(anyhow!("httpMcp.port must be between 1 and 65535"));
    }
    config.http_mcp.port = port;
    Ok(())
}

fn set_http_mcp_bearer_token(config: &mut Config, value: &str) -> Result<()> {
    if !value.is_empty() {
        config::validate_http_mcp_bearer_token(value)?;
    }
    config.http_mcp.bearer_token = value.to_string();
    Ok(())
}

fn set_http_mcp_public_url(config: &mut Config, value: &str) -> Result<()> {
    config.http_mcp.public_url = if value == "null" {
        None
    } else {
        Some(config::normalize_http_mcp_public_url(value)?)
    };
    Ok(())
}

fn set_http_mcp_allow_hosts(config: &mut Config, value: &str) -> Result<()> {
    config.http_mcp.allow_hosts = config::parse_http_mcp_allow_hosts(value)?;
    Ok(())
}

fn set_workspace_root(config: &mut Config, value: &str) -> Result<()> {
    let old_workspace = config.workspace_root.clone();
    let new_workspace = PathBuf::from(value);
    for root in &mut config.path_policy.write_roots {
        if policy::paths_match(root, &old_workspace) {
            *root = new_workspace.clone();
        }
    }
    config.workspace_root = new_workspace;
    Ok(())
}

fn set_hub_url(config: &mut Config, value: &str) -> Result<()> {
    config.hub.url = value.to_string();
    Ok(())
}

fn set_hub_transport(config: &mut Config, value: &str) -> Result<()> {
    let normalized = value.to_lowercase();
    if normalized != "websocket" && normalized != "sse" {
        return Err(anyhow!("hub.transport must be websocket or sse"));
    }
    config.hub.transport = normalized;
    Ok(())
}

fn set_confirmation_channels(config: &mut Config, value: &str) -> Result<()> {
    let names = serde_json::from_str::<Vec<String>>(value)
        .map_err(|_| anyhow!("confirmationProvider.channels must be a JSON string array"))?;
    config.confirmation_provider =
        config::ConfirmationProviderConfig::from_channel_names(names.iter().map(String::as_str))
            .map_err(|error| anyhow!(error))?;
    Ok(())
}

fn set_confirmation_language(config: &mut Config, value: &str) -> Result<()> {
    config.confirmation_language = normalize_confirmation_language(value);
    Ok(())
}

fn set_sandbox_enabled(config: &mut Config, value: &str) -> Result<()> {
    config.sandbox.enabled = value.parse::<bool>()?;
    Ok(())
}

fn set_bubblewrap_path(config: &mut Config, value: &str) -> Result<()> {
    config.sandbox.bubblewrap_path = value.to_string();
    Ok(())
}

fn set_required_runtime_paths(config: &mut Config, value: &str) -> Result<()> {
    config.sandbox.required_runtime_paths = serde_json::from_str(value)?;
    Ok(())
}

fn set_backup_limit(config: &mut Config, value: &str) -> Result<()> {
    config.backup_limit = value.parse::<usize>()?;
    Ok(())
}

fn set_max_concurrent_tasks(config: &mut Config, value: &str) -> Result<()> {
    config.limits.max_concurrent_tasks = value.parse::<usize>()?;
    Ok(())
}

fn set_max_active_processes(config: &mut Config, value: &str) -> Result<()> {
    config.limits.max_active_processes = config::parse_max_active_processes(value)?;
    Ok(())
}

fn set_max_file_search_context_lines(config: &mut Config, value: &str) -> Result<()> {
    let parsed = value.parse::<usize>()?;
    config::validate_max_file_search_context_lines(parsed)?;
    config.limits.max_file_search_context_lines = parsed;
    Ok(())
}

fn set_process_response_bytes(config: &mut Config, value: &str) -> Result<()> {
    let parsed = value.parse::<usize>()?;
    config::validate_process_response_bytes(parsed)?;
    config.limits.process_response_bytes = parsed;
    Ok(())
}

fn set_skills_max_files(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_files = value.parse::<usize>()?;
    Ok(())
}

fn set_skills_max_file_bytes(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_file_bytes = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_max_package_bytes(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_package_bytes = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_max_skill_md_bytes(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_skill_md_bytes = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_max_inline_bytes(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_inline_bytes = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_connect_timeout_secs(config: &mut Config, value: &str) -> Result<()> {
    config.skills.connect_timeout_secs = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_request_timeout_secs(config: &mut Config, value: &str) -> Result<()> {
    config.skills.request_timeout_secs = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_idle_timeout_secs(config: &mut Config, value: &str) -> Result<()> {
    config.skills.idle_timeout_secs = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_max_redirects(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_redirects = value.parse::<usize>()?;
    Ok(())
}

fn set_skills_max_concurrent_installs(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_concurrent_installs = value.parse::<usize>()?;
    Ok(())
}

fn set_skills_max_parallel_downloads(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_parallel_downloads = value.parse::<usize>()?;
    Ok(())
}

fn set_skills_max_attempts(config: &mut Config, value: &str) -> Result<()> {
    config.skills.max_attempts = value.parse::<u32>()?;
    Ok(())
}

fn set_skills_total_deadline_secs(config: &mut Config, value: &str) -> Result<()> {
    config.skills.total_deadline_secs = value.parse::<u64>()?;
    Ok(())
}

fn set_skills_allowed_hosts(config: &mut Config, value: &str) -> Result<()> {
    config.skills.allowed_hosts = serde_json::from_str(value)?;
    Ok(())
}

fn set_repository_root(config: &mut Config, value: &str) -> Result<()> {
    config.room.repository_root = if value == "null" {
        None
    } else {
        Some(PathBuf::from(value))
    };
    Ok(())
}

fn set_room_maintenance_mode(config: &mut Config, value: &str) -> Result<()> {
    config.room.maintenance.mode = match value.to_ascii_lowercase().as_str() {
        "local" => RoomMaintenanceMode::Local,
        "workflow" => RoomMaintenanceMode::Workflow,
        _ => return Err(anyhow!("room.maintenance.mode must be local or workflow")),
    };
    Ok(())
}

fn set_room_maintenance_auto_push(config: &mut Config, value: &str) -> Result<()> {
    config.room.maintenance.auto_push = value.parse::<bool>()?;
    Ok(())
}

fn set_room_timezone(config: &mut Config, value: &str) -> Result<()> {
    config.room.timezone = value.to_string();
    Ok(())
}

fn set_diary_day_boundary_hour(config: &mut Config, value: &str) -> Result<()> {
    let hour = value.parse::<u32>()?;
    if hour > 23 {
        return Err(anyhow!(
            "room.diaryDayBoundaryHour must be an integer from 0 to 23"
        ));
    }
    config.room.diary_day_boundary_hour = hour;
    Ok(())
}

fn set_tunnel_id(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).tunnel_id = value.to_string();
    Ok(())
}

fn set_tunnel_api_key(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).api_key = value.to_string();
    Ok(())
}

fn set_tunnel_client_version(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).client.version = if value == "null" {
        None
    } else {
        Some(value.to_string())
    };
    Ok(())
}

fn set_tunnel_client_cache_dir(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).client.cache_dir = PathBuf::from(value);
    Ok(())
}

fn set_tunnel_client_auto_download(config: &mut Config, value: &str) -> Result<()> {
    let parsed = value.parse::<bool>()?;
    tunnel_config(config).client.auto_download = parsed;
    Ok(())
}

fn set_tunnel_client_executable(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).client.executable = if value == "null" {
        None
    } else {
        Some(PathBuf::from(value))
    };
    Ok(())
}

fn set_tunnel_client_download_url(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).client.download_url = if value == "null" {
        None
    } else {
        Some(value.to_string())
    };
    Ok(())
}

fn set_tunnel_client_sha256(config: &mut Config, value: &str) -> Result<()> {
    tunnel_config(config).client.sha256 = if value == "null" {
        None
    } else {
        Some(value.to_string())
    };
    Ok(())
}

fn set_tunnel_hub_reporting_enabled(config: &mut Config, value: &str) -> Result<()> {
    let parsed = value.parse::<bool>()?;
    tunnel_config(config).hub_reporting.enabled = parsed;
    Ok(())
}

fn set_tunnel_hub_reporting_detail(config: &mut Config, value: &str) -> Result<()> {
    let detail = match value {
        "metadata" => ReportingDetail::Metadata,
        "full" => ReportingDetail::Full,
        _ => {
            return Err(anyhow!(
                "tunnel hub reporting detail must be metadata or full"
            ))
        }
    };
    tunnel_config(config).hub_reporting.detail = detail;
    Ok(())
}
fn set_events_low_ttl_seconds(config: &mut Config, value: &str) -> Result<()> {
    let low_ttl_seconds = value
        .parse::<u64>()
        .map_err(|_| anyhow!("events.lowTtlSeconds must be a non-negative integer"))?;
    let mut events = config.events.clone();
    events.low_ttl_seconds = low_ttl_seconds;
    events.validate()?;
    config.events = events;
    Ok(())
}

fn set_internal_override(config: &mut Config, event_type: &str, value: &str) -> Result<()> {
    if !INTERNAL_EVENT_TYPES.contains(&event_type) {
        return Err(anyhow!("unknown internal event type: {event_type}"));
    }
    let mut events = config.events.clone();
    if value == "null" {
        events.internal_overrides.remove(event_type);
    } else {
        let level = match value {
            "low" => EventNotificationLevel::Low,
            "medium" => EventNotificationLevel::Medium,
            "high" => EventNotificationLevel::High,
            "off" => EventNotificationLevel::Off,
            _ => {
                return Err(anyhow!(
                    "events.internalOverrides value must be low, medium, high, off, or null"
                ))
            }
        };
        events
            .internal_overrides
            .insert(event_type.to_string(), level);
    }
    events.validate()?;
    config.events = events;
    Ok(())
}

macro_rules! internal_override_setter {
    ($name:ident, $event_type:literal) => {
        fn $name(config: &mut Config, value: &str) -> Result<()> {
            set_internal_override(config, $event_type, value)
        }
    };
}

internal_override_setter!(set_process_completed_level, "process.completed");
internal_override_setter!(set_process_failed_level, "process.failed");
internal_override_setter!(set_process_rejected_level, "process.rejected");
internal_override_setter!(set_process_cancelled_level, "process.cancelled");
internal_override_setter!(set_process_timed_out_level, "process.timed_out");
internal_override_setter!(set_process_detached_level, "process.detached");
internal_override_setter!(
    set_process_unknown_after_restart_level,
    "process.unknown_after_restart"
);
internal_override_setter!(set_process_skipped_level, "process.skipped");
internal_override_setter!(set_skill_install_completed_level, "skill_install.completed");
internal_override_setter!(set_skill_install_failed_level, "skill_install.failed");
internal_override_setter!(set_skill_install_cancelled_level, "skill_install.cancelled");

fn tunnel_config(config: &mut Config) -> &mut config::TunnelConfig {
    config
        .tunnel
        .get_or_insert_with(config::TunnelConfig::default)
}
