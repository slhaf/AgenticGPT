use std::ffi::{OsStr, OsString};
use std::io::{self, Write};

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{CommandFactory, FromArgMatches};

use crate::config_templates::PendingAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum LanguageChoice {
    Auto,
    #[value(name = "zh-CN", alias = "zh", alias = "zh_CN")]
    ZhCn,
    #[value(name = "en", alias = "en-US", alias = "en_US")]
    En,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiLanguage {
    ZhCn,
    En,
}

pub(crate) trait LocaleSource {
    fn get(&self, key: &str) -> Option<OsString>;
}

pub(crate) fn resolve_language(choice: LanguageChoice, env: &impl LocaleSource) -> UiLanguage {
    match choice {
        LanguageChoice::ZhCn => UiLanguage::ZhCn,
        LanguageChoice::En => UiLanguage::En,
        LanguageChoice::Auto => {
            for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                if let Some(locale) = env.get(key) {
                    if locale.is_empty() {
                        continue;
                    }
                    return normalize_locale(&locale);
                }
            }
            UiLanguage::En
        }
    }
}

pub(crate) struct ProcessLocale;

impl LocaleSource for ProcessLocale {
    fn get(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

pub(crate) fn prescan_language(args: &[OsString]) -> Result<LanguageChoice, String> {
    let mut choice = LanguageChoice::Auto;
    let mut index = 0;
    let mut after_double_dash = false;

    while index < args.len() {
        let argument = args[index].to_string_lossy();
        if argument == "--" {
            after_double_dash = true;
            index += 1;
            continue;
        }
        if after_double_dash {
            index += 1;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--language=") {
            choice = parse_language_value(value)?;
        } else if argument == "--language" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--language requires a value".to_string())?;
            let value = value
                .to_str()
                .ok_or_else(|| "--language value must be valid UTF-8".to_string())?;
            choice = parse_language_value(value)?;
            index += 1;
        }
        index += 1;
    }

    Ok(choice)
}

fn parse_language_value(value: &str) -> Result<LanguageChoice, String> {
    match value {
        "auto" => Ok(LanguageChoice::Auto),
        "zh-CN" | "zh" | "zh_CN" => Ok(LanguageChoice::ZhCn),
        "en" | "en-US" | "en_US" => Ok(LanguageChoice::En),
        _ => Err(format!("invalid value '{value}' for --language")),
    }
}

fn normalize_locale(value: &OsStr) -> UiLanguage {
    let Some(value) = value.to_str() else {
        return UiLanguage::En;
    };
    let normalized = value.to_ascii_lowercase().replace('_', "-");
    if normalized.starts_with("zh") {
        UiLanguage::ZhCn
    } else {
        UiLanguage::En
    }
}

#[derive(Clone, Copy)]
struct CatalogEntry {
    path: &'static str,
    en: &'static str,
    zh_cn: &'static str,
}

impl CatalogEntry {
    const fn new(path: &'static str, en: &'static str, zh_cn: &'static str) -> Self {
        Self { path, en, zh_cn }
    }

    fn text(self, language: UiLanguage) -> &'static str {
        match language {
            UiLanguage::ZhCn => self.zh_cn,
            UiLanguage::En => self.en,
        }
    }
}

// Keep this list explicit.  The unit test below walks the built command tree and fails if a
// visible command is added without a description in both locales.
const COMMAND_CATALOG: &[CatalogEntry] = &[
    CatalogEntry::new(
        "",
        "Linux local agent for Agentic GPT",
        "Agentic GPT Linux 本地代理",
    ),
    CatalogEntry::new(
        "run",
        "Run the Agentic GPT hub agent",
        "运行 Agentic GPT Hub 代理",
    ),
    CatalogEntry::new(
        "run-as-room",
        "Run a room-profile hub agent",
        "运行 Room 配置的 Hub 代理",
    ),
    CatalogEntry::new(
        "run-as-standalone",
        "Run the standalone supervised agent",
        "运行受监督的独立代理",
    ),
    CatalogEntry::new(
        "run-as-local",
        "Run the local agent without a Hub",
        "运行不连接 Hub 的本地代理",
    ),
    CatalogEntry::new(
        "local",
        "Manage the local MCP control service",
        "管理本地 MCP 控制服务",
    ),
    CatalogEntry::new(
        "local.list-tools",
        "List tools exposed by the local service",
        "列出本地服务提供的工具",
    ),
    CatalogEntry::new(
        "local.call",
        "Call one local MCP tool",
        "调用一个本地 MCP 工具",
    ),
    CatalogEntry::new(
        "stdio-worker",
        "Run an internal supervised worker",
        "运行内部受监督工作进程",
    ),
    CatalogEntry::new(
        "config",
        "Manage Agentic GPT configuration",
        "管理 Agentic GPT 配置",
    ),
    CatalogEntry::new("config.init", "Initialize configuration", "初始化配置文件"),
    CatalogEntry::new(
        "config.show",
        "Show the current configuration",
        "显示当前配置",
    ),
    CatalogEntry::new("config.keys", "List configuration keys", "列出配置键"),
    CatalogEntry::new("config.set", "Set a configuration value", "设置配置值"),
    CatalogEntry::new(
        "config.allow",
        "Manage commands allowed by policy",
        "管理策略允许的命令",
    ),
    CatalogEntry::new("config.allow.add", "Allow a command", "允许一个命令"),
    CatalogEntry::new(
        "config.allow.remove",
        "Remove an allowed command",
        "移除允许的命令",
    ),
    CatalogEntry::new(
        "config.confirm",
        "Manage commands requiring confirmation",
        "管理需要确认的命令",
    ),
    CatalogEntry::new(
        "config.confirm.add",
        "Require confirmation for a command",
        "要求命令确认",
    ),
    CatalogEntry::new(
        "config.confirm.remove",
        "Remove a confirmation rule",
        "移除确认规则",
    ),
    CatalogEntry::new(
        "config.deny",
        "Manage commands denied by policy",
        "管理策略拒绝的命令",
    ),
    CatalogEntry::new("config.deny.add", "Deny a command", "拒绝一个命令"),
    CatalogEntry::new(
        "config.deny.remove",
        "Remove a denied command",
        "移除拒绝的命令",
    ),
    CatalogEntry::new(
        "config.path",
        "Manage filesystem path policy",
        "管理文件系统路径策略",
    ),
    CatalogEntry::new(
        "config.path.list",
        "List filesystem path policy",
        "列出文件系统路径策略",
    ),
    CatalogEntry::new(
        "config.path.write",
        "Manage writable path roots",
        "管理可写路径根目录",
    ),
    CatalogEntry::new(
        "config.path.write.add",
        "Add a writable path root",
        "添加可写路径根目录",
    ),
    CatalogEntry::new(
        "config.path.write.remove",
        "Remove a writable path root",
        "移除可写路径根目录",
    ),
    CatalogEntry::new(
        "config.path.readonly",
        "Manage read-only path roots",
        "管理只读路径根目录",
    ),
    CatalogEntry::new(
        "config.path.readonly.add",
        "Add a read-only path root",
        "添加只读路径根目录",
    ),
    CatalogEntry::new(
        "config.path.readonly.remove",
        "Remove a read-only path root",
        "移除只读路径根目录",
    ),
    CatalogEntry::new(
        "config.path.deny",
        "Manage denied path roots",
        "管理拒绝访问的路径根目录",
    ),
    CatalogEntry::new(
        "config.path.deny.add",
        "Add a denied path root",
        "添加拒绝访问的路径根目录",
    ),
    CatalogEntry::new(
        "config.path.deny.remove",
        "Remove a denied path root",
        "移除拒绝访问的路径根目录",
    ),
    CatalogEntry::new(
        "config.mcp",
        "Manage configured MCP servers",
        "管理已配置的 MCP 服务器",
    ),
    CatalogEntry::new(
        "config.mcp.list",
        "List configured MCP servers",
        "列出已配置的 MCP 服务器",
    ),
    CatalogEntry::new("config.mcp.add", "Add an MCP server", "添加 MCP 服务器"),
    CatalogEntry::new(
        "config.mcp.remove",
        "Remove an MCP server",
        "移除 MCP 服务器",
    ),
    CatalogEntry::new(
        "config.mcp.enable",
        "Enable an MCP server",
        "启用 MCP 服务器",
    ),
    CatalogEntry::new(
        "config.mcp.disable",
        "Disable an MCP server",
        "禁用 MCP 服务器",
    ),
    CatalogEntry::new(
        "tmux",
        "Manage Agentic GPT tmux sessions",
        "管理 Agentic GPT tmux 会话",
    ),
    CatalogEntry::new("tmux.list", "List tmux sessions", "列出 tmux 会话"),
    CatalogEntry::new(
        "tmux.attach",
        "Attach to a tmux session",
        "连接到 tmux 会话",
    ),
    CatalogEntry::new("tmux.create", "Create a tmux session", "创建 tmux 会话"),
    CatalogEntry::new("tmux.close", "Close a tmux session", "关闭 tmux 会话"),
    CatalogEntry::new(
        "help",
        "Print this message or the help of the given subcommand(s)",
        "显示此消息或指定子命令的帮助",
    ),
];

// Argument IDs are clap's stable internal IDs, not user-facing option tokens.  Keeping them in a
// table lets the runtime tree retain all derive-time parsing behavior while replacing only UI text.
const ARG_CATALOG: &[CatalogEntry] = &[
    CatalogEntry::new(".language", "Select the interface language", "选择界面语言"),
    CatalogEntry::new(".config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new("run.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new("run-as-room.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new(
        "run-as-standalone.config",
        "Configuration file to use",
        "要使用的配置文件",
    ),
    CatalogEntry::new(
        "run-as-standalone.profile",
        "Capability profile for the worker",
        "工作进程的能力配置",
    ),
    CatalogEntry::new("run-as-local.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new(
        "run-as-local.profile",
        "Capability profile for the local agent",
        "本地代理的能力配置",
    ),
    CatalogEntry::new("local.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new("local.call.tool", "Name of the tool to call", "要调用的工具名称"),
    CatalogEntry::new("local.call.arguments", "JSON arguments for the tool", "工具的 JSON 参数"),
    CatalogEntry::new(
        "local.call.arguments_file",
        "Read JSON arguments from a file or stdin",
        "从文件或标准输入读取 JSON 参数",
    ),
    CatalogEntry::new("stdio-worker.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new(
        "stdio-worker.profile",
        "Capability profile for the worker",
        "工作进程的能力配置",
    ),
    CatalogEntry::new(
        "stdio-worker.supervisor_token",
        "Internal supervisor authorization token",
        "内部监督器授权令牌",
    ),
    CatalogEntry::new("config.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new("config.init.mode", "Runtime mode to initialize", "要初始化的运行模式"),
    CatalogEntry::new("config.init.profile", "Capability profile to initialize", "要初始化的能力配置"),
    CatalogEntry::new(
        "config.init.non_interactive",
        "Do not prompt; use supplied values and safe defaults",
        "不提示；使用提供的值和安全默认值",
    ),
    CatalogEntry::new("config.init.tunnel_id", "Tunnel identifier", "隧道标识符"),
    CatalogEntry::new(
        "config.init.tunnel_api_key",
        "Tunnel API key or secret reference",
        "隧道 API 密钥或密钥引用",
    ),
    CatalogEntry::new("config.init.hub_url", "Hub URL", "Hub URL"),
    CatalogEntry::new("config.init.hub_transport", "Hub transport", "Hub 传输方式"),
    CatalogEntry::new("config.init.agent_id", "Agent identifier", "代理标识符"),
    CatalogEntry::new(
        "config.init.agent_secret",
        "Agent secret (visible to local process inspection and shell history; interactive hidden input is preferred)",
        "代理密钥（会暴露给本地进程检查和 shell 历史；建议使用交互式隐藏输入）",
    ),
    CatalogEntry::new("config.keys.section", "Filter keys by section", "按部分筛选配置键"),
    CatalogEntry::new("config.keys.json", "Print machine-readable JSON", "输出机器可读的 JSON"),
    CatalogEntry::new("config.set.key", "Registered configuration key", "已注册的配置键"),
    CatalogEntry::new("config.set.value", "Value to assign", "要设置的值"),
    CatalogEntry::new("config.allow.add.program", "Program name", "程序名称"),
    CatalogEntry::new("config.allow.add.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.allow.remove.program", "Program name", "程序名称"),
    CatalogEntry::new("config.allow.remove.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.confirm.add.program", "Program name", "程序名称"),
    CatalogEntry::new("config.confirm.add.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.confirm.remove.program", "Program name", "程序名称"),
    CatalogEntry::new("config.confirm.remove.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.deny.add.program", "Program name", "程序名称"),
    CatalogEntry::new("config.deny.add.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.deny.remove.program", "Program name", "程序名称"),
    CatalogEntry::new("config.deny.remove.args_prefix", "Optional argument prefix", "可选参数前缀"),
    CatalogEntry::new("config.path.write.add.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.path.write.remove.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.path.readonly.add.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.path.readonly.remove.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.path.deny.add.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.path.deny.remove.path", "Path root", "路径根目录"),
    CatalogEntry::new("config.mcp.add.server_id", "MCP server identifier", "MCP 服务器标识符"),
    CatalogEntry::new("config.mcp.add.url", "MCP server URL", "MCP 服务器 URL"),
    CatalogEntry::new("config.mcp.add.transport", "MCP transport", "MCP 传输方式"),
    CatalogEntry::new("config.mcp.add.enabled", "Whether the server starts enabled", "服务器是否启用"),
    CatalogEntry::new("config.mcp.remove.server_id", "MCP server identifier", "MCP 服务器标识符"),
    CatalogEntry::new("config.mcp.enable.server_id", "MCP server identifier", "MCP 服务器标识符"),
    CatalogEntry::new("config.mcp.disable.server_id", "MCP server identifier", "MCP 服务器标识符"),
    CatalogEntry::new("tmux.config", "Configuration file to use", "要使用的配置文件"),
    CatalogEntry::new("tmux.attach.session", "Session name", "会话名称"),
    CatalogEntry::new("tmux.create.name", "Session name", "会话名称"),
    CatalogEntry::new("tmux.create.cwd", "Working directory", "工作目录"),
    CatalogEntry::new("tmux.close.name", "Session name", "会话名称"),
    CatalogEntry::new("help.subcommand", "Print help for the subcommand(s)", "显示指定子命令的帮助"),
];

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CliText {
    pub(crate) app_about: &'static str,
    pub(crate) config_about: &'static str,
    pub(crate) config_init_about: &'static str,
    pub(crate) config_keys_about: &'static str,
    pub(crate) config_set_about: &'static str,
    pub(crate) usage_heading: &'static str,
    pub(crate) commands_heading: &'static str,
    pub(crate) options_heading: &'static str,
    pub(crate) arguments_heading: &'static str,
    pub(crate) help_flag: &'static str,
    pub(crate) version_flag: &'static str,
    pub(crate) cancelled: &'static str,
    pub(crate) initialized: &'static str,
    pub(crate) replace_tunnel_id: &'static str,
    pub(crate) provision_tunnel_secret: &'static str,
    pub(crate) configure_hub_url: &'static str,
    pub(crate) replace_agent_secret: &'static str,
    pub(crate) optional_settings_prompt: &'static str,
    pub(crate) safe_defaults_option: &'static str,
    pub(crate) choose_sections_option: &'static str,
}

pub(crate) static ZH_CN_TEXT: CliText = CliText {
    app_about: "Agentic GPT Linux 本地代理",
    config_about: "管理 Agentic GPT 配置",
    config_init_about: "初始化配置文件",
    config_keys_about: "列出配置键",
    config_set_about: "设置配置值",
    usage_heading: "用法：",
    commands_heading: "命令：",
    options_heading: "选项：",
    arguments_heading: "参数：",
    help_flag: "显示帮助信息",
    version_flag: "显示版本信息",
    cancelled: "配置已取消。",
    initialized: "配置已初始化:",
    replace_tunnel_id: "下一步：替换 tunnel ID。",
    provision_tunnel_secret: "下一步：配置 tunnel API 密钥引用。",
    configure_hub_url: "下一步：配置 Hub URL。",
    replace_agent_secret: "下一步：替换代理密钥。",
    optional_settings_prompt: "是否配置可选设置？",
    safe_defaults_option: "使用安全默认值",
    choose_sections_option: "选择要配置的部分",
};

pub(crate) static EN_TEXT: CliText = CliText {
    app_about: "Linux local agent for Agentic GPT",
    config_about: "Manage Agentic GPT configuration",
    config_init_about: "Initialize the configuration file",
    config_keys_about: "List configuration keys",
    config_set_about: "Set a configuration value",
    usage_heading: "Usage:",
    commands_heading: "Commands:",
    options_heading: "Options:",
    arguments_heading: "Arguments:",
    help_flag: "Print help information",
    version_flag: "Print version information",
    cancelled: "Configuration cancelled.",
    initialized: "Configuration initialized:",
    replace_tunnel_id: "Next step: replace tunnel ID.",
    provision_tunnel_secret: "Next step: provision the tunnel API secret reference.",
    configure_hub_url: "Next step: configure the Hub URL.",
    replace_agent_secret: "Next step: replace the agent secret.",
    optional_settings_prompt: "Configure optional settings?",
    safe_defaults_option: "Use safe defaults",
    choose_sections_option: "Choose sections to configure",
};

pub(crate) fn text(language: UiLanguage) -> &'static CliText {
    match language {
        UiLanguage::ZhCn => &ZH_CN_TEXT,
        UiLanguage::En => &EN_TEXT,
    }
}

pub(crate) fn pending_action_text(action: PendingAction, language: UiLanguage) -> &'static str {
    let catalog = text(language);
    match action {
        PendingAction::ReplaceTunnelId => catalog.replace_tunnel_id,
        PendingAction::ProvisionTunnelSecret => catalog.provision_tunnel_secret,
        PendingAction::ConfigureHubUrl => catalog.configure_hub_url,
        PendingAction::ReplaceAgentSecret => catalog.replace_agent_secret,
    }
}

const EN_TEMPLATE_NONE: &str = "{about-with-newline}Usage: {usage}\n";
const EN_TEMPLATE_C: &str = "{about-with-newline}Usage: {usage}\n\nCommands:\n{subcommands}\n";
const EN_TEMPLATE_P: &str = "{about-with-newline}Usage: {usage}\n\nArguments:\n{positionals}\n";
const EN_TEMPLATE_O: &str = "{about-with-newline}Usage: {usage}\n\nOptions:\n{options}\n";
const EN_TEMPLATE_CP: &str =
    "{about-with-newline}Usage: {usage}\n\nCommands:\n{subcommands}\n\nArguments:\n{positionals}\n";
const EN_TEMPLATE_CO: &str =
    "{about-with-newline}Usage: {usage}\n\nCommands:\n{subcommands}\n\nOptions:\n{options}\n";
const EN_TEMPLATE_PO: &str =
    "{about-with-newline}Usage: {usage}\n\nArguments:\n{positionals}\n\nOptions:\n{options}\n";
const EN_TEMPLATE_CPO: &str = "{about-with-newline}Usage: {usage}\n\nCommands:\n{subcommands}\n\nArguments:\n{positionals}\n\nOptions:\n{options}\n";

const ZH_TEMPLATE_NONE: &str = "{about-with-newline}用法：{usage}\n";
const ZH_TEMPLATE_C: &str = "{about-with-newline}用法：{usage}\n\n命令：\n{subcommands}\n";
const ZH_TEMPLATE_P: &str = "{about-with-newline}用法：{usage}\n\n参数：\n{positionals}\n";
const ZH_TEMPLATE_O: &str = "{about-with-newline}用法：{usage}\n\n选项：\n{options}\n";
const ZH_TEMPLATE_CP: &str =
    "{about-with-newline}用法：{usage}\n\n命令：\n{subcommands}\n\n参数：\n{positionals}\n";
const ZH_TEMPLATE_CO: &str =
    "{about-with-newline}用法：{usage}\n\n命令：\n{subcommands}\n\n选项：\n{options}\n";
const ZH_TEMPLATE_PO: &str =
    "{about-with-newline}用法：{usage}\n\n参数：\n{positionals}\n\n选项：\n{options}\n";
const ZH_TEMPLATE_CPO: &str = "{about-with-newline}用法：{usage}\n\n命令：\n{subcommands}\n\n参数：\n{positionals}\n\n选项：\n{options}\n";

fn help_template(
    language: UiLanguage,
    commands: bool,
    positionals: bool,
    options: bool,
) -> &'static str {
    match (language, commands, positionals, options) {
        (UiLanguage::En, false, false, false) => EN_TEMPLATE_NONE,
        (UiLanguage::En, true, false, false) => EN_TEMPLATE_C,
        (UiLanguage::En, false, true, false) => EN_TEMPLATE_P,
        (UiLanguage::En, false, false, true) => EN_TEMPLATE_O,
        (UiLanguage::En, true, true, false) => EN_TEMPLATE_CP,
        (UiLanguage::En, true, false, true) => EN_TEMPLATE_CO,
        (UiLanguage::En, false, true, true) => EN_TEMPLATE_PO,
        (UiLanguage::En, true, true, true) => EN_TEMPLATE_CPO,
        (UiLanguage::ZhCn, false, false, false) => ZH_TEMPLATE_NONE,
        (UiLanguage::ZhCn, true, false, false) => ZH_TEMPLATE_C,
        (UiLanguage::ZhCn, false, true, false) => ZH_TEMPLATE_P,
        (UiLanguage::ZhCn, false, false, true) => ZH_TEMPLATE_O,
        (UiLanguage::ZhCn, true, true, false) => ZH_TEMPLATE_CP,
        (UiLanguage::ZhCn, true, false, true) => ZH_TEMPLATE_CO,
        (UiLanguage::ZhCn, false, true, true) => ZH_TEMPLATE_PO,
        (UiLanguage::ZhCn, true, true, true) => ZH_TEMPLATE_CPO,
    }
}

fn catalog_entry(entries: &[CatalogEntry], path: &str) -> Option<CatalogEntry> {
    entries.iter().copied().find(|entry| entry.path == path)
}

fn command_entry(path: &str) -> Option<CatalogEntry> {
    catalog_entry(COMMAND_CATALOG, path)
        .or_else(|| {
            path.strip_prefix("help.")
                .and_then(|path| catalog_entry(COMMAND_CATALOG, path))
        })
        .or_else(|| {
            path.split_once(".help.").and_then(|(prefix, suffix)| {
                let candidate = format!("{prefix}.{suffix}");
                catalog_entry(COMMAND_CATALOG, &candidate)
            })
        })
        .or_else(|| {
            path.ends_with(".help")
                .then(|| catalog_entry(COMMAND_CATALOG, "help"))
                .flatten()
        })
}

fn arg_entry(path: &str, id: &str) -> Option<CatalogEntry> {
    let exact = format!("{path}.{id}");
    catalog_entry(ARG_CATALOG, &exact)
        .or_else(|| {
            path.strip_prefix("help.")
                .and_then(|path| catalog_entry(ARG_CATALOG, &format!("{path}.{id}")))
        })
        .or_else(|| {
            path.split_once(".help.").and_then(|(prefix, suffix)| {
                let candidate = format!("{prefix}.{suffix}.{id}");
                catalog_entry(ARG_CATALOG, &candidate)
            })
        })
        .or_else(|| {
            path.ends_with(".help")
                .then(|| catalog_entry(ARG_CATALOG, &format!("help.{id}")))
                .flatten()
        })
        .or_else(|| catalog_entry(ARG_CATALOG, &format!(".{id}")))
}

fn value_name(id: &str, language: UiLanguage) -> Option<&'static str> {
    match (id, language) {
        ("language", UiLanguage::En) => Some("LANGUAGE"),
        ("language", UiLanguage::ZhCn) => Some("语言"),
        ("config", UiLanguage::En) => Some("CONFIG"),
        ("config", UiLanguage::ZhCn) => Some("配置文件"),
        ("profile", UiLanguage::En) => Some("PROFILE"),
        ("profile", UiLanguage::ZhCn) => Some("配置"),
        ("mode", UiLanguage::En) => Some("MODE"),
        ("mode", UiLanguage::ZhCn) => Some("模式"),
        ("section", UiLanguage::En) => Some("SECTION"),
        ("section", UiLanguage::ZhCn) => Some("部分"),
        ("key", UiLanguage::En) => Some("KEY"),
        ("key", UiLanguage::ZhCn) => Some("键"),
        ("value", UiLanguage::En) => Some("VALUE"),
        ("value", UiLanguage::ZhCn) => Some("值"),
        ("tool", UiLanguage::En) => Some("TOOL"),
        ("tool", UiLanguage::ZhCn) => Some("工具"),
        ("arguments", UiLanguage::En) => Some("JSON"),
        ("arguments", UiLanguage::ZhCn) => Some("JSON"),
        ("arguments_file", UiLanguage::En) => Some("PATH|-"),
        ("arguments_file", UiLanguage::ZhCn) => Some("路径|-"),
        ("program", UiLanguage::En) => Some("PROGRAM"),
        ("program", UiLanguage::ZhCn) => Some("程序"),
        ("args_prefix", UiLanguage::En) => Some("ARGS_PREFIX"),
        ("args_prefix", UiLanguage::ZhCn) => Some("参数前缀"),
        ("path", UiLanguage::En) => Some("PATH"),
        ("path", UiLanguage::ZhCn) => Some("路径"),
        ("server_id", UiLanguage::En) => Some("SERVER_ID"),
        ("server_id", UiLanguage::ZhCn) => Some("服务器 ID"),
        ("url", UiLanguage::En) => Some("URL"),
        ("url", UiLanguage::ZhCn) => Some("URL"),
        ("transport", UiLanguage::En) => Some("TRANSPORT"),
        ("transport", UiLanguage::ZhCn) => Some("传输方式"),
        ("session", UiLanguage::En) => Some("SESSION"),
        ("session", UiLanguage::ZhCn) => Some("会话"),
        ("name", UiLanguage::En) => Some("NAME"),
        ("name", UiLanguage::ZhCn) => Some("名称"),
        ("cwd", UiLanguage::En) => Some("DIR"),
        ("cwd", UiLanguage::ZhCn) => Some("目录"),
        ("tunnel_id", UiLanguage::En) => Some("TUNNEL_ID"),
        ("tunnel_id", UiLanguage::ZhCn) => Some("隧道 ID"),
        ("tunnel_api_key", UiLanguage::En) => Some("API_KEY"),
        ("tunnel_api_key", UiLanguage::ZhCn) => Some("API 密钥"),
        ("hub_url", UiLanguage::En) => Some("URL"),
        ("hub_url", UiLanguage::ZhCn) => Some("URL"),
        ("hub_transport", UiLanguage::En) => Some("TRANSPORT"),
        ("hub_transport", UiLanguage::ZhCn) => Some("传输方式"),
        ("agent_id", UiLanguage::En) => Some("AGENT_ID"),
        ("agent_id", UiLanguage::ZhCn) => Some("代理 ID"),
        ("agent_secret", UiLanguage::En) => Some("SECRET"),
        ("agent_secret", UiLanguage::ZhCn) => Some("密钥"),
        ("supervisor_token", UiLanguage::En) => Some("TOKEN"),
        ("supervisor_token", UiLanguage::ZhCn) => Some("令牌"),
        ("enabled", UiLanguage::En) => Some("BOOL"),
        ("enabled", UiLanguage::ZhCn) => Some("布尔值"),
        ("subcommand", UiLanguage::En) => Some("COMMAND"),
        ("subcommand", UiLanguage::ZhCn) => Some("命令"),
        _ => None,
    }
}

fn possible_values_suffix(argument: &clap::Arg, language: UiLanguage) -> Option<String> {
    let values = argument.get_possible_values();
    let values = values
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_string())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    Some(match language {
        UiLanguage::En => format!(" [possible values: {}]", values.join(", ")),
        UiLanguage::ZhCn => format!("【可选值：{}】", values.join("、")),
    })
}

fn default_value_suffix(argument: &clap::Arg, language: UiLanguage) -> Option<String> {
    let values = argument
        .get_default_values()
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    Some(match language {
        UiLanguage::En => format!(" [default: {}]", values.join(", ")),
        UiLanguage::ZhCn => format!("【默认值：{}】", values.join("、")),
    })
}

fn localized_argument_help(
    argument: &clap::Arg,
    help: &'static str,
    language: UiLanguage,
) -> &'static str {
    let mut suffix = String::new();
    if let Some(values) = possible_values_suffix(argument, language) {
        suffix.push_str(&values);
    }
    if let Some(default) = default_value_suffix(argument, language) {
        suffix.push_str(&default);
    }
    if suffix.is_empty() {
        return help;
    }
    Box::leak(format!("{help}{suffix}").into_boxed_str())
}

fn localize_command(command: clap::Command, path: &str, language: UiLanguage) -> clap::Command {
    let mut command = command;
    if let Some(entry) = command_entry(path) {
        let about = entry.text(language);
        command = command.about(about).long_about(about);
    }

    let command_heading = match language {
        UiLanguage::En => "Commands",
        UiLanguage::ZhCn => "命令",
    };
    command = command.subcommand_help_heading(command_heading);
    let has_commands = command
        .get_subcommands()
        .any(|subcommand| !subcommand.is_hide_set());
    let has_positionals = command
        .get_arguments()
        .any(|argument| argument.is_positional() && !argument.is_hide_set());
    let has_options = command
        .get_arguments()
        .any(|argument| !argument.is_positional() && !argument.is_hide_set());
    command = command.help_template(help_template(
        language,
        has_commands,
        has_positionals,
        has_options,
    ));

    command = command.mut_args(|argument| {
        let id = argument.get_id().as_str().to_string();
        let localized = if id == "help" {
            Some(text(language).help_flag)
        } else if id == "version" {
            Some(text(language).version_flag)
        } else {
            arg_entry(path, &id).map(|entry| entry.text(language))
        };
        let mut argument = match localized {
            Some(help) => {
                let help = localized_argument_help(&argument, help, language);
                argument
                    .hide_possible_values(true)
                    .hide_default_value(true)
                    .help(help)
                    .long_help(help)
            }
            None => argument,
        };
        if let Some(value_name) = value_name(&id, language) {
            argument = argument.value_name(value_name);
        }
        argument
    });

    command = command.mut_subcommands(|subcommand| {
        let child_path = if path.is_empty() {
            subcommand.get_name().to_string()
        } else {
            format!("{path}.{}", subcommand.get_name())
        };
        localize_command(subcommand, &child_path, language)
    });
    command
}

pub(crate) fn localized_command(language: UiLanguage) -> clap::Command {
    let mut command = crate::Cli::command();
    // Building first materializes clap's generated help/version arguments and help subtree.  They
    // are then localized by the same recursive walk as derive-owned commands and arguments.
    command.build();
    localize_command(command, "", language)
}

pub(crate) fn parse_cli(
    args: Vec<OsString>,
    env: &impl LocaleSource,
) -> Result<(crate::Cli, UiLanguage), clap::Error> {
    let choice = prescan_language(&args).unwrap_or(LanguageChoice::Auto);
    let language = resolve_language(choice, env);
    let command = localized_command(language);
    let matches = command.try_get_matches_from(args)?;
    let cli = crate::Cli::from_arg_matches(&matches)?;
    Ok((cli, language))
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RenderedCliError {
    pub(crate) text: String,
    pub(crate) use_stderr: bool,
    pub(crate) exit_code: i32,
}

pub(crate) fn render_cli_error(error: clap::Error, language: UiLanguage) -> RenderedCliError {
    let help_error = matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            | ErrorKind::DisplayVersion
    );
    let use_stderr = if help_error {
        false
    } else {
        error.use_stderr()
    };
    let exit_code = if help_error { 0 } else { error.exit_code() };
    let rendered = error.render().to_string();
    let text = if language == UiLanguage::ZhCn {
        render_zh_error(&error, &rendered).unwrap_or(rendered)
    } else {
        rendered
    };
    RenderedCliError {
        text,
        use_stderr,
        exit_code,
    }
}

pub(crate) fn exit_with_cli_error(error: clap::Error, language: UiLanguage) -> ! {
    let rendered = render_cli_error(error, language);
    let mut stream: Box<dyn Write> = if rendered.use_stderr {
        Box::new(io::stderr().lock())
    } else {
        Box::new(io::stdout().lock())
    };
    let _ = stream.write_all(rendered.text.as_bytes());
    let _ = stream.flush();
    std::process::exit(rendered.exit_code);
}

fn context_string(error: &clap::Error, kind: ContextKind) -> Option<&str> {
    match error.get(kind) {
        Some(ContextValue::String(value)) => Some(value),
        _ => None,
    }
}

fn context_strings(error: &clap::Error, kind: ContextKind) -> Option<Vec<String>> {
    match error.get(kind) {
        Some(ContextValue::Strings(values)) => Some(values.clone()),
        Some(ContextValue::String(value)) => Some(vec![value.clone()]),
        _ => None,
    }
}

fn context_number(error: &clap::Error, kind: ContextKind) -> Option<isize> {
    match error.get(kind) {
        Some(ContextValue::Number(value)) => Some(*value),
        _ => None,
    }
}

fn render_zh_error(error: &clap::Error, rendered: &str) -> Option<String> {
    let message = match error.kind() {
        ErrorKind::InvalidValue => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            let value = context_string(error, ContextKind::InvalidValue)?;
            let mut message = if value.is_empty() {
                format!("参数 '{argument}' 需要一个值，但未提供")
            } else {
                format!("参数 '{argument}' 的值 '{value}' 无效")
            };
            if let Some(values) = context_strings(error, ContextKind::ValidValue) {
                if !values.is_empty() {
                    message.push_str(&format!("\n[可选值：{}]", values.join(", ")));
                }
            }
            message
        }
        ErrorKind::ValueValidation => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            let value = context_string(error, ContextKind::InvalidValue)?;
            format!("参数 '{argument}' 的值 '{value}' 无效")
        }
        ErrorKind::UnknownArgument => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            format!("发现意外参数 '{argument}'")
        }
        ErrorKind::InvalidSubcommand => {
            let subcommand = context_string(error, ContextKind::InvalidSubcommand)?;
            format!("无法识别的子命令 '{subcommand}'")
        }
        ErrorKind::MissingRequiredArgument => {
            let arguments = context_strings(error, ContextKind::InvalidArg)?;
            let mut message = String::from("未提供以下必需参数：");
            for argument in arguments {
                message.push_str("\n  ");
                message.push_str(&argument);
            }
            message
        }
        ErrorKind::MissingSubcommand => {
            let command = context_string(error, ContextKind::InvalidSubcommand)?;
            let mut message = format!("'{command}' 需要一个子命令，但未提供");
            if let Some(values) = context_strings(error, ContextKind::ValidSubcommand) {
                if !values.is_empty() {
                    message.push_str(&format!("\n[子命令：{}]", values.join(", ")));
                }
            }
            message
        }
        ErrorKind::TooManyValues => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            let value = context_string(error, ContextKind::InvalidValue)?;
            format!("参数 '{argument}' 收到意外值 '{value}'；不应再提供更多值")
        }
        ErrorKind::TooFewValues => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            let actual = context_number(error, ContextKind::ActualNumValues)?;
            let minimum = context_number(error, ContextKind::MinValues)?;
            format!("参数 '{argument}' 至少需要 {minimum} 个值，但只提供了 {actual} 个")
        }
        ErrorKind::WrongNumberOfValues => {
            let argument = context_string(error, ContextKind::InvalidArg)?;
            let actual = context_number(error, ContextKind::ActualNumValues)?;
            let expected = context_number(error, ContextKind::ExpectedNumValues)?;
            format!("参数 '{argument}' 需要 {expected} 个值，但提供了 {actual} 个")
        }
        _ => return None,
    };

    let tail = rendered
        .find("\n\n")
        .map(|index| &rendered[index..])
        .unwrap_or("\n");
    let tail = tail
        .replace("Usage:", "用法：")
        .replace("For more information, try", "更多信息请尝试")
        .replace("tip:", "提示：")
        .replace("a similar subcommand exists:", "存在相似的子命令：")
        .replace("a similar argument exists:", "存在相似的参数：")
        .replace("a similar value exists:", "存在相似的值：");
    Some(format!("错误：{message}{tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct TestEnv(BTreeMap<String, OsString>);

    impl LocaleSource for TestEnv {
        fn get(&self, key: &str) -> Option<OsString> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn explicit_language_overrides_locale_environment() {
        let env = TestEnv(BTreeMap::from([("LC_ALL".into(), "en_US.UTF-8".into())]));
        assert_eq!(
            resolve_language(LanguageChoice::ZhCn, &env),
            UiLanguage::ZhCn
        );
    }

    #[test]
    fn locale_precedence_is_lc_all_then_lc_messages_then_lang() {
        let env = TestEnv(BTreeMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("LC_MESSAGES".into(), "zh_TW.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
        ]));
        assert_eq!(resolve_language(LanguageChoice::Auto, &env), UiLanguage::En);
    }

    #[test]
    fn prescan_accepts_equals_and_split_forms_anywhere() {
        assert_eq!(
            prescan_language(&[
                "agentic-gpt".into(),
                "config".into(),
                "--language=zh-CN".into(),
            ])
            .unwrap(),
            LanguageChoice::ZhCn
        );
        assert_eq!(
            prescan_language(&[
                "agentic-gpt".into(),
                "config".into(),
                "init".into(),
                "--language".into(),
                "en".into(),
            ])
            .unwrap(),
            LanguageChoice::En
        );
    }

    #[test]
    fn every_catalog_entry_is_non_empty_for_each_language() {
        for language in [UiLanguage::ZhCn, UiLanguage::En] {
            let catalog = text(language);
            let entries = [
                catalog.app_about,
                catalog.config_about,
                catalog.config_init_about,
                catalog.config_keys_about,
                catalog.config_set_about,
                catalog.usage_heading,
                catalog.commands_heading,
                catalog.options_heading,
                catalog.arguments_heading,
                catalog.help_flag,
                catalog.version_flag,
                catalog.cancelled,
                catalog.initialized,
                catalog.replace_tunnel_id,
                catalog.provision_tunnel_secret,
                catalog.configure_hub_url,
                catalog.replace_agent_secret,
                catalog.optional_settings_prompt,
                catalog.safe_defaults_option,
                catalog.choose_sections_option,
            ];

            for entry in entries {
                assert!(!entry.is_empty());
            }
            assert!(COMMAND_CATALOG
                .iter()
                .all(|entry| !entry.en.is_empty() && !entry.zh_cn.is_empty()));
            assert!(ARG_CATALOG
                .iter()
                .all(|entry| !entry.en.is_empty() && !entry.zh_cn.is_empty()));
        }
    }

    fn assert_command_tree(command: &clap::Command, path: &str) {
        if !command.is_hide_set() {
            assert!(
                command_entry(path).is_some(),
                "visible command path is missing from the catalog: {path}"
            );
            assert!(
                command
                    .get_about()
                    .or_else(|| command.get_long_about())
                    .is_some_and(|about| !about.to_string().trim().is_empty()),
                "missing command catalog entry for {path}"
            );
        }
        for argument in command
            .get_arguments()
            .filter(|argument| !argument.is_hide_set())
        {
            let id = argument.get_id().as_str();
            if id != "help" && id != "version" {
                assert!(
                    arg_entry(path, id).is_some(),
                    "visible argument is missing from the catalog: {path}.{id}"
                );
            }
            assert!(
                argument
                    .get_help()
                    .or_else(|| argument.get_long_help())
                    .is_some_and(|help| !help.to_string().trim().is_empty()),
                "missing argument catalog entry for {path}.{id}"
            );
        }
        for subcommand in command.get_subcommands() {
            let child_path = if path.is_empty() {
                subcommand.get_name().to_string()
            } else {
                format!("{path}.{}", subcommand.get_name())
            };
            assert_command_tree(subcommand, &child_path);
        }
    }

    #[test]
    fn localized_command_tree_has_complete_visible_metadata() {
        for language in [UiLanguage::En, UiLanguage::ZhCn] {
            let command = localized_command(language);
            assert_command_tree(&command, "");
        }
    }

    #[test]
    fn localized_help_errors_keep_stream_and_exit_semantics() {
        let mut command = localized_command(UiLanguage::ZhCn);
        let error = command
            .try_get_matches_from_mut(["agentic-gpt", "config", "init", "--mode", "nope"])
            .unwrap_err();
        let rendered = render_cli_error(error, UiLanguage::ZhCn);
        assert!(rendered.use_stderr);
        assert_eq!(rendered.exit_code, 2);
        assert!(rendered.text.contains("nope"));
        assert!(rendered.text.contains("standalone"));
        assert!(rendered.text.contains("无效"));

        let mut command = localized_command(UiLanguage::ZhCn);
        let error = command
            .try_get_matches_from_mut(["agentic-gpt", "config"])
            .unwrap_err();
        let rendered = render_cli_error(error, UiLanguage::ZhCn);
        assert!(!rendered.use_stderr);
        assert_eq!(rendered.exit_code, 0);
        assert!(rendered.text.contains("命令："));
    }
}
