use std::sync::Arc;

use rmcp::model::{Meta, Tool, ToolAnnotations};
use serde_json::{json, Map, Value};

use crate::{
    config::ToolsetConfig,
    operation::{
        tool_is_destructive, tool_is_open_world, tool_is_read_only, TOOL_NAMESPACE_BY_NAME,
    },
};

const PATCH_SCHEMA_DESCRIPTION: &str = "Codex apply_patch 补丁文本：以 *** Begin Patch 开始、以 *** End Patch 结束；支持跨文件新增、删除、更新和移动文件。";

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
        "process.exec" => &["program"],
        "browser.manual" => &["action"],
        "browser.acquire" => &["name", "idleTimeoutSeconds"],
        "browser.repl" => &["name", "code"],
        "browser.reset" | "browser.release" => &["name"],
        "browser.list" => &[],
        "process.status" | "process.cancel" | "process.output" | "process.result" => &["processId"],
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
        add(
            "agentId",
            string("目标本地 Agent ID；须与所选 worker 匹配。"),
        );
    }
    match name {
        "browser.manual" => {
            add(
                "action",
                json!({
                    "type": "string",
                    "enum": ["read", "search"],
                    "description": "read 读取单个官方文档文件；search 在当前 Browser runtime 附带的文档中查找。两种 action 的字段互斥。",
                }),
            );
            add(
                "path",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "read 使用的文档相对路径；不接受绝对路径或越出文档根目录的路径。",
                }),
            );
            add(
                "startLine",
                json!({"type":"integer","minimum":1,"description":"read 的起始行（含）；可单独指定。"}),
            );
            add(
                "endLine",
                json!({"type":"integer","minimum":1,"description":"read 的结束行（含）；可单独指定。"}),
            );
            add(
                "query",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4096,
                    "description": "search 的区分大小写子串；非空，read 时不得提供。",
                }),
            );
            add(
                "maxResults",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "default": 50,
                    "description": "search 最多返回的匹配数。",
                }),
            );
            add(
                "contextLines",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 5,
                    "default": 2,
                    "description": "每个 search 匹配前后各带的上下文行数。",
                }),
            );
        }
        "browser.acquire" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "调用方选择的持久 lease 名称；须非空白，按名称复用。",
                }),
            );
            add(
                "idleTimeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "lease 空闲回收时限（秒）；复用已有名称时更新时限，超时会清理 lease。",
                }),
            );
        }
        "browser.repl" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "已由 browser.acquire 建立的持久 Browser lease 名称。",
                }),
            );
            add(
                "code",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 262144,
                    "description": "在 lease 中执行的任意 JavaScript（UTF-8 字节数受限）；可通过 Browser SDK 读写网页并产生外部副作用。",
                }),
            );
            add(
                "timeoutMs",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 120000,
                    "default": 20000,
                    "description": "本次 JavaScript 调用的超时（毫秒）；超时不表示网页操作已撤销。",
                }),
            );
            add(
                "title",
                json!({
                    "type": "string",
                    "maxLength": 128,
                    "description": "可选观测标题；只用于记录，不注入 JavaScript。",
                }),
            );
        }
        "browser.reset" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "待恢复的持久 lease 名称；重置会清除 REPL 状态并重新初始化 Browser SDK，但不撤销已发生的网页操作。",
                }),
            );
        }
        "browser.release" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "待最终清理的持久 lease 名称；关闭 kernel 并移除 lease，不回滚网页已发生的操作。",
                }),
            );
        }
        "browser.list" => {}
        "file.read" => {
            add(
                "path",
                string(
                    "工作区内文件或目录路径；按文件路径策略解析。目录仅在 metadata=true 时可读。",
                ),
            );
            add(
                "metadata",
                boolean("返回 type、sizeBytes、modifiedAt 等文件信息；缺省 false。目录只返回 metadata。"),
            );
            add(
                "startLine",
                number("文本读取起始行（含）；必须大于 0，图片不支持行范围。"),
            );
            add(
                "endLine",
                number("文本读取结束行（含）；必须不小于 startLine，图片不支持行范围。"),
            );
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("工作区内文件或目录路径；目录读取须设 metadata=true。"),
                        "metadata":boolean("是否返回文件信息；缺省 false。"),
                        "startLine":number("文本起始行（含）；图片不支持行范围。"), "endLine":number("文本结束行（含）；图片不支持行范围。")
                    },"required":["path"]},
                    "description":"按数组顺序读取；每项须有 path，最多 32 项。与顶层 path/metadata/startLine/endLine 互斥；失败按项返回，不影响其他项。"
                }),
            );
        }
        "file.search" => {
            add("path", string("工作区内搜索根路径；按只读路径策略解析。"));
            add("query", string("待匹配的非空文本或正则表达式。"));
            add(
                "mode",
                json!({"type":"string","enum":["literal","regex"],"default":"literal","description":"literal 按字面文本匹配；regex 按正则匹配。"}),
            );
            add("caseSensitive", boolean("是否区分大小写；缺省 true。"));
            add(
                "include",
                strings("匹配文件相对路径的纳入 glob；最多 16 个。"),
            );
            add(
                "exclude",
                strings("匹配文件相对路径的排除 glob；最多 16 个。"),
            );
            add(
                "contextLines",
                json!({
                    "type":"integer",
                    "minimum":0,
                    "description":"请求的匹配上下文行数；缺省 0，超出当前配置上限时会裁剪并报告 warning。",
                    "default":0
                }),
            );
            add(
                "maxResults",
                number("最多返回的匹配数；省略时为 50，最大 200。"),
            );
            add("hidden", boolean("是否搜索隐藏文件；缺省 false。"));
            add(
                "respectGitignore",
                boolean("是否遵循仓库内 Git ignore 规则；缺省 true。"),
            );
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("工作区内搜索根路径。"),
                        "query":string("非空文本或正则表达式。"),
                        "mode":{"type":"string","enum":["literal","regex"],"default":"literal","description":"literal 按字面文本匹配；regex 按正则匹配。"},
                        "caseSensitive":{"type":"boolean","default":true,"description":"是否区分大小写；缺省 true。"},
                        "include":strings("纳入 glob；最多 16 个。"), "exclude":strings("排除 glob；最多 16 个。"),
                        "contextLines":{"type":"integer","minimum":0,"default":0,"description":"匹配前后各带的上下文行数；缺省 0，受当前配置上限约束。"},
                        "maxResults":{"type":"integer","maximum":200,"default":50,"description":"最多返回的匹配数；缺省 50。"},
                        "hidden":{"type":"boolean","default":false,"description":"是否搜索隐藏文件；缺省 false。"},
                        "respectGitignore":{"type":"boolean","default":true,"description":"是否遵循 Git ignore 规则；缺省 true。"}
                    },"required":["path","query"]},
                    "description":"按数组顺序独立搜索；每项须有 path 和 query，最多 32 项。与任一顶层单次搜索字段互斥；结果保留索引及各项状态。"
                }),
            );
        }
        "file.edit" => {
            add("patch", string(PATCH_SCHEMA_DESCRIPTION));
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"请求在产生实际文件变更前进行一次确认；不替代运行时策略要求的确认。"}),
            );
        }
        "mcp.list" => add(
            "serverId",
            string("可选的已配置 MCP server ID；省略时列出服务器，提供时查询该服务器的工具。"),
        ),
        "process.exec" => {
            add(
                "program",
                string("要启动的可执行文件名或路径；按执行策略检查。"),
            );
            add(
                "args",
                strings("直接传给 program 的参数数组；不是拼接后的 shell 命令。"),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选进程分组键；按去除首尾空白后的精确值筛选或归组。",
                }),
            );
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"请求执行前确认；实际策略仍可要求确认。"}),
            );
            add(
                "workingDirectory",
                string("可选工作目录；省略时使用运行配置默认目录，并按路径策略解析。"),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"启动后等待终态的时间（秒）；仅延迟本次响应，不会取消进程。"}),
            );
        }
        "process.batch" => {
            add(
                "elements",
                json!({
                    "type": "array",
                    "description": "有序子进程请求；每项指定 program，可选 args 和 workingDirectory；元素工作目录覆盖批次默认目录。",
                    "items": {"type": "object", "properties": {
                        "program": string("该子进程的可执行文件名或路径。"),
                        "args": strings("直接传给 program 的参数数组；省略时为空数组。"),
                        "workingDirectory": string("可选子进程工作目录；覆盖批次默认目录。")
                    }, "required": ["program"], "additionalProperties": false}
                }),
            );
            add(
                "needConfirm",
                boolean("请求批次执行前确认；缺省 false，实际策略仍可要求确认。"),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选分组键，子进程继承此分组。",
                }),
            );
            add(
                "workingDirectory",
                string("批次默认工作目录；可由单个元素的 workingDirectory 覆盖。"),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"等待子进程终态的时间（秒）；只决定本次响应等待，不取消已启动任务。"}),
            );
        }
        "process.status" => {
            add(
                "processId",
                string(
                    "由 process.exec、process.batch、mcp.callTool 或 skills.run 返回的进程 ID。",
                ),
            );
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "在终端状态前最多等待的秒数；缺省 5，最多 30；只轮询，不取消进程。",
                ),
            );
        }
        "process.cancel" => {
            add("processId", string("待请求取消的受管理进程 ID。"));
        }
        "process.list" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "按去除首尾空白后的分组值精确筛选。",
                }),
            );
            add(
                "kind",
                json!({"type":"string","enum":["command","skill","mcp"],"description":"只列出指定来源类型的进程。"}),
            );
            add(
                "state",
                json!({"type":"string","enum":["queued","waiting_confirmation","starting","running","completed","failed","rejected","cancel_requested","cancelled","timed_out","detached","unknown_after_restart","skipped"],"description":"只列出指定生命周期状态的进程。"}),
            );
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"default":50,"description":"每页最多返回的进程数；缺省 50。"}),
            );
            add(
                "cursor",
                string("process.list 返回的 opaque 游标；续取下一页时原样传回。"),
            );
        }
        "process.output" => {
            add("processId", string("要读取输出的受管理进程 ID。"));
            add(
                "cursor",
                string("上次 process.output 返回的字节偏移游标；首次读取时省略。"),
            );
            add(
                "maxBytes",
                json!({"type":"integer","minimum":1,"maximum":32768,"default":8192,"description":"本页 stdout 与 stderr 合计最多读取的编码字节数；缺省 8192。"}),
            );
        }
        "process.result" => {
            add("processId", string("要获取保留 MCP JSON 结果的进程 ID。"));
            add(
                "maxBytes",
                json!({"type":"integer","minimum":1,"maximum":524288,"default":8192,"description":"允许返回的结果最大字节数；缺省 8192，过大时只报告状态/预览，不返回部分 JSON。"}),
            );
        }
        "tmux.sessions" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "create", "close"], "description":"list 只接受 action；create 还须 name、cwd；close 还须 name，可选 needConfirm（缺省 true）。"}),
            );
            add(
                "name",
                string("create/close 的 tmux session 名；其余 action 不接受此字段。"),
            );
            add(
                "cwd",
                string("create 的工作目录；其余 action 不接受此字段。"),
            );
            add(
                "needConfirm",
                boolean("close 前是否请求确认；缺省 true。list/create 不接受此字段。"),
            );
        }
        "tmux.panes" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "capture"], "description":"list 可选 session；capture 须 target、可选 lines；两种 action 的字段互斥。"}),
            );
            add(
                "session",
                string("list 限定的 tmux session 名；capture 时不可提供。"),
            );
            add(
                "target",
                string("capture 使用的 tmux pane target；list 时不可提供。"),
            );
            add(
                "lines",
                number("capture 要取回的历史行数；缺省 160，服务端限制在 1..=5000。"),
            );
        }
        "tmux.listPanes" => add(
            "session",
            string("可选的 tmux session 名；省略时列出所有 session 的 panes。"),
        ),
        "tmux.capturePane" => {
            add(
                "target",
                string("远程目标 Agent 上要抓取的 tmux pane target。"),
            );
            add(
                "lines",
                number("返回的历史行数；缺省 160，服务端限制在 1..=5000。"),
            );
        }
        "tmux.pasteText" => {
            add(
                "target",
                string("非 shell tmux pane 或 TUI 的 target；shell pane 会被拒绝。"),
            );
            add("text", string("写入 pane 的文本。"));
            add("submit", boolean("是否在文本后追加 Enter；缺省 false。"));
            add("needConfirm", boolean("写入前是否请求确认；缺省 true。"));
        }
        "tmux.exec" => {
            add(
                "target",
                string("必须是存活且不在 tmux copy mode 中的 shell pane target。"),
            );
            add(
                "program",
                string("要提交到 shell 的程序名或 builtin；按执行策略预检。"),
            );
            add("args", strings("传给 program 的结构化参数数组。"));
            add(
                "needConfirm",
                boolean("是否请求执行前确认；缺省 false，运行策略仍可能要求确认。"),
            );
            add(
                "waitMs",
                number(
                    "提交后等待 pane 输出的时间（毫秒）；缺省 300，最多 5000，不代表命令已完成。",
                ),
            );
            add(
                "captureLines",
                number("提交后快照包含的历史行数；缺省 120，最多 5000。"),
            );
        }
        "tmux.createSession" => {
            add("name", string("新建或复用的 tmux session 名。"));
            add(
                "cwd",
                string("session 工作目录；必须符合当前执行目录策略。"),
            );
        }
        "tmux.closeSession" => {
            add("name", string("要关闭的 tmux session 名。"));
            add(
                "needConfirm",
                boolean("关闭前是否请求确认；缺省 true，关闭会结束该 session 中的任务。"),
            );
        }
        "mcp.listServers" => add(
            "agentId",
            string("要查询其已配置下游 MCP server 的目标 Agent ID。"),
        ),
        "mcp.listTools" => add(
            "serverId",
            string("目标 Agent 上已配置的下游 MCP server ID。"),
        ),
        "mcp.batch" => {
            add(
                "calls",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 16,
                    "description": "按数组顺序提交 1..=16 个下游调用；全部参数先验证再统一准入/确认，总序列化参数不超过 2 MiB，启动后的副作用不回滚。",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serverId", "toolName"],
                        "properties": {
                            "serverId": {"type": "string", "description": "已配置的下游 MCP server ID。"},
                            "toolName": {"type": "string", "description": "该 server 的 mcp.listTools 返回的工具名。"},
                            "arguments": {
                                "type": "object",
                                "default": {},
                                "description": "此调用的 JSON 参数对象；序列化后最多 256 KiB，省略时为空对象。"
                            }
                        }
                    }
                }),
            );
            add(
                "mode",
                json!({"type":"string","enum":["parallel","sequential"],"default":"parallel","description":"并行启动，或按序等待每个子调用进入终态后再启动下一项。"}),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选进程分组键，所有子调用继承此值。",
                }),
            );
            add(
                "failFast",
                json!({"type":"boolean","default":false,"description":"硬失败后跳过尚未启动的子调用；已启动调用不会因此取消。"}),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"等待子进程响应的时间（秒）；只影响本次内联等待，不取消下游调用。"}),
            );
            add(
                "timeoutSeconds",
                json!({"type":"integer","minimum":1,"maximum":900,"default":300,"description":"每个下游调用的确认、连接与请求总执行时限（秒）；不同于 waitSeconds。"}),
            );
        }
        "mcp.callTool" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选进程分组键；子调用继承此值。",
                }),
            );
            add(
                "serverId",
                string("已配置的下游 MCP server ID；先用 mcp.list 发现。"),
            );
            add("toolName", string("由 mcp.listTools 返回的下游工具名。"));
            add(
                "arguments",
                json!({
                    "type": "object",
                    "description": "下游工具的 JSON 参数对象；序列化后最多 256 KiB，省略时为空对象。",
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
                    "description": "等待内联进程响应的时间（秒）；缺省 5、最多 30；不取消调用。",
                }),
            );
            add(
                "timeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 900,
                    "default": 300,
                    "description": "下游确认、连接与执行的总时限（秒）；缺省 300、最多 900，独立于 waitSeconds。",
                }),
            );
        }
        "bootstrap.read" => add(
            "id",
            string("bootstrap 返回的 guide ID；不是任意文件路径。"),
        ),
        "skills.list" => {
            add(
                "query",
                string("可选的非空查询；不区分大小写，提供时按查询结果筛选。"),
            );
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"最多返回的技能数；无查询时省略表示不截断，查询时服务端缺省 20。"}),
            );
            add(
                "activeOnly",
                json!({"type":"boolean","default":false,"description":"只保留 active=true 的技能摘要；不会取消已运行进程。"}),
            );
        }
        "skills.setActive" => {
            add("id", string("要设置激活状态的本地 skill ID。"));
            add(
                "active",
                json!({"type":"boolean","default":false,"description":"true 激活，false 停用；仅修改激活记录，不执行技能或授予权限。"}),
            );
        }
        "skills.read" => {
            add("id", string("要读取的本地 skill ID。"));
            add(
                "path",
                string("可选的包内相对资源路径；省略时只返回技能详情。"),
            );
        }
        "skills.search" => {
            add(
                "query",
                string("非空、不区分大小写的查询；匹配本地 skill 元数据与包内容。"),
            );
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"最多返回的匹配技能数；缺省 20。"}),
            );
        }
        "skills.activate" | "skills.deactivate" => {
            add("id", string("要激活或停用的本地 skill ID。"))
        }
        "skills.install" => {
            add(
                "id",
                string("安装后技能包使用的目标 ID；必须是可用且非保留的 skill ID。"),
            );
            add(
                "source",
                json!({
                    "type":"object",
                    "required":["type"],
                    "additionalProperties":false,
                    "description":"按 type 区分的源描述。github 须且仅须 repository 或 url；files 须提供 files。两类来源都会校验路径和包限制。",
                    "properties":{
                        "type":{"type":"string","enum":["github","files"],"description":"github 从 GitHub 仓库下载；files 使用下方显式文件列表。"},
                        "repository":{"type":"string","description":"GitHub owner/repo；与 url 二选一。"},
                        "url":{"type":"string","description":"HTTPS github.com 仓库或 tree/blob 子路径 URL；与 repository 二选一。"},
                        "ref":{"type":"string","description":"github 的分支、标签或提交引用；优先于 url 中的 ref。"},
                        "path":{"type":"string","description":"github 仓库内子目录；优先于 url 中的子路径。"},
                        "files":{
                            "type":"array",
                            "minItems":1,
                            "description":"type=files 时的显式文件列表；路径须唯一且相对包根目录。",
                            "items":{
                                "type":"object",
                                "required":["path"],
                                "additionalProperties":false,
                                "properties":{
                                    "path":{"type":"string","description":"包内相对目标路径；每个文件都必填。"},
                                    "url":{"type":"string","description":"HTTPS 文件源；与 content、contentBase64 中恰好提供一个。"},
                                    "content":{"type":"string","description":"内联 UTF-8 文件内容；与 url、contentBase64 中恰好提供一个。"},
                                    "contentBase64":{"type":"string","description":"内联 base64 文件字节；与 url、content 中恰好提供一个。"},
                                    "sha256":{"type":"string","description":"可选期望 SHA-256；下载或解码内容不匹配时安装失败。"},
                                    "executable":{"type":"boolean","default":false,"description":"安装后是否将文件标记为可执行；缺省 false。"}
                                }
                            }
                        }
                    }
                }),
            );
            add(
                "replaceExisting",
                json!({"type":"boolean","default":false,"description":"目标 ID 已存在时先归档现有技能再替换；false 时拒绝覆盖。"}),
            );
            add(
                "activateAfterInstall",
                json!({"type":"boolean","description":"true 强制安装后激活；false 不抑制默认行为：新建技能仍默认激活，替换时保留原激活状态。"}),
            );
            add(
                "idempotencyKey",
                json!({"type":"string","minLength":1,"maxLength":128,"description":"同 key、同请求重试时复用原安装任务；同 key 不同请求报冲突。"}),
            );
        }
        "skills.install.get" => {
            add(
                "installId",
                string("skills.install 返回的异步安装任务 ID。"),
            );
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "查询状态前最多等待的秒数；缺省 5、最多 30；只是等待状态变化，不取消安装。",
                ),
            );
        }
        "skills.install.cancel" => add(
            "installId",
            string("要请求协作取消的安装任务 ID；进入提交/激活阶段后可能已太晚。"),
        ),
        "skills.run" => {
            add(
                "id",
                string("必须已激活且可运行的本地 workspace skill ID。"),
            );
            add(
                "path",
                string("包内 scripts/ 下的可执行文件相对路径；不接受 symlink 或目录。"),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "可选进程分组键；按 process.list 的 group 筛选。",
                }),
            );
            add(
                "args",
                strings("传给技能可执行文件的参数数组；省略时为空数组。"),
            );
            add(
                "workingDirectory",
                string("可选进程工作目录；按执行目录策略解析。"),
            );
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "启动后等待进程终态的时间（秒）；缺省 5、最多 30；不取消进程。",
                ),
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
                    "description": "一次提交 1..=5 个不同语义槽；全组先校验且只写空槽，不能覆盖已占用内容。",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["slot", "payload"],
                        "properties": {
                            "slot": {
                                "type": "string",
                                "enum": ["diary.daily", "diary.weekly", "diary.monthly", "notebook", "entity"],
                                "description": "目标语义槽；每项最多一个。diary.* 写对应 current.md，notebook 写指定 Notebook 文档，entity 写指定实体文档。"
                            },
                            "payload": {
                                "description": "diary.* 可含 summary（缺省空串、≤8 Ki 字符）和 entries（缺省空数组、≤128 项；每项仅含必填 text（≤8 Ki）与可选 tags（≤8 个、每个≤64 字符））；notebook 须 path（Notebook/ 下 .md、≤240 字符）和 body（必填、≤64 Ki），title 可选（缺省空串、≤512）；entity 须单文件名 entity（≤160）及必填 content（≤64 Ki）。每个 payload 序列化后≤64 KiB；格式错误拒绝整批。",
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
                    "description": "可选覆盖配置中的执行模式；local 在 Room 仓库生成文档并提交本地 commit，workflow 提交仓库请求并经配置的 workflow 处理。",
                }),
            );
            add(
                "waitSeconds",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 30,
                    "default": 0,
                    "description": "workflow 模式下等待消费/快速前移的秒数；缺省 0，最多 30；只等结果，不取消工作流。local 模式不使用此等待。",
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
                    "description": "选择 daily、weekly 或 monthly 日记目录。",
                }),
            );
            add(
                "period",
                json!({
                    "type": "string",
                    "pattern": "^(current|\\d{4}-\\d{2}-\\d{2}(--\\d{4}-\\d{2}-\\d{2})?)$",
                    "description": "current 读取该层 current.md；daily 也接受 YYYY-MM-DD；weekly/monthly 接受 YYYY-MM-DD--YYYY-MM-DD（两端为有效日期且起始不晚于结束，不强制周/月跨度）。",
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
                string("不区分大小写的子串；搜索 Notebook 相对路径、首个 H1 标题及正文，非空且至多 256 字符。"),
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
                string("从发现结果复制的精确 Notebook 相对 Markdown 路径；仅接受 Notebook/ 下的 .md 文件，不接受任意仓库路径。"),
            );
        }
        "room.state.read" => {
            add(
                "entity",
                string("State/entities/ 下的单个实体文件名 stem（不含 .md）；不得含路径分隔符，使用 room.state.list 发现有效项。"),
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
        "file.read" => "读取工作区 UTF-8 文件或目录元数据；单次 path 与 requests 批次二选一。文本返回 content/可选 metadata/nextStartLine；PNG/JPEG/WebP 及 GIF 以 image content blocks 返回，GIF 为采样 PNG 帧并带帧元数据。批次按序逐项给状态/错误，单项失败不阻断其他项；目录须请求 metadata，图片不支持行范围。只读。".to_string(),
        "file.search" => "在允许的工作区路径搜索文本，支持字面/正则、glob 和有序批次；返回匹配位置/上下文及跳过、裁剪状态，批次逐项标明结果或错误。空查询、无效正则/glob 或路径会失败；上下文可能按配置裁剪。只读。".to_string(),
        "file.edit" => "用 Codex apply_patch 补丁更新、新增、删除或移动一个/多个工作区文件；返回每文件变更、diff、行数和 revision。无效补丁、路径策略、确认拒绝或并发冲突会报错；有效操作会写盘，needConfirm 可请求一次确认。".to_string(),
        "process.exec" => "用 program 与参数数组启动一个本地受管理进程；不是把参数拼成 shell 命令。返回 processId、status、completedInline、进程信息及可用的内联输出/结果；用 status/output/result/cancel 跟进。执行受策略/确认控制并可产生真实副作用；waitSeconds 只等响应，不取消进程。".to_string(),
        "process.batch" => "在同一准入边界下启动一组有序子进程；返回 batchId、状态和各进程响应。失败时已启动的子进程及副作用不回滚；按子 processId 跟进。waitSeconds 只控制本次等待，不取消任务。".to_string(),
        "process.status" => "查询 processId 的状态与元数据（含生命周期、时间和执行信息），不返回输出/结果正文；返回 waitElapsedMs。可短暂轮询，未知或过期 ID 返回结构化错误；等待不等于取消。".to_string(),
        "process.list" => "按 group、kind、state 分页发现进程；返回进程 ID、类型、状态、时间、捕获状态及 nextCursor。游标错误会失败；只读，不读取输出正文。".to_string(),
        "process.output" => "按不透明字节游标读取 stdout/stderr 页；返回 data、encoding、起止 offset、nextCursor、hasMore/eof 和 captureStatus。分页不消费输出；未知进程或无效游标报错。".to_string(),
        "process.result" => "读取已保留的 MCP JSON 结果；返回 complete/too_large/unavailable 状态、resultAvailable、可用结果或错误及大小/hash/预览。超限时不返回部分 JSON；未知进程报错。".to_string(),
        "process.cancel" => "向进程所有者请求取消；返回当前 state、cancelOutcome、terminationEvidence 和可选 error。MCP 通过下游取消通知，返回请求/通知不证明远端副作用已停止。".to_string(),
        "tmux.listSessions" => "列出目标 Agent 的持久 tmux sessions；调用时提供 agentId。返回 sessions 列表；tmux server 未运行时为空列表，其他错误返回 error。只读。".to_string(),
        "tmux.sessions" => "本地 session 合并入口：list 只传 action；create 需 name/cwd；close 需 name，可选 needConfirm（缺省 true）。返回列表或 session/created 结果；close 会结束 session 内任务，拒绝或 tmux 错误返回 error。".to_string(),
        "tmux.listPanes" => "列出 agentId 指定 Agent 的 tmux panes，可选 session 限定范围；返回 panes 元数据。只读；无效 session 或远端错误返回 error。".to_string(),
        "tmux.panes" => "本地 pane 合并入口：list 可选 session，capture 需 target、可选 lines；分别返回 panes 或有界 capture 文本。只观察持久终端状态，不提交命令；目标错误返回 error。".to_string(),
        "tmux.capturePane" => "抓取 agentId 指定 Agent 的一个 pane 历史；返回 capture 文本，lines 缺省 160 并受服务端上限约束。只读；无效 target 或 tmux 错误返回 error。".to_string(),
        "tmux.pasteText" => "向非 shell pane/TUI 写入 text；shell pane 会拒绝并应改用 tmux.exec。submit=true 会追加 Enter，可能触发界面动作；返回 accepted/submitted。needConfirm 缺省 true，确认拒绝或写入失败返回 error。".to_string(),
        "tmux.exec" => "把结构化 program/args 提交到本地 shell pane；返回 submitted 及可选输出快照/warning，不代表命令已完成或成功。策略可拒绝/要求确认；命令会在 pane 中真实执行。".to_string(),
        "tmux.createSession" => "在策略允许的 cwd 创建或复用 agentId 指定 Agent 的持久 tmux session；返回 session、cwd、created。目录或 tmux 错误返回 error；创建后 session 持续存在。".to_string(),
        "tmux.closeSession" => "关闭 agentId 指定 Agent 的持久 session，会结束其中的 pane/任务；needConfirm 缺省 true。返回关闭结果或确认/执行错误；仅在确需终止时调用。".to_string(),
        "mcp.listServers" => "列出已配置的下游 MCP servers，可选 agentId 指定目标；返回 id、enabled、transport 和 url 摘要。只读发现；目标或远端错误返回 error。".to_string(),
        "mcp.listTools" => "用 agentId 与 serverId 查询下游 MCP server 暴露的工具定义；返回其原始 tools/schema，便于选择 toolName 和构造 arguments。连接、配置或远端错误返回 error；只读。".to_string(),
        "mcp.list" => "本地合并发现入口：省略 serverId 列出已配置服务器，提供时连接该 server 并列出 tools；返回 servers 或原始工具定义。配置/连接错误返回 error；只读。".to_string(),
        "mcp.batch" => "把 1..=16 个下游 MCP 调用作为受管理子进程批量启动；返回批次状态、错误和各项进程结果。failFast 只跳过未启动项，已启动副作用不回滚；waitSeconds 仅内联等待，timeoutSeconds 才是执行期限，取消另用 process.cancel。".to_string(),
        "mcp.callTool" => "调用已发现的下游 MCP 工具并登记为受管理进程；返回 processId/status/completedInline 与可用结果/错误，后续用 process 工具查询。下游可能产生外部副作用；waitSeconds 不取消，timeoutSeconds 是执行期限，取消须另行请求。".to_string(),
        "bootstrap" => "加载 Room bootstrap 索引与引导摘要；返回 schemaVersion、revision、entrypoint、guide 列表/数量和 warnings。只读，不是通用文件读取器。".to_string(),
        "bootstrap.read" => "用 bootstrap 返回的 guide ID 读取一份引导文档；返回 guide 摘要、frontmatter、resource 路径/编码/内容和 warnings。未知 ID 或读取错误返回 error；只读。".to_string(),
        "skills.list" => "发现本地 skills 并返回摘要与 warnings；非空 query 不区分大小写检索，activeOnly 只保留 active 项。空 query 等同未查询；不修改技能或运行状态。".to_string(),
        "skills.setActive" => "本地激活状态合并入口：active=true 验证并激活，false 停用；返回 active/changed。仅改激活记录，不授权或启动/取消进程；错误返回 error。".to_string(),
        "skills.read" => "读取本地 skill 详情，可附 package-relative path 读取一个资源；返回 skill 元数据及可选 resource 的路径/编码/内容。不是通用文件读取；ID、资源或大小错误返回 error。".to_string(),
        "skills.search" => "按不区分大小写的 query 搜索本地 skill 元数据和包内容；返回匹配摘要与 warnings，可用 limit 截断。只读；空查询或扫描错误返回 error。".to_string(),
        "skills.active" => "检查激活记录；返回 activeSkills 的激活时间、active/missing 状态、stale 标记和可用摘要，并保留 warnings。缺失技能不会隐藏；只读。".to_string(),
        "skills.activate" => "激活一个存在且有效的本地 skill；返回 id、active、changed 和激活时间。只写激活状态，不执行技能或授予额外权限；无效/不存在 ID 返回 error。".to_string(),
        "skills.deactivate" => "停用本地 skill 并返回 active=false/changed；只移除激活状态，不取消已经运行的技能进程。无效 ID 或状态写入失败返回 error。".to_string(),
        "skills.install" => "从 GitHub 或显式文件源启动异步安装；返回 installId、状态、queued/deduplicated 和 pollAfterMs。会下载并写入/替换本地技能，替换旧包会归档；用 get/cancel 跟进，安装错误含阶段和 retryable。".to_string(),
        "skills.install.get" => "查询安装任务状态，可短暂等待；返回 status/phase、attempt/progress/source 及终态 result/error。等待不取消安装；未知 installId 返回 error。".to_string(),
        "skills.install.cancel" => "请求协作取消安装并返回 outcome/status/phase；排队任务可立即取消，提交或激活阶段可能返回 tooLate，已完成写入不会被此请求撤销。".to_string(),
        "skills.run" => "运行已激活、可写本地 skill 的 scripts/ 下可执行文件，作为受管理进程返回 processId/status/completedInline 和输出/错误；用 process 工具跟进。脚本可产生真实副作用；inactive、路径或可执行性错误会拒绝。".to_string(),
        "room.maintenance.status" => "检查 Room 仓库、schema/scaffold、本地执行器、配置模式、workflow/remote/sync heads 和五个槽位占用；返回各 readiness/status 字段。只读；仓库不可检查时返回 error。".to_string(),
        "room.maintenance.submit" => "按槽位 payload 更新 Room 日记、Notebook 或实体；local 写入目标文档并提交本地 commit，workflow 提交请求并可能等待消费。返回 mode/state/localApplied/sync/revision；不会覆盖已占用槽，仓库未就绪、重复槽或 payload 错误会拒绝。".to_string(),
        "room.diary.active" => "读取 daily/weekly/monthly 三个 current.md；每层返回 period/path/available 与 content 或 missing、unreadable、invalid_utf8 issue。语义化只读，不接受任意路径。".to_string(),
        "room.diary.read" => "按 layer/period 读取精确日记文档；返回 document 的路径、可用状态、内容或 issue。daily 接 current/日期，weekly/monthly 接日期范围；非法日期/范围报错，缺失文档以 issue 表示。".to_string(),
        "room.notebook.recent" => "按有效时间列出最近 Notebook Markdown 预览；返回 documents（path/title/contentPreview/truncated/effectiveAt）和 warnings。缺少 Notebook 时为空结果；只读。".to_string(),
        "room.notebook.search" => "以不区分大小写的子串搜索 Notebook 路径、首个 H1 和正文；返回有界预览及 warnings。query 空或过长报错；只读，不接受正则。".to_string(),
        "room.notebook.read" => "读取 Notebook/ 下精确的 .md 相对路径；返回 path/content。未知或超大文档返回错误；只读，不是任意仓库文件读取器。".to_string(),
        "room.state.list" => "列出 State/entities/ 下的 Markdown 实体；返回按路径排序的 entities（entity/path），跳过 symlink 和非 Markdown 文件。只读。".to_string(),
        "room.state.read" => "按实体文件名 stem 读取 State/entities/{entity}.md；返回 path/content。缺失或超大文档报错；不接受路径输入，只读。".to_string(),
        _ => "未知本地工具；请使用已列出的工具名，输入和结果由对应工具合同定义。".to_string(),
    }
}
