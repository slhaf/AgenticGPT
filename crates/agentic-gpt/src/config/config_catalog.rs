//! Shared descriptions and addressing for CLI, TUI, and the configuration API.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::{
    config::{self, Config},
    config_cli::config_keys::CONFIG_KEYS,
};
use agentic_gpt_protocol::{
    ConfigApplyEffect, ConfigChange, ConfigManualDetail, ConfigManualRequest, ConfigManualResponse,
};

#[derive(Clone, Debug)]
pub(crate) struct ConfigEntry {
    pub(crate) path: Vec<String>,
    pub(crate) section: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) kind: String,
    pub(crate) nullable: bool,
    pub(crate) example: Value,
    pub(crate) sensitive: bool,
    pub(crate) collection: bool,
    pub(crate) choices: Vec<String>,
    pub(crate) default_value: Option<Value>,
}

/// Read values after defaults, including sparse fields whose omission has a runtime meaning.
pub(crate) fn snapshot(config: &Config) -> Result<Value> {
    let mut value = serde_json::to_value(config)?;
    if value.get("shell").is_none() {
        value["shell"] = json!({});
    }
    if value["shell"].get("initFile").is_none() {
        value["shell"]["initFile"] = json!(config::DEFAULT_SHELL_INIT_FILE);
    }
    value["browser"] = json!({"managed":config.browser.managed,"runtime":config.browser.runtime});
    if value.get("tunnel").is_none() {
        value["tunnel"] = Value::Null;
    }
    Ok(value)
}

pub(crate) fn pointer(path: &[String]) -> String {
    path.iter()
        .map(|part| format!("/{}", part.replace('~', "~0").replace('/', "~1")))
        .collect()
}

fn static_path(key: &str) -> Vec<String> {
    if let Some(event) = key.strip_prefix("events.internalOverrides.") {
        return vec!["events".into(), "internalOverrides".into(), event.into()];
    }
    key.split('.').map(str::to_owned).collect()
}

pub(crate) fn section(path: &[String]) -> &'static str {
    match path.first().map(String::as_str) {
        Some("mode" | "profile") => "runtime",
        Some("agentId" | "displayName") => "identity",
        Some("workspaceRoot") => "workspace",
        Some("backupLimit") => "runtime",
        Some("confirmationProvider" | "confirmationLanguage") => "confirmation",
        Some("httpMcp") => "http-mcp",
        Some("mcpServers") => "mcp",
        Some("toolsets") => "toolsets",
        Some("config") => "config",
        Some("hub") => "hub",
        Some("policy") => "policy",
        Some("shell") => "shell",
        Some("limits") => "limits",
        Some("skills") => "skills",
        Some("room") => "room",
        Some("email") => "email",
        Some("tunnel") => "tunnel",
        Some("browser") => "browser",
        Some("events") => "events",
        _ => "runtime",
    }
}

fn sensitive_leaf(path: &[String]) -> bool {
    let parts = path.iter().map(String::as_str).collect::<Vec<_>>();
    matches!(
        parts.as_slice(),
        ["hub", "agentSecret"]
            | ["httpMcp", "bearerToken"]
            | ["tunnel", "apiKey"]
            | ["email", "account", "password"]
            | [
                "room",
                "secret",
                "serverUrl" | "account" | "clientId" | "clientSecret" | "masterPassword"
            ]
            | ["mcpServers", _, "auth", "token"]
    )
}

pub(crate) fn effect(path: &[String]) -> ConfigApplyEffect {
    let parts = path.iter().map(String::as_str).collect::<Vec<_>>();
    match parts.as_slice() {
        ["mode" | "profile" | "agentId" | "workspaceRoot", ..] => {
            ConfigApplyEffect::RestartRequired
        }
        ["tunnel", "hubReporting", "detail", ..] => ConfigApplyEffect::LiveUpdate,
        ["tunnel", "hubReporting"] | ["tunnel", "hubReporting", "enabled", ..] => {
            ConfigApplyEffect::ComponentReconfigure
        }
        ["tunnel", ..] => ConfigApplyEffect::RestartRequired,
        ["browser" | "hub" | "httpMcp" | "email" | "toolsets", ..] => {
            ConfigApplyEffect::ComponentReconfigure
        }
        ["room", "secret", "getPolicy" | "upsertPolicy" | "deletePolicy" | "syncWaitTimeoutMs"] => {
            ConfigApplyEffect::LiveUpdate
        }
        ["room"]
        | ["skills"]
        | ["room", "secret", ..]
        | ["room", "repositoryRoot"]
        | ["skills", "maxConcurrentInstalls"] => ConfigApplyEffect::ComponentReconfigure,
        _ => ConfigApplyEffect::LiveUpdate,
    }
}

fn collection(path: &[String]) -> bool {
    let parts = path.iter().map(String::as_str).collect::<Vec<_>>();
    matches!(
        parts.as_slice(),
        ["toolsets", "enabled"]
            | ["policy", "command", "allow" | "confirm" | "deny"]
            | ["policy", "path", "readOnly" | "write" | "deny"]
            | ["skills", "allowedHosts"]
            | ["httpMcp", "allowHosts"]
    )
}

fn title(path: &[String]) -> String {
    let key = path.iter().map(String::as_str).collect::<Vec<_>>();
    match key.as_slice() {
        ["config", "updatePolicy"] => "配置修改确认策略",
        ["browser", "managed", "enabled"] => "启用托管浏览器运行环境",
        ["browser", "managed", "autoProvision"] => "自动准备浏览器运行环境",
        ["browser", "runtime"] => "显式浏览器运行环境",
        ["browser", "runtime", "nodeModuleDirs"] => "Node 模块查找目录",
        ["mcpServers", "{serverId}", "enabled"] => "启用此 MCP 服务",
        ["mcpServers", "{serverId}", "transport"] => "MCP 服务传输方式",
        ["mcpServers", "{serverId}", "url"] => "MCP 服务地址或 stdio 启动命令",
        ["email", "account", "password"] => "邮箱密码引用",
        _ => return path.join("."),
    }
    .to_owned()
}

fn description(path: &[String]) -> String {
    let parts = path.iter().map(String::as_str).collect::<Vec<_>>();
    match parts.as_slice() {
        ["config", "updatePolicy"] => "allow 允许配置 API 修改；confirm 必须取得明确批准。确认不可用、拒绝、超时或取消均不写入；修改本策略按修改前的有效值授权。",
        ["browser", "managed", "enabled"] => "允许使用托管 Browser 运行环境；禁用不影响显式配置的运行环境。变更可能需要重建 Browser 租约。",
        ["browser", "managed", "autoProvision"] => "允许 Browser 在首次需要时准备缺失依赖；准备结果仍受本机环境影响。",
        ["browser", "runtime", "nodeModuleDirs"] => "按顺序设置 Node 模块查找目录；使用 set 替换有序列表。",
        ["browser", "runtime", _] => "显式 Browser 运行环境字段；null 表示未指定。变更后重建运行环境，旧租约可能失效。路径与外部依赖会在准备或调用时检查。",
        ["mcpServers"] => "以服务 ID 为 key 的映射。ID 可含点、下划线和连字符，长度 1–64；用真实路径段寻址。新建条目必须包含 enabled、transport 及对应 url。",
        ["mcpServers", "{serverId}", "transport"] => "streamable-http 使用 HTTP(S) URL；stdio 使用启动命令且不支持 bearer auth。新调用使用新配置，进行中调用保持原快照。",
        ["mcpServers", "{serverId}", "auth", "token"] => "Bearer token；查询、预览和报告脱敏，stdio 传输不允许设置 auth。",
        ["email", "account", "password"] => "仅接受 env:NAME 或 file:PATH 凭据引用；不接受明文密码。",
        ["email", "account", _] => "邮箱账户字段；启用邮件时须构成完整有效账户。同批提交关联字段后统一校验。",
        ["toolsets", "enabled"] => "启用的工具集集合；add/remove 幂等。config 在所有初始化预设中默认不选中；未选中时三个配置工具均不可调用，本地 CLI/TUI 管理不受影响。",
        ["room", "timezone"] => "当前版本保存此字段，但现有 Room 运行逻辑尚未使用它；修改或重启不会改变已有仓库的日记时区。",
        ["room", "diaryDayBoundaryHour"] => "0–23；仅用于新空 Room 仓库初始化的逻辑日期计算，不改写已有仓库。",
        _ => "通过真实配置路径读取或修改；对象 set 替换子树，unset 恢复缺省或删除 map 条目。",
    }.to_owned()
}

// Optional object templates describe shape only. Actual defaults still come from Config/Serde.
fn shape() -> Result<Value> {
    let mut value = serde_json::to_value(Config::default_config()?)?;
    value["browser"] = json!({
        "managed": serde_json::to_value(config::ManagedBrowserConfig::default())?,
        "runtime": serde_json::to_value(serde_json::from_value::<config::ExplicitBrowserRuntimeConfig>(json!({}))?)?
    });
    value["shell"] = json!({"initFile": null});
    value["tunnel"] = serde_json::to_value(config::TunnelConfig::default())?;
    value["email"]["account"] = serde_json::to_value(serde_json::from_value::<
        config::EmailAccountConfig,
    >(json!({
        "imapHost":"", "smtpHost":"", "username":"", "password":"", "from":""
    }))?)?;
    value["mcpServers"] = json!({"{serverId}":{"enabled":false,"transport":"streamable-http","url":"https://mcp.example.com/mcp","auth":{"type":"bearer","token":""}}});
    // Serde deliberately omits optional fields in saved files. Their edit paths
    // still exist; reuse the established field descriptions rather than dropping them.
    for spec in CONFIG_KEYS.iter().filter(|spec| spec.alias_of.is_none()) {
        let path = static_path(spec.key);
        if value.pointer(&pointer(&path)).is_some() {
            continue;
        }
        let mut parent = &mut value;
        for segment in &path[..path.len() - 1] {
            if parent.get(segment).is_none() {
                parent[segment] = json!({});
            }
            parent = parent
                .get_mut(segment)
                .ok_or_else(|| anyhow!("config_catalog_parent_invalid"))?;
        }
        parent[path.last().expect("nonempty configuration key")] =
            serde_json::from_str(spec.example).unwrap_or_else(|_| json!(spec.example));
    }
    for event in config::INTERNAL_EVENT_TYPES {
        value["events"]["internalOverrides"][*event] = json!("low");
    }
    Ok(value)
}

pub(crate) fn entries() -> Result<Vec<ConfigEntry>> {
    let defaults = snapshot(&Config::default_config()?)?;
    let mut entries = Vec::new();
    fn walk(value: &Value, path: &mut Vec<String>, defaults: &Value, out: &mut Vec<ConfigEntry>) {
        if !path.is_empty() {
            let spec = CONFIG_KEYS
                .iter()
                .find(|spec| spec.alias_of.is_none() && static_path(spec.key) == *path);
            let kind = spec
                .map(|s| s.kind.as_str())
                .unwrap_or(match value {
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "non-negative-integer",
                    Value::Array(_) => "json-string-array",
                    Value::Object(_) => "object",
                    Value::Null => "nullable-string",
                    _ => "string",
                })
                .to_owned();
            let nullable = value.is_null()
                || spec
                    .is_some_and(|s| s.nullable && s.kind.as_str() != "event-notification-level")
                || matches!(
                    path.iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    ["browser", "runtime"] | ["email", "account"] | ["tunnel"]
                );
            let choices = match path
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["config", "updatePolicy"] => vec!["allow".into(), "confirm".into()],
                ["mcpServers", "{serverId}", "auth", "type"] => vec!["bearer".into()],
                ["mcpServers", "{serverId}", "transport"] => {
                    vec!["streamable-http".into(), "stdio".into()]
                }
                ["email", "account", "imapTls" | "smtpTls"] => {
                    vec!["implicit".into(), "starttls".into()]
                }
                _ => spec
                    .and_then(|s| s.kind.choices())
                    .unwrap_or_default()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            };
            let custom = matches!(
                path.first().map(String::as_str),
                Some("browser" | "mcpServers" | "config" | "toolsets")
            ) || matches!(
                path.iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice(),
                ["room", "timezone" | "diaryDayBoundaryHour"]
            );
            let description = if custom {
                description(path)
            } else {
                spec.map_or_else(|| description(path), |s| s.description.zh_cn.to_owned())
            };
            out.push(ConfigEntry {
                path: path.clone(),
                section: section(path).into(),
                title: title(path),
                description,
                kind,
                nullable,
                example: spec
                    .map(|s| serde_json::from_str(s.example).unwrap_or_else(|_| json!(s.example)))
                    .unwrap_or_else(|| value.clone()),
                sensitive: sensitive_leaf(path),
                collection: collection(path),
                choices,
                default_value: defaults.pointer(&pointer(path)).cloned(),
            });
        }
        if let Some(object) = value.as_object() {
            for (key, child) in object {
                path.push(key.clone());
                walk(child, path, defaults, out);
                path.pop();
            }
        }
    }
    walk(&shape()?, &mut Vec::new(), &defaults, &mut entries);
    Ok(entries)
}

fn matches_template(template: &[String], path: &[String]) -> bool {
    template.len() == path.len()
        && template
            .iter()
            .zip(path)
            .all(|(a, b)| a == "{serverId}" || a == b)
}

pub(crate) fn entry(path: &[String]) -> Result<ConfigEntry> {
    if path.len() > 16
        || path
            .iter()
            .any(|p| p.is_empty() || p.len() > 256 || p.chars().any(char::is_control))
    {
        return Err(anyhow!("config_path_invalid"));
    }
    let mut entry = entries()?
        .into_iter()
        .find(|item| matches_template(&item.path, path))
        .ok_or_else(|| anyhow!("config_path_unknown: {}", pointer(path)))?;
    entry.path = path.to_vec();
    Ok(entry)
}

pub(crate) fn resolve_cli_key(key: &str) -> Result<Vec<String>> {
    let path = if key.starts_with('[') {
        serde_json::from_str(key)?
    } else if let Some(pointer) = key.strip_prefix('/') {
        pointer
            .split('/')
            .map(|p| p.replace("~1", "/").replace("~0", "~"))
            .collect()
    } else {
        let canonical = CONFIG_KEYS
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| s.alias_of)
            .unwrap_or(key);
        static_path(canonical)
    };
    entry(&path)?;
    Ok(path)
}

pub(crate) fn parse_cli_change(key: &str, raw: &str) -> Result<ConfigChange> {
    let path = resolve_cli_key(key)?;
    let spec = entry(&path)?;
    if path.starts_with(&["events".into(), "internalOverrides".into()]) && raw == "null" {
        return Ok(ConfigChange::Unset { path });
    }
    let value = if raw == "null" && spec.nullable {
        Value::Null
    } else if spec.kind == "object"
        || spec.kind == "email-account-object-or-null"
        || spec.kind.contains("array")
        || matches!(
            spec.kind.as_str(),
            "boolean" | "port" | "non-negative-integer" | "auto-or-non-negative-integer"
        )
    {
        serde_json::from_str(raw).or_else(|error| {
            if spec.kind == "auto-or-non-negative-integer" && raw == "auto" {
                Ok(json!("auto"))
            } else {
                Err(error)
            }
        })?
    } else {
        json!(raw)
    };
    Ok(ConfigChange::Set { path, value })
}

pub(crate) fn redact(path: &[String], value: &Value) -> (Value, bool) {
    fn wrap(path: &[String], value: Value) -> Value {
        if let Some((head, tail)) = path.split_first() {
            let mut map = serde_json::Map::new();
            map.insert(head.clone(), wrap(tail, value));
            Value::Object(map)
        } else {
            value
        }
    }
    if value.is_null() {
        return (Value::Null, false);
    }
    let mut wrapped = wrap(path, value.clone());
    config::redact_config_secrets(&mut wrapped);
    let redacted = wrapped
        .pointer(&pointer(path))
        .cloned()
        .unwrap_or(Value::Null);
    let changed = redacted != *value;
    (redacted, changed)
}

fn chapter(section: &str) -> &'static str {
    match section {
        "runtime" | "identity" | "workspace" => "mode、profile、agentId 和 workspaceRoot 固定于本次实例启动；保存后须手动重启才能应用。displayName 和 backupLimit 使用新快照。",
        "config" => "配置 API 只管理目标 Agent。整个 update 批次统一按当前有效 updatePolicy 授权；修改策略自身也须先通过旧策略。CLI/TUI 是本地管理入口。",
        "toolsets" => "工具集在实际调用时再次检查；Room 七个子工具集独立控制发现、准入和资源，不接受聚合 room。关闭 Email/Secret 时排空并释放 owner，保留账户与数据；关闭 config 后可从本地 CLI/TUI 重新启用。",
        "hub" => "Hub 连接由当前实例的连接 owner 重连。配置此连接的请求先向原 Hub 返回结果；旧结果不会转发给另一 Hub。连接失败不撤销配置保存。",
        "http-mcp" => "HTTP MCP 入口属于 standalone。监听和认证由 HTTP owner 更新；旧已接纳请求完成后关闭旧会话。新地址绑定失败尽可能保留旧监听；外部凭据引用仍须可解析。",
        "mcp" => "这是以稳定 server ID 为 key 的映射。ID 中的点是一个路径段；用同一批操作补齐 enabled、transport、url 及可选 auth。下游客户端按调用创建，进行中调用保留原快照。",
        "tunnel" => "Tunnel 身份、凭据和客户端启动参数保存后须手动重启；hubReporting.enabled 局部控制辅助 Hub，detail 直接更新报告策略。自动故障恢复沿用原启动契约。",
        "browser" => "显式 runtime 优先于托管发现。字段修改会准备新运行环境并排空旧调用，重建后旧租约失效；准备失败保留可用旧 owner，不重放浏览器操作。",
        "email" => "启用时 account 必须完整且使用 env:/file: 密码引用。账户切换等待在途操作结束并释放旧 owner，保留邮件存储；其他设置按新请求快照使用，不重放发送。",
        "room" => "仓库路径切换只切换并初始化新目标，不迁移已有内容。Secret 后端和凭据由自身 owner 重新建立；策略及 syncWaitTimeoutMs 直接更新，已接纳请求保留快照。",
        "skills" => "新安装读取新的限制；并发上限降低不取消正在运行的安装，待其完成后逐步达到新上限。",
        "policy" | "shell" | "limits" | "events" | "confirmation" => "新准入请求使用更新后的配置；已准入 Process/Skill 操作继续使用原配置快照。凭据值在查询、预览及公共报告中脱敏。",
        _ => "值使用原生 JSON，批量修改最终整体校验。",
    }
}

pub(crate) fn manual(request: &ConfigManualRequest) -> Result<ConfigManualResponse> {
    let entries = entries()?;
    let sections = entries
        .iter()
        .map(|e| e.section.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(section) = &request.section {
        if !sections.contains(section.as_str()) {
            return Err(anyhow!("config_section_unknown"));
        }
    }
    let mut content = String::from("# Agent 配置手册\n\n使用路径段数组寻址；值使用原生 JSON。set 替换目标；unset 恢复缺省或删除条目；集合 add/remove 幂等。有序列表整体 set。批量最终校验后一次保存，保存与运行态生效分别报告。\n\n");
    if request.section.is_none() && request.query.is_none() {
        for section in &sections {
            content.push_str(&format!(
                "- `{section}`：{} 个配置条目。{}\n",
                entries.iter().filter(|e| e.section == *section).count(),
                chapter(section)
            ));
        }
        return Ok(ConfigManualResponse {
            content,
            matches: sections.len(),
        });
    }
    let query = request.query.as_deref().unwrap_or("").to_lowercase();
    let matched = entries
        .iter()
        .filter(|e| {
            request.section.as_ref().is_none_or(|s| *s == e.section)
                && format!("{} {} {}", e.path.join("."), e.title, e.description)
                    .to_lowercase()
                    .contains(&query)
        })
        .collect::<Vec<_>>();
    let mut current_section = None;
    for item in &matched {
        if current_section != Some(item.section.as_str()) {
            content.push_str(&format!(
                "## {}\n\n{}\n\n",
                item.section,
                chapter(&item.section)
            ));
            current_section = Some(item.section.as_str());
        }
        content.push_str(&format!(
            "## {}\n\n路径：`{}`\n\n{}\n\n",
            item.title,
            serde_json::to_string(&item.path)?,
            item.description
        ));
        if request.detail == ConfigManualDetail::Full {
            content.push_str(&format!("类型：{}；可空：{}；支持：set{}{}。\n\n", item.kind, item.nullable, if matches!(item.path.as_slice(), [key] if matches!(key.as_str(), "mode" | "profile" | "toolsets")) { "" } else { "/unset" }, if item.collection { "/add/remove" } else { "" }));
            let default = item.default_value.as_ref().map(|v| redact(&item.path, v).0);
            content.push_str(&format!(
                "默认规则：{}。生效方式：`{}`。\n\n",
                default.map_or_else(
                    || "未指定；创建父条目时按关联约束补齐".to_owned(),
                    |v| v.to_string()
                ),
                serde_json::to_value(effect(&item.path))?
                    .as_str()
                    .unwrap_or_default()
            ));
            if !item.choices.is_empty() {
                content.push_str(&format!("可选值：{}。\n\n", item.choices.join("、")));
            }
            content.push_str(&format!(
                "```json\n{}\n```\n\n",
                serde_json::to_string_pretty(
                    &json!({"changes":[{"op":"set","path":item.path,"value":item.example}]})
                )?
            ));
            if item.sensitive {
                content.push_str("此值在查询、预览及报告中脱敏。请遵循原有凭据引用格式。\n\n");
            }
        }
        if content.len() > 128 * 1024 {
            return Err(anyhow!("config_manual_too_large: narrow section or query"));
        }
    }
    Ok(ConfigManualResponse {
        content,
        matches: matched.len(),
    })
}
