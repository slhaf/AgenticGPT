use std::sync::Arc;

use rmcp::model::{Meta, Tool, ToolAnnotations};
use serde_json::{json, Map, Value};

use crate::{
    config::ToolsetConfig,
    operation::{
        tool_is_destructive, tool_is_open_world, tool_is_read_only, TOOL_NAMESPACE_BY_NAME,
    },
};

const PATCH_SCHEMA_DESCRIPTION: &str = "Codex apply_patch text beginning with *** Begin Patch and ending with *** End Patch; supports Add File, Delete File, Update File, and Move to across multiple files.";

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
        add("agentId", string("Target local agent id."));
    }
    match name {
        "browser.manual" => {
            add(
                "action",
                json!({
                    "type": "string",
                    "enum": ["read", "search"],
                    "description": "read returns one official-docs file; search scans selected runtime docs.",
                }),
            );
            add(
                "path",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Docs-relative path for action read; absolute paths are not accepted.",
                }),
            );
            add(
                "startLine",
                json!({"type":"integer","minimum":1,"description":"Optional inclusive line number for action read."}),
            );
            add(
                "endLine",
                json!({"type":"integer","minimum":1,"description":"Optional inclusive line number for action read."}),
            );
            add(
                "query",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4096,
                    "description": "Required bounded substring query for action search.",
                }),
            );
            add(
                "maxResults",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "default": 50,
                    "description": "Maximum matches for action search.",
                }),
            );
            add(
                "contextLines",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 5,
                    "default": 2,
                    "description": "Context lines before and after each action-search match.",
                }),
            );
        }
        "browser.acquire" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Caller-chosen persistent lease name; Browser manager validates its lease syntax.",
                }),
            );
            add(
                "idleTimeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "Idle lease timeout in seconds; acquire is idempotent by name.",
                }),
            );
        }
        "browser.repl" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Persistent Browser lease name acquired with browser.acquire.",
                }),
            );
            add(
                "code",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 262144,
                    "description": "Arbitrary JavaScript using official Browser SDK bindings; non-empty and at most 256 KiB UTF-8 bytes.",
                }),
            );
            add(
                "timeoutMs",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 120000,
                    "default": 20000,
                    "description": "Per-call JavaScript deadline in milliseconds.",
                }),
            );
            add(
                "title",
                json!({
                    "type": "string",
                    "maxLength": 128,
                    "description": "Optional bounded observability title; not injected into JavaScript.",
                }),
            );
        }
        "browser.reset" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Persistent Browser lease name; recovery/admin reset preserves the lease.",
                }),
            );
        }
        "browser.release" => {
            add(
                "name",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "description": "Persistent Browser lease name; release performs final cleanup.",
                }),
            );
        }
        "browser.list" => {}
        "file.read" => {
            add(
                "path",
                string("File or directory path; resolved and checked by path policy."),
            );
            add(
                "metadata",
                boolean("Include file metadata alongside content; default false."),
            );
            add("startLine", number("Inclusive start line."));
            add("endLine", number("Inclusive end line."));
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("File or directory path; resolved and checked by path policy."),
                        "metadata":boolean("Include file metadata alongside content; default false."),
                        "startLine":number("Inclusive start line."), "endLine":number("Inclusive end line.")
                    },"required":["path"]},
                    "description":"Ordered batch reads; mutually exclusive with flat single-read fields. Maximum 32."
                }),
            );
        }
        "file.search" => {
            add(
                "path",
                string("File or directory root; resolved and checked by path policy."),
            );
            add("query", string("Literal or regex query."));
            add(
                "mode",
                json!({"type":"string","enum":["literal","regex"],"default":"literal"}),
            );
            add("caseSensitive", boolean("Case-sensitive; default true."));
            add("include", strings("Include globs; max 16."));
            add("exclude", strings("Exclude globs; max 16."));
            add(
                "contextLines",
                json!({
                    "type":"integer",
                    "minimum":0,
                    "description":"Requested context lines; values above the live configured maximum are clipped and reported.",
                    "default":0
                }),
            );
            add("maxResults", number("Maximum matches, max 200."));
            add("hidden", boolean("Include hidden files; default false."));
            add(
                "respectGitignore",
                boolean("Honor Git ignore rules inside repositories; default true."),
            );
            add(
                "requests",
                json!({
                    "type":"array", "minItems":1, "maxItems":32,
                    "items":{"type":"object","additionalProperties":false,"properties":{
                        "path":string("File or directory root; resolved and checked by path policy."),
                        "query":string("Literal or regex query."),
                        "mode":{"type":"string","enum":["literal","regex"],"default":"literal"},
                        "caseSensitive":{"type":"boolean","default":true},
                        "include":strings("Include globs; max 16."), "exclude":strings("Exclude globs; max 16."),
                        "contextLines":{"type":"integer","minimum":0,"default":0},
                        "maxResults":{"type":"integer","maximum":200,"default":50},
                        "hidden":{"type":"boolean","default":false},
                        "respectGitignore":{"type":"boolean","default":true}
                    },"required":["path","query"]},
                    "description":"Ordered batch searches; mutually exclusive with flat single-search fields. Maximum 32."
                }),
            );
        }
        "file.edit" => {
            add("patch", string(PATCH_SCHEMA_DESCRIPTION));
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"Request one confirmation before an effective mutation."}),
            );
        }
        "mcp.list" => add("serverId", string("Optional configured MCP server id.")),
        "process.exec" => {
            add("program", string("Executable name or path."));
            add("args", strings("Direct argument vector."));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional workstream key; max 32 characters."
                }),
            );
            add(
                "needConfirm",
                json!({"type":"boolean","default":false,"description":"Request confirmation before execution."}),
            );
            add("workingDirectory", string("Process working directory."));
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"Bounded inline wait in seconds."}),
            );
        }
        "process.batch" => {
            add(
                "elements",
                json!({
                    "type": "array",
                    "items": {"type": "object", "properties": {
                        "program": string("Executable name or path."),
                        "args": strings("Direct argument vector."),
                        "workingDirectory": string("Per-element working directory.")
                    }, "required": ["program"], "additionalProperties": false}
                }),
            );
            add(
                "needConfirm",
                boolean("Request confirmation for the batch."),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Workstream key inherited by children."
                }),
            );
            add(
                "workingDirectory",
                string("Default process working directory."),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"Bounded inline wait in seconds."}),
            );
        }
        "process.status" => {
            add("processId", string("Managed process id."));
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "Bounded wait in seconds; defaults to 5 and is capped at 30.",
                ),
            );
        }
        "process.cancel" => {
            add("processId", string("Managed process id."));
        }
        "process.list" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Exact human-readable workstream filter."
                }),
            );
            add(
                "kind",
                json!({"type":"string","enum":["command","skill","mcp"]}),
            );
            add(
                "state",
                json!({"type":"string","enum":["queued","waiting_confirmation","starting","running","completed","failed","rejected","cancel_requested","cancelled","timed_out","detached","unknown_after_restart","skipped"]}),
            );
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"default":50,"description":"Maximum processes to return."}),
            );
            add(
                "cursor",
                string("Opaque cursor returned by a prior process.list response."),
            );
        }
        "process.output" => {
            add("processId", string("Managed process id."));
            add(
                "cursor",
                string("Opaque byte-offset cursor returned by a prior process.output response."),
            );
            add(
                "maxBytes",
                json!({"type":"integer","minimum":1,"maximum":32768,"default":8192,"description":"Maximum aggregate encoded stdout and stderr bytes for this page."}),
            );
        }
        "process.result" => {
            add("processId", string("Managed process id."));
            add(
                "maxBytes",
                json!({"type":"integer","minimum":1,"maximum":524288,"default":8192,"description":"Maximum retained result bytes to return; oversized results are reported without partial JSON."}),
            );
        }
        "tmux.sessions" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "create", "close"], "description":"list discovers sessions; create requires name/cwd; close terminates a session."}),
            );
            add("name", string("tmux session name for create or close."));
            add("cwd", string("Working directory for create."));
            add(
                "needConfirm",
                boolean("Request confirmation before closing."),
            );
        }
        "tmux.panes" => {
            add(
                "action",
                json!({"type": "string", "enum": ["list", "capture"], "description":"list discovers panes; capture requires a target and returns bounded history."}),
            );
            add("session", string("Optional tmux session name for list."));
            add("target", string("tmux pane target for capture."));
            add("lines", number("History lines for capture, default 160."));
        }
        "tmux.listPanes" => add("session", string("Optional tmux session name.")),
        "tmux.capturePane" => {
            add("target", string("tmux pane target."));
            add("lines", number("History lines, default 160."));
        }
        "tmux.pasteText" => {
            add("target", string("tmux pane target."));
            add("text", string("Text to paste."));
            add("submit", boolean("Append Enter after pasting."));
            add(
                "needConfirm",
                boolean("Request confirmation before writing."),
            );
        }
        "tmux.exec" => {
            add("target", string("Shell pane target."));
            add("program", string("Program or shell builtin."));
            add("args", strings("Structured argument vector."));
            add("needConfirm", boolean("Request confirmation."));
            add("waitMs", number("Post-submit wait in milliseconds."));
            add("captureLines", number("Post-submit history lines."));
        }
        "tmux.createSession" => {
            add("name", string("tmux session name."));
            add("cwd", string("Session working directory."));
        }
        "tmux.closeSession" => {
            add("name", string("tmux session name."));
            add(
                "needConfirm",
                boolean("Request confirmation before closing."),
            );
        }
        "mcp.listServers" => add("agentId", string("Optional local agent id.")),
        "mcp.listTools" => add("serverId", string("Configured MCP server id.")),
        "mcp.batch" => {
            add(
                "calls",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 16,
                    "description": "Ordered downstream MCP calls; every call is validated before capacity admission/confirmation, aggregate serialized arguments are capped at 2 MiB, and downstream side effects are not rolled back.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["serverId", "toolName"],
                        "properties": {
                            "serverId": {"type": "string", "description": "Configured downstream MCP server id."},
                            "toolName": {"type": "string", "description": "Tool name returned by mcp.listTools for this server."},
                            "arguments": {
                                "type": "object",
                                "default": {},
                                "description": "Per-call arguments capped at 256 KiB serialized."
                            }
                        }
                    }
                }),
            );
            add(
                "mode",
                json!({"type":"string","enum":["parallel","sequential"],"default":"parallel","description":"Scheduling mode; sequential waits for each child to become terminal."}),
            );
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Workstream key inherited by children."
                }),
            );
            add(
                "failFast",
                json!({"type":"boolean","default":false,"description":"After a hard failure, skip only children that have not started; already-started calls are never cancelled."}),
            );
            add(
                "waitSeconds",
                json!({"type":"integer","minimum":0,"maximum":30,"default":5,"description":"Bounded inline wait before returning child process responses; maximum 30 seconds."}),
            );
            add(
                "timeoutSeconds",
                json!({"type":"integer","minimum":1,"maximum":900,"default":300,"description":"Absolute downstream confirmation/connect/request deadline; maximum 900 seconds."}),
            );
        }
        "mcp.callTool" => {
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional workstream key."
                }),
            );
            add(
                "serverId",
                string("Configured downstream MCP server id; discover valid values with mcp.list."),
            );
            add(
                "toolName",
                string("Downstream MCP tool name returned by mcp.list for serverId."),
            );
            add(
                "arguments",
                json!({
                    "type": "object",
                    "description": "Downstream tool arguments as a JSON object; maximum serialized size 256 KiB.",
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
                    "description": "Bounded inline wait before returning the process response."
                }),
            );
            add(
                "timeoutSeconds",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 900,
                    "default": 300,
                    "description": "Absolute downstream execution deadline."
                }),
            );
        }
        "bootstrap.read" => add("id", string("Guide id returned by bootstrap.")),
        "skills.list" => {
            add("query", string("Optional case-insensitive skill query."));
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"Maximum skills returned."}),
            );
            add(
                "activeOnly",
                json!({"type":"boolean","default":false,"description":"Return only valid active skill summaries."}),
            );
        }
        "skills.setActive" => {
            add("id", string("Skill id."));
            add(
                "active",
                json!({"type":"boolean","default":false,"description":"Whether the skill should be active."}),
            );
        }
        "skills.read" => {
            add("id", string("Skill id."));
            add("path", string("Optional package-relative resource path."));
        }
        "skills.search" => {
            add("query", string("Case-insensitive search query."));
            add(
                "limit",
                json!({"type":"integer","minimum":1,"maximum":100,"description":"Maximum skills returned."}),
            );
        }
        "skills.activate" | "skills.deactivate" => add("id", string("Skill id.")),
        "skills.install" => {
            add("id", string("Target skill id."));
            add(
                "source",
                json!({
                    "type":"object",
                    "required":["type"],
                    "additionalProperties":false,
                    "description":"Tagged skill source descriptor. Use github sources for repositories and files sources for explicit file payloads.",
                    "properties":{
                        "type":{"type":"string","enum":["github","files"]},
                        "repository":{"type":"string"},
                        "url":{"type":"string"},
                        "ref":{"type":"string"},
                        "path":{"type":"string"},
                        "files":{
                            "type":"array",
                            "minItems":1,
                            "description":"Explicit file payloads for files source.",
                            "items":{
                                "type":"object",
                                "required":["path"],
                                "additionalProperties":false,
                                "properties":{
                                    "path":{"type":"string","description":"Package-relative destination path."},
                                    "url":{"type":"string","description":"HTTPS source URL."},
                                    "content":{"type":"string","description":"Inline UTF-8 content."},
                                    "contentBase64":{"type":"string","description":"Inline base64 content."},
                                    "sha256":{"type":"string","description":"Optional expected SHA-256."},
                                    "executable":{"type":"boolean","default":false}
                                }
                            }
                        }
                    }
                }),
            );
            add(
                "replaceExisting",
                json!({"type":"boolean","default":false,"description":"Archive an existing skill before replacement."}),
            );
            add(
                "activateAfterInstall",
                json!({"type":"boolean","description":"Optional activation choice."}),
            );
            add(
                "idempotencyKey",
                json!({"type":"string","minLength":1,"maxLength":128,"description":"Optional retry key."}),
            );
        }
        "skills.install.get" => {
            add("installId", string("Installation job id."));
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "Bounded status wait in seconds; defaults to 5 and is capped at 30.",
                ),
            );
        }
        "skills.install.cancel" => add("installId", string("Installation job id.")),
        "skills.run" => {
            add("id", string("Skill id."));
            add("path", string("Package-relative executable path."));
            add(
                "group",
                json!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 32,
                    "description": "Optional skill workstream key."
                }),
            );
            add("args", strings("Script argument vector."));
            add("workingDirectory", string("Optional working directory."));
            add(
                "waitSeconds",
                wait_seconds_schema(
                    5,
                    "Bounded inline wait in seconds; defaults to 5 and is capped at 30.",
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
                    "description": "One to five maintenance requests; each slot may appear at most once. The set is validated against the Room repository before any mutation.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["slot", "payload"],
                        "properties": {
                            "slot": {
                                "type": "string",
                                "enum": ["diary.daily", "diary.weekly", "diary.monthly", "notebook", "entity"],
                                "description": "Unique Room semantic slot to maintain."
                            },
                            "payload": {
                                "description": "Slot-specific maintenance payload; validated by the Room maintenance executor."
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
                    "description": "Optional execution mode override; local applies in the validated Room repository, workflow submits through the configured Room workflow."
                }),
            );
            add(
                "waitSeconds",
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 30,
                    "default": 0,
                    "description": "Optional bounded wait for workflow consumption and local fast-forward, from 0 through 30 seconds."
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
                    "description": "Room diary temporal layer."
                }),
            );
            add(
                "period",
                json!({
                    "type": "string",
                    "pattern": "^(current|\\d{4}-\\d{2}-\\d{2}(--\\d{4}-\\d{2}-\\d{2})?)$",
                    "description": "Room-local logical period: daily uses current or YYYY-MM-DD; weekly/monthly use current or YYYY-MM-DD--YYYY-MM-DD."
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
                    "description": "Maximum bounded recent Notebook previews returned."
                }),
            );
        }
        "room.notebook.search" => {
            add(
                "query",
                string("Case-insensitive bounded substring query over Notebook paths, H1 titles, and bodies."),
            );
            add(
                "limit",
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum bounded Notebook previews returned."
                }),
            );
        }
        "room.notebook.read" => {
            add(
                "path",
                string("Exact Notebook-relative Markdown path returned or discovered under Notebook/; arbitrary repository paths are rejected."),
            );
        }
        "room.state.read" => {
            add(
                "entity",
                string("State entity filename stem resolved under State/entities/; arbitrary repository paths are rejected."),
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
        "agent.info" => "Inspect local Agent runtime, workspace policy, connectivity, capacity, and health; read-only diagnostics.".to_string(),
        "browser.manual" => "Read or search official Browser SDK documentation bundled with the selected runtime; results stay docs-relative.".to_string(),
        "browser.acquire" => "Acquire a named persistent Browser lease; its JavaScript kernel and Browser SDK bindings survive multiple calls.".to_string(),
        "browser.repl" => "Run arbitrary JavaScript in a persistent Browser lease; use official Browser SDK semantics in JS, with destructive open-world effects.".to_string(),
        "browser.reset" => "Reset a Browser lease for recovery or administration; preserves the lease name and is not normal per-call cleanup.".to_string(),
        "browser.release" => "Release a Browser lease for final cleanup; ends the turn and removes its persistent kernel.".to_string(),
        "browser.list" => "List bounded Browser runtime availability and named lease state without exposing runtime paths or opaque IDs.".to_string(),
        "file.read" => "Read bounded UTF-8 workspace files without mutation. Supports single reads and ordered batch reads; use line ranges for large files and use metadata only when file information is needed.".to_string(),
        "file.search" => "Search bounded workspace text without mutation. Supports scoped literal or regex searches and ordered batch searches; use filters to limit noisy workspace scans.".to_string(),
        "file.edit" => "Apply a Codex apply_patch patch to workspace files; mutations remain policy and confirmation controlled.".to_string(),
        "process.exec" => "Start one managed local process; use process.status/output/result for lifecycle follow-up.".to_string(),
        "process.batch" => "Start multiple managed local processes under one admission boundary; started side effects are not rolled back.".to_string(),
        "process.status" => "Inspect process state and metadata without output or result bodies; optionally wait up to 30 seconds.".to_string(),
        "process.list" => "List process metadata with optional filters and pagination; read-only discovery.".to_string(),
        "process.output" => "Read bounded, non-consuming process output pages using lossless byte offsets.".to_string(),
        "process.result" => "Retrieve a retained MCP result explicitly; unavailable or oversized results are reported without partial JSON.".to_string(),
        "process.cancel" => "Request cancellation of one managed process; returned state is observed evidence, not a termination guarantee.".to_string(),
        "tmux.listSessions" => "List persistent tmux sessions; read-only.".to_string(),
        "tmux.sessions" => "Manage persistent tmux sessions. Use list for discovery, create for reusable sessions, and close only when the session should be terminated.".to_string(),
        "tmux.listPanes" => "List tmux panes; read-only.".to_string(),
        "tmux.panes" => "Inspect tmux panes or capture bounded pane output. Use this for observing persistent terminal state, not for submitting commands.".to_string(),
        "tmux.capturePane" => "Capture bounded history from one tmux pane; read-only.".to_string(),
        "tmux.pasteText" => "Paste text into a non-shell tmux pane or TUI; shell panes are rejected.".to_string(),
        "tmux.exec" => "Submit one structured command to a tmux shell pane; submission does not prove command completion.".to_string(),
        "tmux.createSession" => "Create or reuse one persistent tmux session in an allowed working directory.".to_string(),
        "tmux.closeSession" => "Close one persistent tmux session; destructive and confirmation-aware.".to_string(),
        "mcp.listServers" => "List configured downstream MCP servers; read-only discovery.".to_string(),
        "mcp.listTools" => "List tools exposed by one downstream MCP server; read-only discovery.".to_string(),
        "mcp.list" => "List downstream MCP servers or one server's tools; read-only discovery.".to_string(),
        "mcp.batch" => "Run multiple downstream MCP calls as managed processes under one admission boundary; downstream side effects are not rolled back.".to_string(),
        "mcp.callTool" => "Run one downstream MCP tool as a managed process; use process tools for lifecycle follow-up.".to_string(),
        "bootstrap" => "Load Room bootstrap guidance; read-only and not a generic file reader.".to_string(),
        "bootstrap.read" => "Read one validated Room bootstrap guide; not an arbitrary path reader.".to_string(),
        "skills.list" => "List local skills with optional filtering; read-only discovery.".to_string(),
        "skills.setActive" => "Set one local skill's active state; grants no permissions and executes nothing.".to_string(),
        "skills.read" => "Read one local skill package or package resource; not a generic file reader.".to_string(),
        "skills.search" => "Search local skill metadata and content; read-only discovery.".to_string(),
        "skills.active" => "List active local skill state, including stale entries; read-only.".to_string(),
        "skills.activate" => "Mark one valid local skill active; executes nothing.".to_string(),
        "skills.deactivate" => "Remove active state for one local skill; executes nothing.".to_string(),
        "skills.install" => "Start an asynchronous local skill installation from a validated source. Follow the returned installation job with get or cancel; installation may mutate the local skills workspace.".to_string(),
        "skills.install.get" => "Inspect or briefly wait for one skill installation; read-only lifecycle inspection.".to_string(),
        "skills.install.cancel" => "Request cooperative cancellation of one skill installation before commit.".to_string(),
        "skills.run" => "Run an executable from an active local skill as a managed process.".to_string(),
        "room.maintenance.status" => "Inspect Room maintenance readiness, repository state, schema/scaffold support, executor configuration, workflow/remote availability, synchronization heads, and deterministic occupancy for all five semantic slots; read-only and non-destructive.".to_string(),
        "room.maintenance.submit" => "Apply one to five unique Room maintenance slot requests after exact validation; destructive but confined to the validated Room repository, with optional local/workflow mode and bounded workflow wait; not open-world.".to_string(),
        "room.diary.active" => "Read the active daily, weekly, and monthly Room diary documents; read-only, semantic, and bounded.".to_string(),
        "room.diary.read" => "Read one exact Room diary document by validated semantic layer and period; read-only, semantic, and bounded.".to_string(),
        "room.notebook.recent" => "Read bounded recent Room notebook Markdown previews; read-only, semantic, and bounded semantic discovery.".to_string(),
        "room.notebook.search" => "Search Room notebook Markdown by bounded case-insensitive substring fields; read-only, semantic, and bounded discovery.".to_string(),
        "room.notebook.read" => "Read one exact Room notebook Markdown document under the validated Notebook root; read-only, semantic, and bounded.".to_string(),
        "room.state.list" => "List deterministic Room state entity documents; read-only, semantic, and bounded.".to_string(),
        "room.state.read" => "Read one exact Room state entity Markdown document by validated entity name; read-only, semantic, and bounded.".to_string(),
        _ => "Agentic GPT local tool.".to_string(),
    }
}
