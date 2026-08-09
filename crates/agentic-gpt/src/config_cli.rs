use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::{
    cli_i18n::{self, UiLanguage},
    config::{
        self, normalize_confirmation_language, write_config_with_backup, Config, ReportingDetail,
    },
    config_setup::SetupSeed,
    config_templates::{self, InitInput, InitSummary, RuntimeMode, SecretValue},
    mcp::{self, McpConfigCommand},
    policy::{self, PolicyDecision},
    WorkerProfile,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigValueKind {
    String,
    Path,
    Boolean,
    NonNegativeInteger,
    AutoOrNonNegativeInteger,
    JsonStringArray,
    JsonPathArray,
    NullableString,
    NullablePath,
    ConfirmationProvider,
    Language,
    HubTransport,
    ReportingDetail,
}

impl ConfigValueKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Path => "path",
            Self::Boolean => "boolean",
            Self::NonNegativeInteger => "non-negative-integer",
            Self::AutoOrNonNegativeInteger => "auto-or-non-negative-integer",
            Self::JsonStringArray => "json-string-array",
            Self::JsonPathArray => "json-path-array",
            Self::NullableString => "nullable-string",
            Self::NullablePath => "nullable-path",
            Self::ConfirmationProvider => "confirmation-provider",
            Self::Language => "language",
            Self::HubTransport => "hub-transport",
            Self::ReportingDetail => "reporting-detail",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum ConfigSection {
    Identity,
    Hub,
    Confirmation,
    Sandbox,
    Limits,
    Skills,
    Room,
    Tunnel,
}

impl ConfigSection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Hub => "hub",
            Self::Confirmation => "confirmation",
            Self::Sandbox => "sandbox",
            Self::Limits => "limits",
            Self::Skills => "skills",
            Self::Room => "room",
            Self::Tunnel => "tunnel",
        }
    }

    fn label(self, language: UiLanguage) -> &'static str {
        match (self, language) {
            (Self::Identity, UiLanguage::En) => "Identity",
            (Self::Identity, UiLanguage::ZhCn) => "身份",
            (Self::Hub, UiLanguage::En) => "Hub",
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
        "agentSecret",
        Identity,
        String,
        false,
        "Agent authentication secret or reference.",
        "代理认证密钥或密钥引用。",
        "env:AGENT_SECRET",
        set_agent_secret
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
        "hubUrl",
        Hub,
        String,
        false,
        "Hub URL used for agent communication.",
        "代理通信使用的 Hub URL。",
        "http://localhost:8787",
        set_hub_url
    ),
    config_key!(
        "workerUrl",
        Hub,
        String,
        false,
        "Legacy alias for hubUrl.",
        "hubUrl 的旧别名。",
        "http://localhost:8787",
        set_hub_url,
        "hubUrl"
    ),
    config_key!(
        "hubTransport",
        Hub,
        HubTransport,
        false,
        "Hub transport; websocket or sse.",
        "Hub 传输方式：websocket 或 sse。",
        "websocket",
        set_hub_transport
    ),
    config_key!(
        "confirmationProvider",
        Confirmation,
        ConfirmationProvider,
        false,
        "Confirmation channels: none, freedesktop, ntfy, or default.",
        "确认通道：none、freedesktop、ntfy 或 default。",
        "default",
        set_confirmation_provider
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
        "Maximum concurrently running child Jobs within one process.batch call.",
        "单次 process.batch 中同时运行的子 Job 最大数量。",
        "4",
        set_max_concurrent_tasks
    ),
    config_key!(
        "limits.maxActiveJobs",
        Limits,
        AutoOrNonNegativeInteger,
        false,
        "Total active Job capacity: auto or a non-negative integer.",
        "活动 Job 总容量：auto 或非负整数。",
        "auto",
        set_max_active_jobs
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
        "room.notebookRoot",
        Room,
        NullablePath,
        true,
        "Notebook root path, or null to use the default.",
        "笔记本根目录；使用 null 可恢复默认值。",
        "null",
        set_notebook_root
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
];

pub(crate) fn apply_config_key(config: &mut Config, key: &str, value: &str) -> Result<()> {
    let spec = CONFIG_KEYS
        .iter()
        .find(|spec| spec.key == key)
        .ok_or_else(|| anyhow!("unsupported config key: {key}"))?;
    (spec.apply)(config, value)
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

const CONFIG_SECTION_ORDER: [ConfigSection; 8] = [
    ConfigSection::Identity,
    ConfigSection::Hub,
    ConfigSection::Confirmation,
    ConfigSection::Sandbox,
    ConfigSection::Limits,
    ConfigSection::Skills,
    ConfigSection::Room,
    ConfigSection::Tunnel,
];

fn print_config_keys(
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
            let example_label = match language {
                UiLanguage::En => "example",
                UiLanguage::ZhCn => "示例",
            };
            println!(
                "  {} [{}]{} — {} ({}: {})",
                spec.key,
                spec.kind.as_str(),
                alias,
                localized_description(spec, language),
                example_label,
                spec.example
            );
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

fn set_agent_id(config: &mut Config, value: &str) -> Result<()> {
    config.agent_id = value.to_string();
    Ok(())
}

fn set_display_name(config: &mut Config, value: &str) -> Result<()> {
    config.display_name = value.to_string();
    Ok(())
}

fn set_agent_secret(config: &mut Config, value: &str) -> Result<()> {
    config.agent_secret = value.to_string();
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
    config.hub_url = value.to_string();
    Ok(())
}

fn set_hub_transport(config: &mut Config, value: &str) -> Result<()> {
    let normalized = value.to_lowercase();
    if normalized != "websocket" && normalized != "sse" {
        return Err(anyhow!("hubTransport must be websocket or sse"));
    }
    config.hub_transport = normalized;
    Ok(())
}

fn set_confirmation_provider(config: &mut Config, value: &str) -> Result<()> {
    config
        .confirmation_provider
        .set_legacy(value)
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

fn set_max_active_jobs(config: &mut Config, value: &str) -> Result<()> {
    config.limits.max_active_jobs = config::parse_max_active_jobs(value)?;
    Ok(())
}

fn set_max_file_search_context_lines(config: &mut Config, value: &str) -> Result<()> {
    let parsed = value.parse::<usize>()?;
    config::validate_max_file_search_context_lines(parsed)?;
    config.limits.max_file_search_context_lines = parsed;
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

fn set_notebook_root(config: &mut Config, value: &str) -> Result<()> {
    config.room.notebook_root = if value == "null" {
        None
    } else {
        Some(PathBuf::from(value))
    };
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

#[derive(clap::Args, Clone, Default)]
pub(crate) struct ConfigInitArgs {
    #[arg(long, value_enum)]
    pub(crate) mode: Option<RuntimeMode>,
    #[arg(long, value_enum)]
    pub(crate) profile: Option<WorkerProfile>,
    #[arg(long)]
    pub(crate) non_interactive: bool,
    #[arg(long)]
    pub(crate) tunnel_id: Option<String>,
    #[arg(long)]
    pub(crate) tunnel_api_key: Option<String>,
    #[arg(long)]
    pub(crate) hub_url: Option<String>,
    #[arg(long, value_parser = ["websocket", "sse"])]
    pub(crate) hub_transport: Option<String>,
    #[arg(long)]
    pub(crate) agent_id: Option<String>,
    #[arg(
        long,
        help = "Agent secret (visible to local process inspection and shell history; interactive hidden input is preferred)"
    )]
    pub(crate) agent_secret: Option<String>,
}

pub(crate) fn init_non_interactive(
    config_path: &Path,
    args: &ConfigInitArgs,
    language: UiLanguage,
) -> Result<InitSummary> {
    let mut input = InitInput::non_interactive_defaults(language);
    input.mode = args.mode.unwrap_or(input.mode);
    input.profile = args.profile.unwrap_or(input.profile);
    input.tunnel_id = args.tunnel_id.clone();
    input.tunnel_api_key = args.tunnel_api_key.clone();
    input.hub_url = args.hub_url.clone();
    input.hub_transport = args.hub_transport.clone();
    input.agent_id = args.agent_id.clone();
    input.agent_secret = args
        .agent_secret
        .as_ref()
        .map(|value| SecretValue::new(value.clone()));

    let built = config_templates::build_config(input)?;
    write_config_with_backup(config_path, &built.config)?;
    Ok(InitSummary {
        mode: built.mode,
        profile: built.profile,
        config_path: config_path.to_path_buf(),
        pending: built.pending,
    })
}

pub(crate) fn setup_seed_from_args(args: &ConfigInitArgs) -> SetupSeed {
    SetupSeed {
        mode: args.mode,
        profile: args.profile,
        tunnel_id: args.tunnel_id.clone(),
        tunnel_api_key: args.tunnel_api_key.clone(),
        hub_url: args.hub_url.clone(),
        hub_transport: args.hub_transport.clone(),
        agent_id: args.agent_id.clone(),
        agent_secret: args
            .agent_secret
            .as_ref()
            .map(|value| SecretValue::new(value.clone())),
    }
}

pub(crate) fn should_use_interactive_init(
    non_interactive: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    !non_interactive && stdin_is_terminal && stdout_is_terminal && stderr_is_terminal
}

fn process_should_use_interactive_init(non_interactive: bool) -> bool {
    should_use_interactive_init(
        non_interactive,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

fn interactive_init_required_message(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => {
            "Interactive config init requires a TTY; re-run with --non-interactive for piped or scripted use."
        }
        UiLanguage::ZhCn => "交互式配置初始化需要 TTY；管道或脚本场景请使用 --non-interactive。",
    }
}

fn handle_init(config_path: &Path, args: ConfigInitArgs, language: UiLanguage) -> Result<()> {
    let summary = if args.non_interactive {
        init_non_interactive(config_path, &args, language)?
    } else if process_should_use_interactive_init(args.non_interactive) {
        crate::config_tui::run_config_tui(config_path, setup_seed_from_args(&args), language)
            .map_err(|error| {
                if error.to_string() == "config_init_cancelled" {
                    anyhow!(cli_i18n::text(language).cancelled)
                } else {
                    error
                }
            })?
    } else {
        return Err(anyhow!(interactive_init_required_message(language)));
    };
    let _ = (summary.mode, summary.profile);
    println!(
        "{} {}",
        cli_i18n::text(language).initialized,
        summary.config_path.display()
    );
    for action in summary.pending {
        eprintln!("{}", cli_i18n::pending_action_text(action, language));
    }
    Ok(())
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    Init(ConfigInitArgs),
    Show,
    Keys {
        #[arg(long, value_enum)]
        section: Option<ConfigSection>,
        #[arg(long)]
        json: bool,
    },
    Set {
        key: String,
        value: String,
    },
    Allow {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Confirm {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Deny {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Path {
        #[command(subcommand)]
        command: PathCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpConfigCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum RuleCommand {
    Add {
        program: String,
        args_prefix: Vec<String>,
    },
    Remove {
        program: String,
        args_prefix: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum PathCommand {
    List,
    Write {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Readonly {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Deny {
        #[command(subcommand)]
        command: PathRootCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum PathRootCommand {
    Add { path: PathBuf },
    Remove { path: PathBuf },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PathRootKind {
    Write,
    Readonly,
    Deny,
}

pub(crate) async fn handle_config(
    config_path: PathBuf,
    command: ConfigCommand,
    language: UiLanguage,
) -> Result<()> {
    match command {
        ConfigCommand::Init(args) => handle_init(&config_path, args, language)?,
        ConfigCommand::Show => {
            let config = Config::load(&config_path)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigCommand::Keys { section, json } => {
            print_config_keys(section, json, language)?;
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load_or_default(&config_path)?;
            apply_config_key(&mut config, &key, &value)?;
            write_config_with_backup(&config_path, &config)?;
        }
        ConfigCommand::Allow { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Allow, command)?
        }
        ConfigCommand::Confirm { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Confirm, command)?
        }
        ConfigCommand::Deny { command } => {
            policy::mutate_rule(config_path, PolicyDecision::Deny, command)?
        }
        ConfigCommand::Path { command } => policy::mutate_path_policy(config_path, command)?,
        ConfigCommand::Mcp { command } => mcp::mutate_servers(config_path, command)?,
    }
    Ok(())
}

fn tunnel_config(config: &mut Config) -> &mut config::TunnelConfig {
    config
        .tunnel
        .get_or_insert_with(config::TunnelConfig::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_init_requires_all_three_terminals_and_no_non_interactive_flag() {
        let cases = [
            (true, true, true, true, false),
            (false, false, true, true, false),
            (false, true, false, true, false),
            (false, true, true, false, false),
            (false, false, false, false, false),
            (false, true, true, true, true),
        ];

        for (non_interactive, stdin, stdout, stderr, expected) in cases {
            assert_eq!(
                should_use_interactive_init(non_interactive, stdin, stdout, stderr),
                expected,
                "unexpected interactive-init decision"
            );
        }
    }

    #[test]
    fn registry_applies_new_scalar_and_list_keys() {
        let mut config = Config::default_config().unwrap();
        apply_config_key(&mut config, "displayName", "Desk Agent").unwrap();
        apply_config_key(&mut config, "backupLimit", "7").unwrap();
        apply_config_key(
            &mut config,
            "sandbox.requiredRuntimePaths",
            r#"["/usr","/opt/runtime"]"#,
        )
        .unwrap();
        apply_config_key(&mut config, "limits.maxConcurrentTasks", "4").unwrap();
        apply_config_key(&mut config, "limits.maxActiveJobs", "auto").unwrap();
        apply_config_key(&mut config, "limits.maxFileSearchContextLines", "12").unwrap();

        assert_eq!(config.display_name, "Desk Agent");
        assert_eq!(config.backup_limit, 7);
        assert_eq!(config.sandbox.required_runtime_paths.len(), 2);
        assert_eq!(config.limits.max_concurrent_tasks, 4);
        assert_eq!(config.limits.max_file_search_context_lines, 12);
    }

    #[test]
    fn registry_clears_nullable_notebook_root() {
        let mut config = Config::default_config().unwrap();
        apply_config_key(&mut config, "room.notebookRoot", "/tmp/notebook").unwrap();
        apply_config_key(&mut config, "room.notebookRoot", "null").unwrap();
        assert!(config.room.notebook_root.is_none());
    }

    #[test]
    fn registry_keys_are_unique_and_have_bilingual_metadata() {
        let mut seen = std::collections::BTreeSet::new();
        for spec in CONFIG_KEYS {
            assert!(seen.insert(spec.key), "duplicate key: {}", spec.key);
            assert!(!spec.description.en.is_empty());
            assert!(!spec.description.zh_cn.is_empty());
            assert!(!spec.example.is_empty());
        }
    }

    #[test]
    fn setup_seed_conversion_preserves_editable_flags_and_redacts_agent_secret() {
        let marker = "setup-seed-secret-marker";
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Room),
            tunnel_id: Some("seed-tunnel".to_string()),
            tunnel_api_key: Some("env:TUNNEL_KEY".to_string()),
            hub_url: Some("https://hub.example.com".to_string()),
            hub_transport: Some("sse".to_string()),
            agent_id: Some("desk".to_string()),
            agent_secret: Some(marker.to_string()),
            ..ConfigInitArgs::default()
        };

        let seed = setup_seed_from_args(&args);
        assert_eq!(seed.mode, args.mode);
        assert_eq!(seed.profile, args.profile);
        assert_eq!(seed.tunnel_id.as_deref(), Some("seed-tunnel"));
        assert_eq!(seed.tunnel_api_key.as_deref(), Some("env:TUNNEL_KEY"));
        assert_eq!(seed.hub_url.as_deref(), Some("https://hub.example.com"));
        assert_eq!(seed.hub_transport.as_deref(), Some("sse"));
        assert_eq!(seed.agent_id.as_deref(), Some("desk"));
        assert_eq!(seed.agent_secret.as_ref().unwrap().expose(), marker);
        assert!(!format!("{seed:?}").contains(marker));
    }
}
