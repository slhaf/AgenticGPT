use anyhow::Result;

use crate::cli_i18n::{self, UiLanguage};

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
    PolicyAction,
    SecretPolicy,
    EmailAccount,
    PathDefaultAction,
}

impl ConfigValueKind {
    pub(crate) fn as_str(self) -> &'static str {
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
            Self::PolicyAction => "policy-action",
            Self::SecretPolicy => "secret-policy",
            Self::EmailAccount => "email-account-object-or-null",
            Self::PathDefaultAction => "path-default-action",
            Self::EventNotificationLevel => "event-notification-level",
        }
    }

    pub(crate) fn choices(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Boolean => Some(&["true", "false"]),
            Self::PolicyAction => Some(&["allow", "confirm", "deny"]),
            Self::SecretPolicy => Some(&["allow", "confirm"]),
            Self::PathDefaultAction => Some(&["allow", "deny"]),
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
    Policy,
    Shell,
    Limits,
    Skills,
    Room,
    Email,
    Tunnel,
    Events,
    Workspace,
    Toolsets,
    Config,
    Browser,
    Mcp,
}

impl ConfigSection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Toolsets => "toolsets",
            Self::Config => "config",
            Self::Browser => "browser",
            Self::Mcp => "mcp",
            Self::Runtime => "runtime",
            Self::HttpMcp => "http-mcp",
            Self::Identity => "identity",
            Self::Hub => "hub",
            Self::Confirmation => "confirmation",
            Self::Policy => "policy",
            Self::Shell => "shell",
            Self::Limits => "limits",
            Self::Skills => "skills",
            Self::Room => "room",
            Self::Email => "email",
            Self::Tunnel => "tunnel",
            Self::Events => "events",
        }
    }

    pub(crate) fn label(self, language: UiLanguage) -> &'static str {
        match (self, language) {
            (Self::Workspace, UiLanguage::En) => "Workspace",
            (Self::Workspace, UiLanguage::ZhCn) => "工作区",
            (Self::Toolsets, UiLanguage::En) => "Toolsets",
            (Self::Toolsets, UiLanguage::ZhCn) => "工具集",
            (Self::Config, UiLanguage::En) => "Configuration API",
            (Self::Config, UiLanguage::ZhCn) => "配置 API",
            (Self::Browser, UiLanguage::En) => "Browser",
            (Self::Browser, UiLanguage::ZhCn) => "浏览器",
            (Self::Mcp, UiLanguage::En) => "MCP servers",
            (Self::Mcp, UiLanguage::ZhCn) => "MCP 服务",
            (Self::Runtime, UiLanguage::En) => "Runtime",
            (Self::Runtime, UiLanguage::ZhCn) => "运行时",
            (Self::Identity, UiLanguage::En) => "Identity",
            (Self::Identity, UiLanguage::ZhCn) => "身份",
            (Self::Hub, UiLanguage::En) => "Hub",
            (Self::Hub, UiLanguage::ZhCn) => "Hub",
            (Self::HttpMcp, UiLanguage::En) => "HTTP MCP",
            (Self::HttpMcp, UiLanguage::ZhCn) => "HTTP MCP",
            (Self::Confirmation, UiLanguage::En) => "Confirmation",
            (Self::Confirmation, UiLanguage::ZhCn) => "确认",
            (Self::Policy, UiLanguage::En) => "Policy",
            (Self::Policy, UiLanguage::ZhCn) => "策略",
            (Self::Shell, UiLanguage::En) => "Shell",
            (Self::Shell, UiLanguage::ZhCn) => "Shell",
            (Self::Limits, UiLanguage::En) => "Limits",
            (Self::Limits, UiLanguage::ZhCn) => "限制",
            (Self::Skills, UiLanguage::En) => "Skills",
            (Self::Skills, UiLanguage::ZhCn) => "技能",
            (Self::Email, UiLanguage::En) => "Email",
            (Self::Email, UiLanguage::ZhCn) => "邮件",
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
    pub(crate) kind: ConfigValueKind,
    pub(crate) nullable: bool,
    pub(crate) description: LocalizedText,
    pub(crate) example: &'static str,
    pub(crate) alias_of: Option<&'static str>,
}

macro_rules! config_key {
    ($key:literal, $section:ident, $kind:ident, $nullable:expr, $en:expr, $zh_cn:expr, $example:expr, $apply:ident) => {
        ConfigKeySpec {
            key: $key,
            kind: ConfigValueKind::$kind,
            nullable: $nullable,
            description: LocalizedText {
                en: $en,
                zh_cn: $zh_cn,
            },
            example: $example,
            alias_of: None,
        }
    };
    ($key:literal, $section:ident, $kind:ident, $nullable:expr, $en:expr, $zh_cn:expr, $example:expr, $apply:ident, $reset:ident) => {
        ConfigKeySpec {
            key: $key,
            kind: ConfigValueKind::$kind,
            nullable: $nullable,
            description: LocalizedText {
                en: $en,
                zh_cn: $zh_cn,
            },
            example: $example,
            alias_of: None,
        }
    };
    ($key:literal, $section:ident, $kind:ident, $nullable:expr, $en:expr, $zh_cn:expr, $example:expr, $apply:ident, $alias_of:literal) => {
        ConfigKeySpec {
            key: $key,
            kind: ConfigValueKind::$kind,
            nullable: $nullable,
            description: LocalizedText {
                en: $en,
                zh_cn: $zh_cn,
            },
            example: $example,
            alias_of: Some($alias_of),
        }
    };
}

pub(crate) static CONFIG_KEYS: &[ConfigKeySpec] = &[
    config_key!(
        "room.secret.serverUrl",
        Room,
        String,
        false,
        "Vaultwarden server URL; changes require restart.",
        "Vaultwarden 服务地址；修改后需重启。",
        "https://vault.example.invalid",
        set_room_secret_server_url
    ),
    config_key!(
        "room.secret.account",
        Room,
        String,
        false,
        "Dedicated Vaultwarden account; changes require restart.",
        "Vaultwarden 专用账号；修改后需重启。",
        "agent@example.invalid",
        set_room_secret_account
    ),
    config_key!(
        "room.secret.clientId",
        Room,
        String,
        false,
        "Personal API client ID reference; env:NAME or file:PATH, never plaintext.",
        "Personal API client ID 引用；仅接受 env:NAME 或 file:PATH。",
        "env:BITWARDEN_CLIENT_ID",
        set_room_secret_client_id
    ),
    config_key!(
        "room.secret.clientSecret",
        Room,
        String,
        false,
        "Personal API client secret reference; env:NAME or file:PATH, never plaintext.",
        "Personal API client secret 引用；仅接受 env:NAME 或 file:PATH。",
        "env:BITWARDEN_CLIENT_SECRET",
        set_room_secret_client_secret
    ),
    config_key!(
        "room.secret.masterPassword",
        Room,
        String,
        false,
        "Master password reference; env:NAME or file:PATH, never plaintext.",
        "主密码引用；仅接受 env:NAME 或 file:PATH。",
        "env:BITWARDEN_MASTER_PASSWORD",
        set_room_secret_master_password
    ),
    config_key!(
        "room.secret.getPolicy",
        Room,
        SecretPolicy,
        false,
        "Secret retrieval policy; allow or confirm. Hot reload applies to new requests.",
        "密钥读取策略：allow 或 confirm；热更新后用于新请求。",
        "allow",
        set_room_secret_get_policy
    ),
    config_key!(
        "room.secret.upsertPolicy",
        Room,
        SecretPolicy,
        false,
        "Secret creation/update policy; allow or confirm. Hot reload applies to new requests.",
        "密钥创建与更新策略：allow 或 confirm；热更新后用于新请求。",
        "confirm",
        set_room_secret_upsert_policy
    ),
    config_key!(
        "room.secret.deletePolicy",
        Room,
        SecretPolicy,
        false,
        "Secret deletion policy; allow or confirm. Hot reload applies to new requests.",
        "密钥删除策略：allow 或 confirm；热更新后用于新请求。",
        "confirm",
        set_room_secret_delete_policy
    ),
    config_key!(
        "room.secret.syncWaitTimeoutMs",
        Room,
        NonNegativeInteger,
        false,
        "Query sync wait in milliseconds, 0..30000; hot reload applies to new requests. Zero does not wait.",
        "查询等待同步的毫秒数，0..30000；热更新后用于新请求，0 表示不等待。",
        "3000",
        set_room_secret_sync_wait_timeout_ms
    ),
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
        "hub.enabled",
        Hub,
        Boolean,
        false,
        "Enable the auxiliary Hub connection in standalone or local mode. Required for ntfy confirmations and standalone hub reporting.",
        "在 Standalone 或 Local 模式启用 Hub 辅助连接；`ntfy` 确认和 Standalone Hub 上报都需要此连接。",
        "false",
        set_hub_enabled
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
        "policy.enabled",
        Policy,
        Boolean,
        false,
        "Enable command and path policy enforcement.",
        "启用命令与路径策略执行。",
        "true",
        set_policy_enabled
    ),
    config_key!(
        "policy.command.defaultAction",
        Policy,
        PolicyAction,
        false,
        "Default command policy action.",
        "命令未匹配规则时的默认动作。",
        "confirm",
        set_policy_command_default_action
    ),
    config_key!(
        "policy.command.allow",
        Policy,
        JsonStringArray,
        false,
        "Allowed command patterns.",
        "允许的命令模式数组。",
        r#"[]"#,
        set_policy_command_allow
    ),
    config_key!(
        "policy.command.confirm",
        Policy,
        JsonStringArray,
        false,
        "Command patterns that require confirmation.",
        "需要确认的命令模式数组。",
        r#"[]"#,
        set_policy_command_confirm
    ),
    config_key!(
        "policy.command.deny",
        Policy,
        JsonStringArray,
        false,
        "Denied command patterns.",
        "拒绝的命令模式数组。",
        r#"[]"#,
        set_policy_command_deny
    ),
    config_key!(
        "policy.path.defaultAction",
        Policy,
        PathDefaultAction,
        false,
        "Default path policy action.",
        "路径未匹配根目录时的默认动作。",
        "deny",
        set_policy_path_default_action
    ),
    config_key!(
        "policy.path.readOnly",
        Policy,
        JsonPathArray,
        false,
        "Concrete read-only path roots.",
        "具体的只读路径根目录。",
        r#"["/usr"]"#,
        set_policy_path_read_only
    ),
    config_key!(
        "policy.path.write",
        Policy,
        JsonPathArray,
        false,
        "Concrete writable path roots.",
        "具体的可写路径根目录。",
        r#"["~/.agentic_gpt/workspace"]"#,
        set_policy_path_write
    ),
    config_key!(
        "policy.path.deny",
        Policy,
        JsonPathArray,
        false,
        "Concrete denied path roots.",
        "具体的拒绝访问路径根目录。",
        r#"[]"#,
        set_policy_path_deny
    ),
    config_key!(
        "policy.bubblewrap",
        Policy,
        Boolean,
        false,
        "Use bubblewrap to enforce the configured path roots for process execution.",
        "使用 bubblewrap 为进程执行强制应用配置的路径根目录。",
        "false",
        set_bubblewrap
    ),
    config_key!(
        "shell.initFile",
        Shell,
        NullablePath,
        true,
        "Trusted shell initialization file; null disables it. Omit or unset to use the default.",
        "受信任的 shell 初始化文件；null 表示禁用。省略或 unset 使用默认值。",
        "~/.agentic_gpt/.bashrc",
        set_shell_init_file,
        unset_shell_init_file
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
        "Default serialized Process response budget shared by process.exec, process.batch, skills.run, mcp.callTool, mcp.batch, and process.read. If process.read omits maxBytes, it uses this value; an explicit maxBytes overrides it for that read only. Configured range: 4096–1048576 bytes.",
        "process.exec、process.batch、skills.run、mcp.callTool、mcp.batch 和 process.read 共用的序列化 Process 响应默认预算。process.read 省略 maxBytes 时使用此值；显式 maxBytes 仅覆盖本次 read。配置范围：4096–1048576 字节。",
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
        "email.account",
        Email,
        EmailAccount,
        true,
        "Account connection object or null; changes require restart. Password must be an env:/file: reference.",
        "账户连接 JSON 对象或 null；修改后重启生效。密码必须使用 env:/file: 引用。",
        "null",
        set_email_account
    ),
    config_key!(
        "email.syncIntervalSeconds",
        Email,
        NonNegativeInteger,
        false,
        "Synchronization interval in seconds, from 10 through 86400.",
        "同步间隔秒数，范围 10 到 86400。",
        "60",
        set_email_sync_interval_seconds
    ),
    config_key!(
        "email.retentionDays",
        Email,
        NonNegativeInteger,
        false,
        "Local content retention in days, from 1 through 36500.",
        "本地邮件内容保存天数，范围 1 到 36500。",
        "90",
        set_email_retention_days
    ),
    config_key!(
        "email.maxStorageBytes",
        Email,
        NonNegativeInteger,
        false,
        "Local email storage limit in bytes, at least 1048576.",
        "本地邮件容量上限字节数，至少 1048576。",
        "2147483648",
        set_email_max_storage_bytes
    ),
    config_key!(
        "email.sendPolicy",
        Email,
        PolicyAction,
        false,
        "Email sending policy: allow, confirm, or deny.",
        "发信策略：allow、confirm 或 deny。",
        "confirm",
        set_email_send_policy
    ),
    config_key!(
        "email.trashPolicy",
        Email,
        PolicyAction,
        false,
        "Email trash policy: allow, confirm, or deny.",
        "移入垃圾箱策略：allow、confirm 或 deny。",
        "confirm",
        set_email_trash_policy
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
        "tunnel.hubReporting.enabled",
        Tunnel,
        Boolean,
        false,
        "Enable standalone runtime reporting over an already enabled auxiliary Hub connection. This does not enable or disable the connection.",
        "启用 Standalone 运行上报；需要 Hub 辅助连接已开启。此项不启停辅助连接。",
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

const CONFIG_SECTION_ORDER: [ConfigSection; 18] = [
    ConfigSection::Runtime,
    ConfigSection::Identity,
    ConfigSection::Workspace,
    ConfigSection::Hub,
    ConfigSection::HttpMcp,
    ConfigSection::Confirmation,
    ConfigSection::Config,
    ConfigSection::Toolsets,
    ConfigSection::Policy,
    ConfigSection::Shell,
    ConfigSection::Limits,
    ConfigSection::Skills,
    ConfigSection::Room,
    ConfigSection::Email,
    ConfigSection::Tunnel,
    ConfigSection::Browser,
    ConfigSection::Mcp,
    ConfigSection::Events,
];

pub(super) fn print_config_keys(
    section: Option<ConfigSection>,
    json: bool,
    language: UiLanguage,
) -> Result<()> {
    let entries = crate::config_catalog::entries()?;
    let entries = entries
        .into_iter()
        .filter(|entry| entry.kind != "object" || entry.path == ["browser", "runtime"])
        .filter(|entry| section.is_none_or(|s| s.as_str() == entry.section))
        .collect::<Vec<_>>();
    let records = entries.iter().map(|entry| {
        let key = entry.path.join(".");
        let legacy = CONFIG_KEYS.iter().find(|spec| spec.key == key && spec.alias_of.is_none());
        let example = legacy.map(|spec| spec.example.to_owned()).unwrap_or_else(|| entry.example.to_string());
        let en = legacy.map(|spec| spec.description.en).unwrap_or(&entry.description);
        serde_json::json!({"key":key,"path":entry.path,"section":entry.section,"type":entry.kind,"nullable":entry.nullable,
            "choices":entry.choices,"example":example,"description":{"en":en,"zhCN":entry.description}})
    }).collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"keys":records}))?
        );
        return Ok(());
    }
    println!("{}", cli_i18n::text(language).config_keys_about);
    for section in section.map_or_else(|| CONFIG_SECTION_ORDER.to_vec(), |s| vec![s]) {
        let records = records
            .iter()
            .filter(|record| record["section"] == section.as_str())
            .collect::<Vec<_>>();
        if records.is_empty() {
            continue;
        }
        println!("\n{}:", section.label(language));
        for record in records {
            let description = match language {
                UiLanguage::En => &record["description"]["en"],
                UiLanguage::ZhCn => &record["description"]["zhCN"],
            };
            println!(
                "  {} [{}]\n    {}\n    {}",
                record["key"].as_str().unwrap_or_default(),
                record["type"].as_str().unwrap_or_default(),
                description.as_str().unwrap_or_default(),
                record["example"].as_str().unwrap_or_default()
            );
        }
    }
    Ok(())
}
