use std::sync::Arc;

use agentic_gpt_protocol::{MAX_PROCESS_RESPONSE_BYTES, MIN_PROCESS_RESPONSE_BYTES};
use rmcp::model::{Meta, Tool, ToolAnnotations};
use serde_json::{json, Map, Value};

use crate::{
    config::ToolsetConfig,
    operation::{
        tool_is_destructive, tool_is_open_world, tool_is_read_only, TOOL_NAMESPACE_BY_NAME,
    },
};

const PATCH_SCHEMA_DESCRIPTION: &str =
    "以 *** Begin Patch 开始、*** End Patch 结束；支持新增、删除、更新和移动文件。";

fn wait_seconds_schema(default: u64, description: &'static str) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "maximum": 30,
        "default": default,
        "description": description,
    })
}
pub(super) fn tool_descriptors(toolsets: &ToolsetConfig) -> Vec<Tool> {
    let mut tools = TOOL_NAMESPACE_BY_NAME
        .iter()
        .filter(|(_, namespace)| toolsets.is_enabled(*namespace))
        .map(|(name, _)| tool_descriptor(name))
        .collect::<Vec<_>>();
    tools.extend(
        crate::operation::EVENT_API_TOOL_NAMES
            .iter()
            .map(|name| tool_descriptor(name)),
    );
    tools.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    tools
}

pub(super) fn tool_descriptor(name: &str) -> Tool {
    let input_schema = tool_input_schema(name);
    let annotations = ToolAnnotations::new()
        .read_only(tool_is_read_only(name))
        .destructive(tool_is_destructive(name))
        .open_world(tool_is_open_world(name));
    Tool::new(name.to_string(), tool_description(name), input_schema)
        .with_annotations(annotations)
        .with_raw_output_schema(Arc::new(output_schema()))
        .with_meta(Meta(Map::from_iter([(
            "surface".to_string(),
            Value::String("agent-local".to_string()),
        )])))
}

fn tool_input_schema(name: &str) -> Map<String, Value> {
    let (properties, required) = tool_schema(name);
    // Keep the exposed schema in the simple object/property subset supported by
    // MCP consumers. Conditional relationships (for example single vs batch
    // file operations) are enforced by runtime validation and described on the
    // tool/field level instead of using top-level oneOf.
    schema(properties, required)
}

fn tool_schema(name: &str) -> (Map<String, Value>, &'static [&'static str]) {
    let required: &'static [&'static str] = match name {
        "process.exec" => &["command"],
        "browser.manual" => &["action"],
        "browser.acquire" => &["name", "idleTimeoutSeconds"],
        "browser.repl" => &["name", "code"],
        "browser.reset" | "browser.release" => &["name"],
        "browser.list" => &[],
        "process.read" | "process.cancel" => &["processId"],
        "event.get" => &["eventId"],
        "event.mark" => &["eventIds"],
        "privateevent.inject" => &["message", "ref"],
        "file.read" | "file.search" => &[],
        "file.edit" => &["patch"],
        "mcp.callTool" => &["serverId", "toolName"],
        "mcp.batch" => &["calls"],
        "room.maintenance.submit" => &["items"],
        "skills.setActive" => &["id", "active"],
        "tmux.sessions" | "tmux.panes" => &["action"],
        "tmux.exec" => &["target", "program"],
        "tmux.pasteText" => &["target", "text"],
        "tmux.listPanes" => &["agentId"],
        "tmux.capturePane" => &["agentId", "target"],
        "tmux.createSession" => &["agentId", "name", "cwd"],
        "tmux.closeSession" => &["agentId", "name"],
        "mcp.listTools" => &["agentId", "serverId"],
        "bootstrap.read" => &["id"],
        "skills.read" => &["id"],
        "skills.search" => &["query"],
        "skills.activate" | "skills.deactivate" => &["id"],
        "skills.install" => &["id", "source"],
        "skills.install.get" | "skills.install.cancel" => &["installId"],
        "skills.run" => &["id", "path"],
        "room.diary.read" => &["layer", "period"],
        "room.notebook.search" => &["query"],
        "room.notebook.read" => &["path"],
        "room.state.read" => &["entity"],
        _ => &[],
    };
    (properties_for(name), required)
}

pub(super) fn properties_for(name: &str) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut add = |key: &str, value: Value| {
        properties.insert(key.to_string(), value);
    };
    let string = |description: &str| json!({"type": "string", "description": description});
    let number = |description: &str| json!({"type": "integer", "description": description});
    let boolean = |description: &str| json!({"type": "boolean", "description": description});
    let strings = |description: &str| json!({"type": "array", "items": {"type": "string"}, "description": description});
    if matches!(
        name,
        "tmux.listPanes"
            | "tmux.capturePane"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.listTools"
    ) {
        add("agentId", string("目标 Agent 的本地 ID。"));
    }
    match name {
        "browser.manual" => {
            add(
                "action",
                json!({
                    "type": "string",
                    "enum": ["read", "search"],
                    "description": "read 读取；search 搜索 runtime 附带文档；按 action 传字段。",
                }),
            );
            add(
                "path",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "文档相对路径；read 拒绝绝对或越界路径。",
                }),
            );
            add(
                "startLine",
                json!({"type":"integer","minimum":1,"description":"read 起始行（含）。"}),
            );
            add(
                "endLine",
                json!({"type":"integer","minimum":1,"description":"read 结束行（含）。"}),
            );
            add(
                "query",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4096,
                    "description": "search 区分大小写子串；read 不可用。",
                }),
            );
            add(
                "maxResults",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "default": 50,
                    "description": "search 匹配上限。",
                }),
            );
            add(
                "contextLines",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 5,
                    "default": 2,
                    "description": "匹配上下文行数。",
                }),
            );
        }
        "browser.acquire" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "非空白 lease 名；同名复用。",
                }),
            );
            add(
                "idleTimeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "闲置回收时限；复用时更新。",
                }),
            );
        }
        "browser.repl" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "已 acquire 的 lease 名。",
                }),
            );
            add(
                "code",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 262144,
                    "description": "JavaScript（最多 256 KiB UTF-8 字节）；可操作网页/外部服务。",
                }),
            );
            add(
                "timeoutMs",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 120000,
                    "default": 20000,
                    "description": "执行等待上限。",
                }),
            );
            add(
                "title",
                json!({
                    "type": "string",
                    "maxLength": 128,
                    "description": "记录标题，不注入代码。",
                }),
            );
        }
        "browser.reset" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "要重置的 lease 名。",
                }),
            );
        }
        "browser.release" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "要最终释放的 lease 名。",
                }),
            );
        }
        "browser.list" => {}
        "file.read" => {
            add("path", string("策略允许的文件/目录路径。"));
            add("metadata", boolean("返回文件信息；缺省 false。"));
            add("startLine", number("文本起始行（含，>0）。"));
            add("endLine", number("文本结束行（含，≥startLine）。"));
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("策略允许的文件/目录路径。"),
                        "metadata":boolean("返回文件信息；缺省 false。"),
                        "startLine":number("文本起始行（含，>0）。"), "endLine":number("文本结束行（含，≥startLine）。")
                    },"required":["path"]},
                    "description":"与顶层字段互斥；逐项读取。"
                }),
            );
        }
        "file.search" => {
            add("path", string("搜索根路径。"));
            add("query", string("非空匹配文本。"));
            add(
                "mode",
                json!({"type":"string","enum":["literal","regex"],"default":"literal","description":"literal 字面；regex 正则。"}),
            );
            add("caseSensitive", boolean("区分大小写；缺省 true。"));
            add("include", strings("纳入 glob（最多 16）。"));
            add("exclude", strings("排除 glob（最多 16）。"));
            add(
                "contextLines",
                json!({
                    "type":"integer",
                    "minimum":0,
                    "description":"上下文行数；超配置上限裁剪并警告。",
                    "default":0
                }),
            );
            add("maxResults", number("结果数上限；缺省 50、最大 200。"));
            add("hidden", boolean("搜索隐藏文件；缺省 false。"));
            add("respectGitignore", boolean("遵循 Git ignore；缺省 true。"));
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("搜索根路径。"),
                        "query":string("非空匹配文本。"),
                        "mode":{"type":"string","enum":["literal","regex"],"default":"literal","description":"literal 字面匹配；regex 正则。"},
                        "caseSensitive":{"type":"boolean","default":true,"description":"是否区分大小写。"},
                        "include":strings("纳入 glob（最多 16）。"), "exclude":strings("排除 glob（最多 16）。"),
                        "contextLines":{"type":"integer","minimum":0,"default":0,"description":"上下文行数；受配置上限裁剪。"},
                        "maxResults":{"type":"integer","maximum":200,"default":50,"description":"结果数上限。"},
                        "hidden":{"type":"boolean","default":false,"description":"搜索隐藏文件。"},
                        "respectGitignore":{"type":"boolean","default":true,"description":"遵循 Git ignore。"}
                    },"required":["path","query"]},
                    "description":"与顶层字段互斥；逐项搜索。"
                }),
            );
        }
        "event.list" => {
            add("agentId", string("目标 Agent ID；省略时使用当前 Agent。"));
            add(
                "status",
                json!({
                    "type":"string",
                    "enum":["pending","handled","expired"],
                    "default":"pending",
                    "description":"事件状态筛选；省略时为 pending。"
                }),
            );
            add(
                "severity",
                json!({
                    "type":"string",
                    "enum":["low","medium","high"],
                    "description":"可选等级筛选。"
                }),
            );
            add(
                "limit",
                json!({
                    "type":"integer",
                    "minimum":1,
                    "maximum":100,
                    "default":20,
                    "description":"页大小；省略时为20，范围1–100。"
                }),
            );
            add("cursor", string("由上一页返回的不透明 nextCursor。"));
        }
        "event.get" => {
            add("agentId", string("目标 Agent ID；省略时使用当前 Agent。"));
            add("eventId", string("event.list 返回的事件 ID。"));
        }
        "event.mark" => {
            add("agentId", string("目标 Agent ID；省略时使用当前 Agent。"));
            add(
                "eventIds",
                json!({
                    "type":"array",
                    "maxItems":512,
                    "items":{"type":"string"},
                    "description":"要标记为 handled 的事件 ID；重复 ID 幂等，未知 ID 返回 notFoundIds。"
                }),
            );
        }
        "privateevent.inject" => {
            add("message", string("事件正文，最大长度由事件存储限制。"));
            add(
                "severity",
                json!({
                    "type":"string",
                    "enum":["low","medium","high"],
                    "description":"可选事件等级；省略时为 low。"
                }),
            );
            add(
                "ref",
                string("调用方提供的外部来源引用。来源种类由服务固定为 external。"),
            );
        }

        "file.edit" => {
            add("patch", string(PATCH_SCHEMA_DESCRIPTION));
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"请求写前确认。"}),
            );
        }
        "mcp.list" => add(
            "serverId",
            string("可选 server ID；省略列服务器，提供时列工具。"),
        ),
        "process.exec" => {
            add("command", string("普通非登录/非交互 Bash 原样执行；pipefail 生效，不设 set -e。cwd 在初始化后应用。策略只分析提交内容；不检查可信初始化文件中的 PATH/函数，非运行时安全边界。"));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选分组键；trim 后精确匹配。",
                }),
            );
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"请求执行前确认。"}),
            );
            add("cwd", string("可选工作目录；省略用配置默认值。"));
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"只等待终态，不会取消进程。"}),
            );
        }
        "process.batch" => {
            add(
                "elements",
                json!({
                    "type": "array",
                    "description": "有序 Bash 命令列表；元素 cwd 可覆盖批次 cwd。",
                    "items": {"type": "object", "properties": {
                        "command": string("普通 Bash 原样执行，不改写；策略只分析提交内容。"),
                        "cwd": string("单项工作目录；覆盖批次 cwd。")
                    }, "required": ["command"], "additionalProperties": false}
                }),
            );
            add("needConfirm", boolean("批次前确认；缺省 false。"));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选分组键；子进程继承。",
                }),
            );
            add("cwd", string("批次默认工作目录；单项可覆盖。"));
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"只等待终态，不会取消进程。"}),
            );
        }
        "process.read" => {
            add("processId", string("受管理进程 ID。"));
            add(
                "waitSeconds",
                wait_seconds_schema(5, "最长等待时间；缺省 5 秒，范围 0..=30 秒。"),
            );
            add(
                "view",
                json!({
                    "type": "string",
                    "enum": ["auto", "status"],
                    "default": "auto",
                    "description": "auto 在有可用输出/结果时返回产物，否则等待终态/采集结算或期限；status 只等待执行终态，不返回产物。",
                }),
            );
            add(
                "cursor",
                string("命令/脚本输出续读游标；仅用于目标进程 kind=command 或 skill。kind=mcp 的完整结果不支持日志游标。"),
            );
            add(
                "maxBytes",
                json!({
                    "type": "integer",
                    "minimum": MIN_PROCESS_RESPONSE_BYTES,
                    "maximum": MAX_PROCESS_RESPONSE_BYTES,
                    "description": "统一响应 JSON 的可选字节上限；范围 4096..=1048576。省略时使用当前 limits.processResponseBytes 配置（出厂默认 8192）；不切分 MCP 结果。",
                }),
            );
        }
        "process.cancel" => {
            add("processId", string("受管理进程 ID。"));
        }
        "process.list" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "精确分组筛选；首尾空白会裁剪。",
                }),
            );
            add(
                "kind",
                json!({
                    "type": "string",
                    "enum": ["command", "skill", "mcp"],
                    "description": "进程类型筛选。",
                }),
            );
            add(
                "state",
                json!({
                    "type": "string",
                    "enum": [
                        "queued", "waiting_confirmation", "starting", "running",
                        "completed", "failed", "rejected", "cancel_requested",
                        "cancelled", "timed_out", "detached", "unknown_after_restart",
                        "skipped"
                    ],
                    "description": "进程状态筛选。",
                }),
            );
            add(
                "limit",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "default": 50,
                    "description": "每页数量；缺省 50。",
                }),
            );
            add("cursor", string("读取下一页的游标。"));
        }
        "tmux.sessions" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "create", "close"], "description":"list 仅 action；create 需 name/cwd；close 需 name，可带 needConfirm（缺省 true）。"}),
            );
            add("name", string("create/close 的 session 名。"));
            add("cwd", string("create 的工作目录。"));
            add("needConfirm", boolean("关闭前确认；缺省 true。"));
        }
        "tmux.panes" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "capture"], "description":"list 可带 session；capture 需 target、可带 lines；参数互斥。"}),
            );
            add("session", string("list 的 session 筛选值。"));
            add("target", string("capture 的 pane target。"));
            add(
                "lines",
                number("capture 行数；缺省 160，服务端限 1..=5000。"),
            );
        }
        "tmux.listPanes" => add("session", string("可选 session；省略列出所有 panes。")),
        "tmux.capturePane" => {
            add("target", string("目标 Agent 上的 pane target。"));
            add("lines", number("返回行数；缺省 160，服务端限 1..=5000。"));
        }
        "tmux.pasteText" => {
            add("target", string("非 shell pane/TUI；shell pane 拒绝。"));
            add("text", string("写入 pane 的文本。"));
            add("submit", boolean("是否追加 Enter；缺省 false。"));
            add("needConfirm", boolean("写入前确认；缺省 true。"));
        }
        "tmux.exec" => {
            add("target", string("存活且非 copy mode 的 shell pane。"));
            add("program", string("shell 程序名/builtin。"));
            add("args", strings("program 参数数组。"));
            add("needConfirm", boolean("执行前确认；缺省 false。"));
            add(
                "waitMs",
                number("等待 pane 输出的毫秒数；缺省 300、最多 5000；不代表命令完成。"),
            );
            add("captureLines", number("快照行数；缺省 120、最多 5000。"));
        }
        "tmux.createSession" => {
            add("name", string("要创建或复用的 session 名。"));
            add("cwd", string("session 工作目录；受执行策略约束。"));
        }
        "tmux.closeSession" => {
            add("name", string("要关闭的 session 名。"));
            add(
                "needConfirm",
                boolean("关闭前确认；缺省 true，会结束 session 中任务。"),
            );
        }
        "mcp.listServers" => add("agentId", string("目标 Agent ID。")),
        "mcp.listTools" => add("serverId", string("目标 Agent 的 server ID。")),
        "mcp.batch" => {
            add(
                "calls",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 16,
                    "description": "参数序列化总量≤2 MiB；统一校验/准入。",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serverId", "toolName"],
                        "properties": {
                            "serverId": {"type": "string", "description": "下游 MCP server ID。"},
                            "toolName": {"type": "string", "description": "mcp.list(serverId) 的工具名。"},
                            "arguments": {
                                "type": "object",
                                "default": {},
                                "description": "JSON 参数；序列化≤256 KiB。",
                            }
                        }
                    }
                }),
            );
            add(
                "mode",
                json!({"type":"string","enum":["parallel","sequential"],"default":"parallel","description":"parallel 并行；sequential 按序等待终态。"}),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选分组键；子调用继承。",
                }),
            );
            add(
                "failFast",
                json!({"type":"boolean","default":false,"description":"硬失败后跳过未启动项。"}),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"内联等待。"}),
            );
            add(
                "timeoutSeconds",
                json!({"type":"integer","minimum":1,"maximum":900,"default":300,"description":"获准占槽后连接/请求时限（秒）；不含确认/排队。"}),
            );
        }
        "mcp.callTool" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选分组键；子调用继承。",
                }),
            );
            add("serverId", string("已发现的下游 server ID。"));
            add("toolName", string("mcp.list(serverId) 返回的下游工具名。"));
            add(
                "arguments",
                json!({
                    "type": "object",
                    "description": "JSON 参数；序列化≤256 KiB。",
                    "default": {}
                }),
            );
            add(
                "waitSeconds",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 30,
                    "default": 5,
                    "description": "内联等待。",
                }),
            );
            add(
                "timeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 900,
                    "default": 300,
                    "description": "获准占槽后连接/请求时限（秒）；不含确认/排队。",
                }),
            );
        }
        "bootstrap.read" => add(
            "id",
            string("bootstrap 返回的 guide ID；不是任意文件路径。"),
        ),
        "skills.list" => {
            add("query", string("不区分大小写；空值等同未提供。"));
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"上限；无查询不截断，有查询缺省20。"}),
            );
            add(
                "activeOnly",
                json!({"type":"boolean","default":false,"description":"仅 active=true；不停止运行进程。"}),
            );
        }
        "skills.setActive" => {
            add("id", string("要设置激活状态的本地 skill ID。"));
            add(
                "active",
                json!({"type":"boolean","default":false,"description":"true 激活、false 停用；只改记录，不执行/授权。"}),
            );
        }
        "skills.read" => {
            add("id", string("本地 skill ID。"));
            add("path", string("可选包内相对路径；省略仅返回详情。"));
        }
        "skills.search" => {
            add(
                "query",
                string("非空、不区分大小写；匹配 ID、SKILL.md 正文/frontmatter、tags，不扫描其他包文件。"),
            );
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"匹配数上限；缺省 20。"}),
            );
        }
        "skills.activate" | "skills.deactivate" => add("id", string("本地 skill ID。")),
        "skills.install" => {
            add("id", string("安装后的 skill ID；须可用且非保留。"));
            add(
                "source",
                json!({
                    "type":"object",
                    "required":["type"],
                    "additionalProperties":false,
                    "description":"github 需 repository/url 恰一；files 需 files；均校验路径限制。",
                    "properties":{
                        "type":{"type":"string","enum":["github","files"],"description":"GitHub 仓库或显式 files 列表。"},
                        "repository":{"type":"string","description":"GitHub owner/repo。"},
                        "url":{"type":"string","description":"HTTPS github.com 仓库或 tree/blob URL。"},
                        "ref":{"type":"string","description":"分支/标签/提交；优先于 URL ref。"},
                        "path":{"type":"string","description":"仓库内目录或单文件；优先于 URL 子路径。"},
                        "files":{
                            "type":"array",
                            "minItems":1,
                            "description":"路径唯一且相对包根；每项 url/content/contentBase64 恰填一项。",
                            "items":{
                                "type":"object",
                                "required":["path"],
                                "additionalProperties":false,
                                "properties":{
                                    "path":{"type":"string","description":"包内相对目标路径。"},
                                    "url":{"type":"string","description":"HTTPS 文件地址。"},
                                    "content":{"type":"string","description":"内联 UTF-8 文本。"},
                                    "contentBase64":{"type":"string","description":"内联 base64 文件字节。"},
                                    "sha256":{"type":"string","description":"期望 SHA-256；不匹配则失败。"},
                                    "executable":{"type":"boolean","default":false,"description":"安装后是否可执行。"}
                                }
                            }
                        }
                    }
                }),
            );
            add(
                "replaceExisting",
                json!({"type":"boolean","default":false,"description":"已存在则归档再替换；false 拒绝。"}),
            );
            add(
                "activateAfterInstall",
                json!({"type":"boolean","description":"true 强制激活；false 保持默认（新建激活、替换保留原状态）。"}),
            );
            add(
                "idempotencyKey",
                json!({"type":"string","minLength":1,"maxLength":128,"description":"仅记录保留期间同 key 同请求复用/不同请求冲突；终态记录最多7天、最近100条。"}),
            );
        }
        "skills.install.get" => {
            add("installId", string("安装任务 ID。"));
            add("waitSeconds", wait_seconds_schema(5, "等待状态变化。"));
        }
        "skills.install.cancel" => add("installId", string("安装任务 ID。")),
        "skills.run" => {
            add("id", string("须已激活且可运行的本地 workspace skill ID。"));
            add(
                "path",
                string("scripts/ 下可执行文件相对路径；拒绝 symlink/目录。"),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选 process.list 分组键。",
                }),
            );
            add("args", strings("程序参数；省略为空数组。"));
            add(
                "workingDirectory",
                string("可选工作目录；按执行目录策略解析。"),
            );
            add(
                "waitSeconds",
                wait_seconds_schema(5, "启动后等待终态的秒数；不取消进程。"),
            );
        }
        "room.maintenance.status" => {}
        "room.maintenance.submit" => {
            add(
                "items",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 5,
                    "description": "各提交槽唯一且仅写空槽；全组先校验，不覆盖已占用内容。",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["slot", "payload"],
                        "properties": {
                            "slot": {
                                "type": "string",
                                "enum": ["diary.daily", "diary.weekly", "diary.monthly", "notebook", "entity"],
                                "description": "diary.* 写对应 current.md；notebook/entity 写 payload 指定文档。"
                            },
                            "payload": {
                                "description": "diary.*: summary 可省略（空串、≤8 Ki），entries 可省略（空数组、≤128 项），每项必填 text（≤8 Ki）、可选 tags（≤8 个、每个≤64 字符）；notebook: 必填 path（Notebook/ 下 .md、≤240）和 body（≤64 Ki），可选 title（空串、≤512）；entity: 必填单文件名 entity（≤160）和 content（≤64 Ki）。每个 payload 序列化≤64 KiB；格式错误拒绝整批。",
                            }
                        }
                    }
                }),
            );
            add(
                "mode",
                json!({
                    "type": "string",
                    "enum": ["local", "workflow"],
                    "description": "覆盖配置执行模式；local 生成文档并提交本地 commit，workflow 提交请求并由配置 workflow 处理。",
                }),
            );
            add(
                "waitSeconds",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 30,
                    "default": 0,
                    "description": "workflow 模式等待消费/结果；local 不使用，不取消工作流。",
                }),
            );
        }
        "room.diary.active" | "room.state.list" => {}
        "room.diary.read" => {
            add(
                "layer",
                json!({
                    "type": "string",
                    "enum": ["daily", "weekly", "monthly"],
                    "description": "选择日记目录层级。",
                }),
            );
            add(
                "period",
                json!({
                    "type": "string",
                    "pattern": "^(current|\\d{4}-\\d{2}-\\d{2}(--\\d{4}-\\d{2}-\\d{2})?)$",
                    "description": "current 读该层 current.md；daily 也接受日期，weekly/monthly 接受日期区间。日期须有效且起始≤结束，不校验周/月跨度。",
                }),
            );
        }
        "room.notebook.recent" => {
            add(
                "limit",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "最多返回的近期 Notebook Markdown 预览数；缺省 20。",
                }),
            );
        }
        "room.notebook.search" => {
            add(
                "query",
                string("不区分大小写子串；搜 Notebook 相对路径、首个 H1 和正文，非空且≤256 字符。"),
            );
            add(
                "limit",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "最多返回的 Notebook 预览数；缺省 20。",
                }),
            );
        }
        "room.notebook.read" => {
            add(
                "path",
                string("精确的 Notebook 相对 Markdown 路径；仅接受 Notebook/ 下 .md 文件。"),
            );
        }
        "room.state.read" => {
            add(
                "entity",
                string("State/entities/ 下单个文件名 stem（不含 .md）；不可含路径分隔符，先用 room.state.list 发现。"),
            );
        }
        _ => {}
    }
    properties
}

fn schema(properties: Map<String, Value>, required: &[&str]) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("type".to_string(), Value::String("object".to_string()));
    result.insert("properties".to_string(), Value::Object(properties));
    result.insert(
        "required".to_string(),
        Value::Array(
            required
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
    result.insert("additionalProperties".to_string(), Value::Bool(false));
    result
}

fn output_schema() -> Map<String, Value> {
    Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("additionalProperties".to_string(), Value::Bool(true)),
    ])
}

fn tool_description(name: &str) -> String {
    match name {
        "agent.info" => "查看本地 Agent 的身份/profile、工作区与路径策略、进程/MCP 容量、确认通道、连接、配置和健康状态；只读，不执行操作。".to_string(),
        "browser.manual" => "需要 Browser SDK 用法或不确定 API 时，先读/搜当前 runtime 附带的官方文档；返回路径、行号、内容或匹配及截断/跳过统计。路径、运行时或文档错误以 error 返回；只读、不联网。".to_string(),
        "browser.acquire" => "创建或复用命名 Browser lease/kernel，再用同名 browser.repl 连续操作；返回 ready、版本和 channel，初始化失败返回 error。复用会更新空闲时限，闲置后自动清理。".to_string(),
        "browser.repl" => "在已 acquire 的持久 lease 中执行任意 JavaScript；需要 API 细节时先查 browser.manual。返回 SDK content/structured result 与 isError；租约或执行错误返回 error。代码可读写网页及外部服务；超时不代表网页副作用已撤销。".to_string(),
        "browser.reset" => "恢复指定 lease：重置 JS 状态并重新初始化 Browser SDK，保留 lease 名称；返回 ready 或 error。不是每次调用后的清理，也不会撤销网页上已经发生的操作。".to_string(),
        "browser.release" => "Browser 工作结束后最终清理指定 lease；关闭 kernel 并移除名称，返回 released；清理错误返回 error。不会回滚网页已发生的副作用。".to_string(),
        "browser.list" => "查看 Browser runtime 与 lease；返回 runtimeAvailable、版本/channel 及 lease 的名称、状态、空闲时限/剩余时间；不可用时返回未就绪状态。只读。".to_string(),
        "file.read" => "按文件路径策略读取 UTF-8 文件或目录元数据；单次 path 与 requests 批次二选一。文本返回 content/可选 metadata/nextStartLine；PNG/JPEG/WebP 及 GIF 以 image content blocks 返回，GIF 为采样 PNG 帧并带帧元数据。批次按序逐项给状态/错误，单项失败不阻断其他项；目录须请求 metadata，图片不支持行范围。只读。".to_string(),
        "file.search" => "在允许的工作区路径搜索文本，支持字面/正则、glob 和有序批次；返回匹配位置/上下文及跳过、裁剪状态，批次逐项标明结果或错误。空查询、无效正则/glob 或路径会失败；上下文可能按配置裁剪。只读。".to_string(),
        "file.edit" => "用 Codex apply_patch 更新、新增、删除或移动工作区文件；成功返回 status/changed/changes（path/action/可选 destination），部分失败保留已完成项和错误，不公开 diff/revision。无效补丁、路径策略、确认拒绝或并发冲突会报错；有效操作会写盘，needConfirm 可请求一次确认。".to_string(),
        "process.exec" => "在指定 Agent 上以普通非登录/非交互 Bash 原样执行 command（pipefail 生效、不设 set -e）；cwd 在可信初始化后生效。策略只分析提交的 command，不检查可信初始化中的 PATH/函数；不是运行时安全边界。返回紧凑进程状态；waitSeconds 只等待、不取消，可用 process.read/cancel 跟进。".to_string(),
        "process.batch" => "在一次准入边界内按序执行原文 Bash 命令；元素 cwd 覆盖批次 cwd。返回批次及逐项进程状态；waitSeconds 只等待不取消，已启动副作用不会回滚。".to_string(),
        "process.read" => "按 processId 读取状态与一页产物，不启动或取消工作。返回 agentId/processId/kind/state/captureStatus，及预算内的 output 或完整下游 mcpResult。view=auto 在有未读输出时立即返回，否则有界等待；view=status 只等执行终态或期限，不返回正文。command/skill 用 cursor 续读；kind=mcp 和 status view 不接受日志 cursor。hasMore 只表示可续读，不要求读完日志；gap 表示已丢失字节，captureStatus=incomplete 时不要无限等待 EOF。MCP 结果状态 pending/deferred 不代表失败；deferred 可提高 maxBytes 领取，not_retained 不可恢复。waitSeconds 默认5、最大30、0立即，超时不取消；maxBytes 省略用 limits.processResponseBytes（出厂8192），显式范围4096..=1048576。预算计 Process JSON 主体，不含传输封套和独立事件面板。未知ID或非法参数返回结构化错误。".to_string(),
        "process.list" => "按 group、kind、state 分页发现进程；返回进程 ID、类型、状态、时间、捕获状态及 nextCursor。游标错误会失败；只读，不读取输出正文。".to_string(),
        "process.cancel" => "向进程所有者请求取消；返回当前 state、cancelOutcome、terminationEvidence 和可选 error。MCP 通过下游取消通知，返回请求/通知不证明远端副作用已停止。".to_string(),
        "mcp.callTool" => "调用已发现的下游 MCP 工具并登记为受管理进程；返回统一进程观察和预算内可用结果/错误，后续用 process.read 查看或 process.cancel 请求取消。下游可能产生外部副作用；waitSeconds 不取消；timeoutSeconds 仅在获准并取得执行槽后限制连接/请求（不含确认/排队），取消须另行请求。".to_string(),
        "tmux.listSessions" => "列出目标 Agent 的持久 tmux sessions；调用时提供 agentId。返回 sessions 列表；tmux server 未运行时为空列表，其他错误返回 error。只读。".to_string(),
        "tmux.sessions" => "本地 session 合并入口：list 只传 action；create 需 name/cwd；close 需 name，可选 needConfirm（缺省 true）。返回列表或 session/created 结果；close 会结束 session 内任务，拒绝或 tmux 错误返回 error。".to_string(),
        "tmux.listPanes" => "列出 agentId 指定 Agent 的 tmux panes，可选 session 限定范围；返回 panes 元数据。只读；无效 session 或远端错误返回 error。".to_string(),
        "tmux.panes" => "本地 pane 合并入口：list 可选 session，capture 需 target、可选 lines；分别返回 panes 或有界 capture 文本。只观察持久终端状态，不提交命令；目标错误返回 error。".to_string(),
        "tmux.capturePane" => "抓取 agentId 指定 Agent 的一个 pane 历史；返回 capture 文本，lines 缺省 160 并受服务端上限约束。只读；无效 target 或 tmux 错误返回 error。".to_string(),
        "tmux.pasteText" => "向非 shell pane/TUI 写入 text；shell pane 会拒绝并应改用 tmux.exec。submit=true 会追加 Enter，可能触发界面动作；成功返回 status=completed。needConfirm 缺省 true，确认拒绝或写入失败返回 error。".to_string(),
        "tmux.exec" => "把结构化 program/args 提交到本地 shell pane；返回 submitted 及可选输出快照/warning，不代表命令已完成或成功。策略可拒绝/要求确认；命令会在 pane 中真实执行。".to_string(),
        "tmux.createSession" => "在策略允许的 cwd 创建或复用 agentId 指定 Agent 的持久 tmux session；返回 session、cwd、created。目录或 tmux 错误返回 error；创建后 session 持续存在。".to_string(),
        "tmux.closeSession" => "关闭 agentId 指定 Agent 的持久 session，会结束其中的 pane/任务；needConfirm 缺省 true。返回关闭结果或确认/执行错误；仅在确需终止时调用。".to_string(),
        "mcp.listServers" => "列出已配置的下游 MCP servers，可选 agentId 指定目标；返回 id、enabled、transport 和 url 摘要。只读发现；目标或远端错误返回 error。".to_string(),
        "mcp.listTools" => "用 agentId 与 serverId 查询下游 MCP server 暴露的工具定义；返回其原始 tools/schema，便于选择 toolName 和构造 arguments。连接、配置或远端错误返回 error；只读。".to_string(),
        "mcp.list" => "本地合并发现入口：省略 serverId 列出已配置服务器，提供时连接该 server 并列出 tools；返回 servers 或原始工具定义。配置/连接错误返回 error；只读。".to_string(),
        "mcp.batch" => "把 1..=16 个下游 MCP 调用作为受管理子进程批量启动；返回批次状态、错误和各项统一进程观察。failFast 只跳过未启动项，已启动副作用不回滚；waitSeconds 仅内联等待，用 process.read 查看子进程观察，取消另用 process.cancel；timeoutSeconds 是获准并取得执行槽后的连接/请求期限（不含确认/排队）。".to_string(),
        "bootstrap" => "加载 Room bootstrap 索引与引导摘要；返回 schemaVersion、revision、entrypoint、guide 列表/数量和 warnings。只读，不是通用文件读取器。".to_string(),
        "bootstrap.read" => "用 bootstrap 返回的 guide ID 读取一份引导文档；返回 guide 摘要、frontmatter、resource 路径/编码/内容和 warnings。未知 ID 或读取错误返回 error；只读。".to_string(),
        "skills.list" => "发现本地 skills（含只读内置 skill-installer，可查看不可运行），返回摘要与 warnings；非空 query 不区分大小写检索，activeOnly 只保留 active 项。空 query 等同未查询；不修改技能或运行状态。".to_string(),
        "skills.setActive" => "本地激活状态合并入口：active=true 验证并激活，false 停用；返回 active/changed。仅改激活记录，不授权或启动/取消进程；错误返回 error。".to_string(),
        "skills.read" => "读取本地 skill 详情（含只读内置 skill-installer，可查看不可运行），可附 package-relative path 读取一个资源；返回 skill 元数据及可选 resource 的路径/编码/内容。不是通用文件读取；ID、资源或大小错误返回 error。".to_string(),
        "skills.search" => "按不区分大小写的 query 搜索本地 skill ID、SKILL.md 正文/frontmatter 和 tags；含只读内置 skill-installer（可查看不可运行），不扫描其他包文件。返回匹配摘要与 warnings，可用 limit 截断；空查询或扫描错误返回 error。".to_string(),
        "skills.active" => "检查激活记录；返回 activeSkills 的激活时间、active/missing 状态、stale 标记和可用摘要，并保留 warnings。缺失技能不会隐藏；只读。".to_string(),
        "skills.activate" => "激活一个存在且有效的本地 skill；返回 id、active、changed 和激活时间。只写激活状态，不执行技能或授予额外权限；无效/不存在 ID 返回 error。".to_string(),
        "skills.deactivate" => "停用本地 skill 并返回 active=false/changed；只移除激活状态，不取消已经运行的技能进程。无效 ID 或状态写入失败返回 error。".to_string(),
        "skills.install" => "从 GitHub 或显式文件源启动异步安装；返回 installId、状态、queued/deduplicated 和 pollAfterMs。会下载并写入/替换本地技能，替换旧包会归档；用 get/cancel 跟进。启动前错误直接返回 code/message；后台失败含 retryable，phase 可缺省。".to_string(),
        "skills.install.get" => "查询安装任务状态，可短暂等待；返回 status、可用 phase、attempt/progress/source 及终态 result/error。等待不取消安装；未知 installId 返回 error。".to_string(),
        "skills.install.cancel" => "请求协作取消安装并返回 outcome/status/phase；排队任务可立即取消，提交或激活阶段可能返回 tooLate，已完成写入不会被此请求撤销。".to_string(),
        "skills.run" => "运行已激活、可写本地 skill 的 scripts/ 下可执行文件，作为受管理进程返回统一紧凑观察；后续用 process.read 查看或 process.cancel 请求取消。脚本可产生真实副作用；inactive、路径或可执行性错误会拒绝。".to_string(),
        "room.maintenance.status" => "检查 Room 仓库、schema/scaffold、本地执行器、配置模式、workflow/remote/sync heads 和五个槽位占用；返回各 readiness/status 字段。只读；仓库不可检查时返回 error。".to_string(),
        "room.maintenance.submit" => "按槽位 payload 更新 Room 日记、Notebook 或实体；local 写入目标文档并提交本地 commit，workflow 提交请求并可能等待消费。返回 mode/state/localApplied/sync/revision；不会覆盖已占用槽，仓库未就绪、重复槽或 payload 错误会拒绝。".to_string(),
        "room.diary.active" => "读取 daily/weekly/monthly 三个 current.md；每层返回 period/path/available 与 content 或 missing、unreadable、invalid_utf8 issue。语义化只读，不接受任意路径。".to_string(),
        "room.diary.read" => "按 layer/period 读取精确日记文档；返回 document 的路径、可用状态、内容或 issue。daily 接 current/日期，weekly/monthly 接日期范围；非法日期/范围报错，缺失文档以 issue 表示。".to_string(),
        "room.notebook.recent" => "按有效时间列出最近 Notebook Markdown 预览；返回 documents（path/title/contentPreview/truncated/effectiveAt）和 warnings。缺少 Notebook 时为空结果；只读。".to_string(),
        "room.notebook.search" => "以不区分大小写的子串搜索 Notebook 路径、首个 H1 和正文；返回有界预览及 warnings。query 空或过长报错；只读，不接受正则。".to_string(),
        "room.notebook.read" => "读取 Notebook/ 下精确的 .md 相对路径；返回 path/content。未知或超大文档返回错误；只读，不是任意仓库文件读取器。".to_string(),
        "room.state.list" => "列出 State/entities/ 下的 Markdown 实体；返回按路径排序的 entities（entity/path），跳过 symlink 和非 Markdown 文件。只读。".to_string(),
        "room.state.read" => "按实体文件名 stem 读取 State/entities/{entity}.md；返回 path/content。缺失或超大文档报错；不接受路径输入，只读。".to_string(),
        "event.list" => "按状态、等级和不透明游标分页查看当前 Agent 的事件；默认列 pending、每页20条，隐藏事件仍包含在列表/计数中。返回 items/nextCursor 与本次紧凑 events 面板；非法游标或目标 Agent 不匹配时返回错误。只读，不读取进程状态。".to_string(),
        "event.get" => "按 eventId 读取完整事件记录和当前 Agent 的紧凑 events 面板；读取不标记 handled，也不操作进程或安装。未知 ID 或目标 Agent 不匹配时返回错误。".to_string(),
        "event.mark" => "将 eventIds 对应事件幂等标记为 handled，并返回 handledIds/notFoundIds 与紧凑 events 面板；不清理历史、不操作进程或安装。".to_string(),
        "privateevent.inject" => "本地集成专用事件注入；仅 LocalUnix ingress 可直接调用，不在 tools/list 中公开。来源固定为 external，调用方只提供 ref；此注入确认不消费事件面板曝光。".to_string(),

        _ => "未知本地工具；请使用已列出的工具名，输入和结果由对应工具合同定义。".to_string(),
    }
}
