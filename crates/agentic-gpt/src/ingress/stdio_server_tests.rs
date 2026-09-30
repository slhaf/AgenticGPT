use std::{
    collections::{BTreeSet, HashMap},
    io::Cursor,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use agentic_gpt_protocol::{
    AgentMessage, McpBatchChildResponse, McpBatchStatus, SkillActivationRequest,
};
use base64::Engine;
use rmcp::{
    model::{CallToolRequestParams, Content},
    ServiceExt,
};
use serde::Deserialize;
use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex, RwLock};

use super::stdio_transport::ResumableStdioTransport;
use super::*;
use crate::{
    config::{Config, ToolsetConfig},
    skill_installs::InstallManager,
    skills::SkillLeaseManager,
    state::RuntimeModel,
};

#[derive(Debug, Deserialize)]
struct ToolContractCase {
    id: String,
    tool: String,
    kind: String,
    arguments: Value,
    expect: Value,
}

fn encoded_test_image(format: image::ImageFormat) -> anyhow::Result<Vec<u8>> {
    let pixels = image::RgbImage::from_fn(2, 2, |x, y| {
        image::Rgb([40 + x as u8 * 20, 90 + y as u8 * 30, 180])
    });
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(pixels).write_to(&mut output, format)?;
    Ok(output.into_inner())
}

fn gif_fixture(canvas: (u16, u16), frame: (u16, u16), delays: &[u16]) -> anyhow::Result<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder =
            gif::Encoder::new(&mut output, canvas.0, canvas.1, &[0, 0, 0, 255, 255, 255])?;
        for delay in delays {
            let frame_data = gif::Frame {
                width: frame.0,
                height: frame.1,
                delay: *delay,
                buffer: std::borrow::Cow::Owned(vec![
                    0;
                    usize::from(frame.0) * usize::from(frame.1)
                ]),
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame_data)?;
        }
    }
    Ok(output)
}

fn append_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = !0_u32;
    for byte in &output[crc_start..] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    output.extend_from_slice(&(!crc).to_be_bytes());
}

fn oversized_png_header() -> anyhow::Result<Vec<u8>> {
    let valid = encoded_test_image(image::ImageFormat::Png)?;
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&4097_u32.to_be_bytes());
    header.extend_from_slice(&4097_u32.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    append_png_chunk(&mut png, b"IHDR", &header);
    png.extend_from_slice(&valid[33..]);
    Ok(png)
}

#[tokio::test]
async fn normal_and_room_tool_sets_follow_fixed_surface_contract() {
    let normal = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let room = AgentMcpServer::new(test_state(CapabilityProfile::Room));
    let normal_tools = normal.current_tools().await;
    let room_tools = room.current_tools().await;
    let names = |tools: Vec<Tool>| {
        tools
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<BTreeSet<_>>()
    };
    let expected_normal = [
        "agent.info",
        "browser.acquire",
        "browser.list",
        "browser.manual",
        "browser.release",
        "browser.repl",
        "browser.reset",
        "file.edit",
        "file.read",
        "file.search",
        "process.cancel",
        "process.list",
        "process.output",
        "process.result",
        "process.status",
        "mcp.batch",
        "mcp.callTool",
        "mcp.list",
        "process.batch",
        "process.exec",
        "skills.install",
        "skills.install.cancel",
        "skills.install.get",
        "skills.list",
        "skills.read",
        "skills.run",
        "skills.setActive",
        "tmux.exec",
        "tmux.panes",
        "tmux.pasteText",
        "tmux.sessions",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let room_additions = [
        "bootstrap",
        "bootstrap.read",
        "room.diary.active",
        "room.diary.read",
        "room.maintenance.status",
        "room.maintenance.submit",
        "room.notebook.read",
        "room.notebook.recent",
        "room.notebook.search",
        "room.state.list",
        "room.state.read",
    ];
    let mut expected_room = expected_normal.clone();
    expected_room.extend(room_additions.into_iter().map(str::to_owned));

    let normal_names = names(normal_tools.clone());
    assert_eq!(normal_names, expected_normal);
    assert_eq!(names(room_tools), expected_room);
    assert!(normal_tools
        .iter()
        .all(|tool| !tool.name.starts_with("room.") && !tool.name.starts_with("bootstrap")));

    // Dispatch-only aliases must never leak into the advertised MCP
    // surface.
    for alias in [
        "file.batch",
        "browser.read",
        "browser.click",
        "browser.navigate",
        "browser.tabs",
        "browser.screenshot",
        "user.notify.deliver",
        "mcp.listServers",
        "mcp.listTools",
        "skills.active",
        "skills.activate",
        "skills.deactivate",
        "tmux.listSessions",
        "tmux.listPanes",
        "tmux.capturePane",
    ] {
        assert!(
            !expected_room.contains(alias),
            "dispatch-only alias advertised: {alias}"
        );
        assert!(!normal_names.contains(alias));
    }

    let mut filtered_config = normal.state.config.read().await.clone();
    filtered_config.toolsets.disable(ToolNamespace::File);
    *normal.state.config.write().await = filtered_config;
    let filtered_names = names(normal.current_tools().await);
    let mut expected_filtered = expected_normal.clone();
    for name in ["file.edit", "file.read", "file.search"] {
        expected_filtered.remove(name);
    }
    assert_eq!(filtered_names, expected_filtered);
    let error = normal
        .call(CallToolRequestParams::new("file.read"))
        .await
        .expect_err("disabled namespace must not remain callable");
    assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    assert!(
        normal
            .call(CallToolRequestParams::new("mcp.list"))
            .await
            .is_ok(),
        "disabling file must leave the MCP namespace callable"
    );
    let mut browser_filtered_config = normal.state.config.read().await.clone();
    browser_filtered_config
        .toolsets
        .disable(ToolNamespace::Browser);
    *normal.state.config.write().await = browser_filtered_config;
    let browser_filtered_names = names(normal.current_tools().await);
    assert!(browser_filtered_names
        .iter()
        .all(|name| !name.starts_with("browser.")));
    let browser_error = normal
        .call(CallToolRequestParams::new("browser.list"))
        .await
        .expect_err("disabled Browser namespace must not remain callable");
    assert_eq!(browser_error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);

    let serialized = serde_json::to_string(&normal_tools).unwrap();
    assert!(!serialized.contains("agentId"));
    assert!(!serialized.contains("confirmMethod"));
    assert!(serialized.contains("mcp.list"));
    assert!(serialized.contains("skills.setActive"));
    assert!(serialized.contains("tmux.sessions"));
    assert!(serialized.contains("tmux.panes"));
}

#[tokio::test]
async fn compact_tool_schema_budgets_hold() {
    let normal = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let room = AgentMcpServer::new(test_state(CapabilityProfile::Room));
    let normal_tools = normal.current_tools().await;
    let room_tools = room.current_tools().await;
    for (label, tools, max_total, max_inputs) in [
        // File plus the six frozen Browser schemas remain under explicit
        // finite caps for the resulting Normal/Room surfaces.
        ("normal", &normal_tools, 32_000usize, 17_000usize),
        ("room", &room_tools, 48_000usize, 24_000usize),
    ] {
        let serialized = serde_json::to_vec(tools).unwrap();
        let input_bytes = tools
            .iter()
            .map(|tool| {
                let value = serde_json::to_value(tool).unwrap();
                serde_json::to_vec(&value["inputSchema"]).unwrap().len()
            })
            .sum::<usize>();
        assert!(
            serialized.len() <= max_total,
            "{label} tool schemas use {} bytes, budget is {max_total}",
            serialized.len()
        );
        assert!(
            input_bytes <= max_inputs,
            "{label} input schemas use {input_bytes} bytes, budget is {max_inputs}",
            input_bytes = input_bytes,
            max_inputs = max_inputs
        );
    }
}

#[tokio::test]
async fn file_surface_schema_is_exact() -> anyhow::Result<()> {
    let tools = AgentMcpServer::new(test_state(CapabilityProfile::Normal))
        .current_tools()
        .await;
    let names = tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    let removed_tool = ["file", "batch"].join(".");
    assert!(!names.iter().any(|name| name == &removed_tool));

    let read = serde_json::to_value(tool_descriptor("file.read"))?;
    assert_eq!(read["inputSchema"]["required"], json!([]));
    assert!(read["inputSchema"]["properties"].get("requests").is_some());
    assert!(read["inputSchema"].get("oneOf").is_none());
    assert_eq!(read["inputSchema"]["properties"]["requests"]["minItems"], 1);
    assert_eq!(
        read["inputSchema"]["properties"]["requests"]["maxItems"],
        32
    );

    let search = serde_json::to_value(tool_descriptor("file.search"))?;
    assert!(search["inputSchema"]["properties"]
        .get("requests")
        .is_some());
    assert!(search["inputSchema"].get("oneOf").is_none());
    assert_eq!(
        search["inputSchema"]["properties"]["requests"]["minItems"],
        1
    );
    assert_eq!(
        search["inputSchema"]["properties"]["requests"]["maxItems"],
        32
    );

    let edit = serde_json::to_value(tool_descriptor("file.edit"))?;
    let edit_fields = edit["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_edit_fields = ["needConfirm", "patch"]
        .into_iter()
        .map(String::from)
        .collect::<BTreeSet<_>>();
    assert_eq!(edit_fields, expected_edit_fields);
    assert_eq!(edit["inputSchema"]["required"], json!(["patch"]));
    Ok(())
}

#[tokio::test]
async fn mutating_tool_annotations_do_not_promise_read_only_or_additive_effects(
) -> anyhow::Result<()> {
    let tools = AgentMcpServer::new(test_state(CapabilityProfile::Normal))
        .current_tools()
        .await;
    for name in [
        "skills.setActive",
        "process.exec",
        "process.batch",
        "mcp.callTool",
        "mcp.batch",
        "tmux.exec",
        "tmux.pasteText",
    ] {
        let tool = tools.iter().find(|tool| tool.name == name).unwrap();
        let descriptor = serde_json::to_value(tool)?;
        assert_eq!(descriptor["annotations"]["readOnlyHint"], false, "{name}");
        assert_eq!(descriptor["annotations"]["destructiveHint"], true, "{name}");
    }
    Ok(())
}

#[test]
fn browser_descriptors_and_annotations_are_frozen() -> anyhow::Result<()> {
    let expected = [
        ("browser.manual", true, false, false, json!(["action"])),
        (
            "browser.acquire",
            false,
            false,
            false,
            json!(["name", "idleTimeoutSeconds"]),
        ),
        ("browser.repl", false, true, true, json!(["name", "code"])),
        ("browser.reset", false, true, false, json!(["name"])),
        ("browser.release", false, true, false, json!(["name"])),
        ("browser.list", true, false, false, json!([])),
    ];
    for (name, read_only, destructive, open_world, required) in expected {
        let descriptor = serde_json::to_value(tool_descriptor(name))?;
        assert_eq!(descriptor["inputSchema"]["required"], required, "{name}");
        assert_eq!(descriptor["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            descriptor["annotations"]["readOnlyHint"], read_only,
            "{name}"
        );
        assert_eq!(
            descriptor["annotations"]["destructiveHint"], destructive,
            "{name}"
        );
        assert_eq!(
            descriptor["annotations"]["openWorldHint"], open_world,
            "{name}"
        );
    }
    let manual = serde_json::to_value(tool_descriptor("browser.manual"))?;
    assert_eq!(
        manual["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        [
            "action",
            "contextLines",
            "endLine",
            "maxResults",
            "path",
            "query",
            "startLine",
        ]
        .into_iter()
        .map(String::from)
        .collect::<BTreeSet<_>>()
    );
    let repl = serde_json::to_value(tool_descriptor("browser.repl"))?;
    assert_eq!(
        repl["inputSchema"]["properties"]["timeoutMs"]["default"],
        20_000
    );
    assert_eq!(repl["inputSchema"]["properties"]["timeoutMs"]["minimum"], 1);
    assert_eq!(
        repl["inputSchema"]["properties"]["timeoutMs"]["maximum"],
        120_000
    );
    assert_eq!(repl["inputSchema"]["properties"]["title"]["maxLength"], 128);
    Ok(())
}

#[test]
fn browser_conditional_arguments_and_bounds_are_validated() {
    assert!(validate_stdio_arguments(
        "browser.manual",
        &json!({"action":"read","path":"api.md","startLine":1,"endLine":2})
    )
    .is_ok());
    assert!(validate_stdio_arguments(
        "browser.manual",
        &json!({"action":"search","query":"tabs","maxResults":1,"contextLines":5})
    )
    .is_ok());
    for arguments in [
        json!({"action":"read"}),
        json!({"action":"read","path":"api.md","query":"tabs"}),
        json!({"action":"search"}),
        json!({"action":"search","query":"tabs","path":"api.md"}),
        json!({"action":"search","query":"tabs","maxResults":0}),
        json!({"action":"search","query":"tabs","contextLines":6}),
        json!({"action":"other","query":"tabs"}),
    ] {
        assert!(
            validate_stdio_arguments("browser.manual", &arguments).is_err(),
            "accepted invalid manual arguments: {arguments}"
        );
    }
    assert!(validate_stdio_arguments(
        "browser.acquire",
        &json!({"name":"lease","idleTimeoutSeconds":1})
    )
    .is_ok());
    assert!(validate_stdio_arguments(
        "browser.acquire",
        &json!({"name":"lease","idleTimeoutSeconds":86400})
    )
    .is_ok());
    for seconds in [0, 86401] {
        assert!(validate_stdio_arguments(
            "browser.acquire",
            &json!({"name":"lease","idleTimeoutSeconds":seconds})
        )
        .is_err());
    }
    assert!(
        validate_stdio_arguments("browser.repl", &json!({"name":"lease","code":"1+1"})).is_ok()
    );
    assert!(validate_stdio_arguments(
        "browser.repl",
        &json!({"name":"lease","code":"1+1","timeoutMs":120000,"title":"ok"})
    )
    .is_ok());
    for arguments in [
        json!({"name":"lease","code":""}),
        json!({"name":"lease","code":"1+1","timeoutMs":0}),
        json!({"name":"lease","code":"1+1","timeoutMs":120001}),
        json!({"name":"lease","code":"1+1","title":"🙂".repeat(129)}),
    ] {
        assert!(
            validate_stdio_arguments("browser.repl", &arguments).is_err(),
            "accepted invalid repl arguments: {arguments}"
        );
    }
}

#[tokio::test]
async fn browser_missing_runtime_has_stable_degraded_results() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let source = "console.log('browser-audit-secret')";
    assert_eq!(
        server.dispatch("browser.list", json!({})).await?,
        json!({"runtimeAvailable":false,"leases":[]})
    );
    for (name, arguments) in [
        ("browser.manual", json!({"action":"search","query":"tabs"})),
        (
            "browser.acquire",
            json!({"name":"lease","idleTimeoutSeconds":1}),
        ),
        ("browser.repl", json!({"name":"lease","code":source})),
        ("browser.reset", json!({"name":"lease"})),
        ("browser.release", json!({"name":"lease"})),
    ] {
        let value = server.dispatch(name, arguments).await?;
        assert_eq!(value["error"]["code"], "browser_runtime_unavailable");
        assert_eq!(value["error"]["message"], "browser_runtime_unavailable");
    }
    let audit = std::fs::read_to_string(
        server
            .state
            .config
            .read()
            .await
            .workspace_root
            .join(".agentic-gpt-audit.jsonl"),
    )?;
    let records = audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["tool"] == "browser.repl")
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["requestSource"], "tunnel:browser.repl");
    assert_eq!(records[0]["codeBytes"], source.len());
    assert_eq!(records[0]["codeSha256"], browser_sha256(source));
    assert_eq!(records[0]["outcome"], "failed");
    assert_eq!(records[0]["errorCode"], "browser_runtime_unavailable");
    assert!(!audit.contains(source));

    Ok(())
}

#[test]
fn browser_reporting_and_error_helpers_do_not_retain_source() {
    let source = "console.log('do-not-record')";
    let reported = browser_report_arguments(&json!({
        "name":"lease",
        "code":source,
        "timeoutMs":42,
        "title":"title"
    }));
    assert!(reported.get("code").is_none());
    assert_eq!(reported["codeBytes"], source.len());
    assert_eq!(reported["codeSha256"], browser_sha256(source));
    assert!(!reported.to_string().contains(source));

    let error = browser_error_value(
        "browser_runtime_lease_not_found: /secret/path and private diagnostics",
    );
    assert_eq!(
        error,
        json!({"error":{"code":"browser_runtime_lease_not_found","message":"browser_runtime_lease_not_found"}})
    );
    assert_eq!(
        browser_error_code(
            "browser_runtime_node_repl_call_failed: /secret/path and private diagnostics"
        ),
        "browser_runtime_node_repl_call_failed"
    );
}

#[tokio::test]
async fn browser_manual_dispatch_uses_selected_runtime_docs_root() -> anyhow::Result<()> {
    let docs_root = std::env::temp_dir().join(format!("agentic-browser-docs-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&docs_root)?;
    std::fs::write(docs_root.join("api.md"), "before\nbrowser sdk\napi after\n")?;
    let server = AgentMcpServer::new(state_with_browser_runtime(
        docs_root.clone(),
        CallToolResult::default(),
    ));

    let read = server
        .dispatch(
            "browser.manual",
            json!({"action":"read","path":"api.md","startLine":2,"endLine":2}),
        )
        .await?;
    assert_eq!(read["path"], "api.md");
    assert_eq!(read["content"], "browser sdk\n");
    assert!(!serde_json::to_string(&read)?.contains(docs_root.to_string_lossy().as_ref()));

    let search = server
        .dispatch(
            "browser.manual",
            json!({"action":"search","query":"browser","maxResults":1}),
        )
        .await?;
    assert_eq!(search["matches"][0]["path"], "api.md");
    assert_eq!(search["matches"][0]["line"], 2);
    assert!(!serde_json::to_string(&search)?.contains(docs_root.to_string_lossy().as_ref()));

    let absolute = server
        .dispatch(
            "browser.manual",
            json!({"action":"read","path":docs_root.join("api.md")}),
        )
        .await?;
    assert_eq!(absolute["error"]["code"], "browser_manual_invalid_path");
    std::fs::remove_dir_all(docs_root)?;
    Ok(())
}

#[tokio::test]
async fn browser_list_maps_runtime_snapshots_and_release_is_idempotent() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(state_with_browser_runtime(
        PathBuf::from("/runtime/docs"),
        CallToolResult::default(),
    ));
    for (name, timeout) in [("zeta", 5_u64), ("alpha", 6_u64)] {
        server
            .dispatch(
                "browser.acquire",
                json!({"name":name,"idleTimeoutSeconds":timeout}),
            )
            .await?;
    }

    let listed = server.dispatch("browser.list", json!({})).await?;
    assert_eq!(listed["runtimeAvailable"], true);
    assert_eq!(listed["appVersion"], "test-browser");
    assert_eq!(listed["channel"], "test");
    let leases = listed["leases"].as_array().expect("lease array");
    assert_eq!(
        leases
            .iter()
            .map(|lease| lease["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    for lease in leases {
        assert!(lease["remainingIdleSeconds"].is_u64());
        assert!(lease["remainingIdleSeconds"].as_u64().unwrap() <= 6);
        let serialized = serde_json::to_string(lease)?;
        for forbidden in ["session_id", "turn_id", "nodeRepl", "/runtime"] {
            assert!(
                !serialized.contains(forbidden),
                "opaque field leaked: {forbidden}"
            );
        }
    }

    let absent = server
        .dispatch("browser.release", json!({"name":"missing"}))
        .await?;
    assert_eq!(absent, json!({"name":"missing","released":false}));
    server
        .dispatch("browser.release", json!({"name":"alpha"}))
        .await?;
    server
        .dispatch("browser.release", json!({"name":"zeta"}))
        .await?;
    Ok(())
}

#[tokio::test]
async fn browser_repl_returns_inner_result_channels_verbatim() -> anyhow::Result<()> {
    let mut inner = CallToolResult::default();
    inner.content = vec![
        Content::text("hello"),
        Content::image("aW1hZ2U=", "image/png"),
    ];
    inner.structured_content = Some(json!({"value":42}));
    inner.is_error = Some(true);
    inner.meta = Some(Meta(Map::from_iter([(
        "result-key".to_string(),
        json!("result-value"),
    )])));

    let server = AgentMcpServer::new(state_with_browser_runtime(
        PathBuf::from("/runtime/docs"),
        inner.clone(),
    ));
    server
        .dispatch(
            "browser.acquire",
            json!({"name":"lease","idleTimeoutSeconds":60}),
        )
        .await?;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });

    let client = ().serve((client_read, client_write)).await?;
    let outer = client
        .call_tool(
            CallToolRequestParams::new("browser.repl").with_arguments(Map::from_iter([
                ("name".to_string(), json!("lease")),
                ("code".to_string(), json!("browser code")),
            ])),
        )
        .await?;
    assert_eq!(serde_json::to_value(&outer)?, serde_json::to_value(&inner)?);
    assert_eq!(outer.content.len(), 2);
    assert_eq!(outer.structured_content, inner.structured_content);
    assert_eq!(outer.is_error, Some(true));
    assert_eq!(outer.meta, inner.meta);
    let _ = client.cancel().await;
    server_task.await??;
    Ok(())
}

#[test]
fn room_maintenance_descriptors_are_frozen() -> anyhow::Result<()> {
    let status = serde_json::to_value(tool_descriptor("room.maintenance.status"))?;
    assert_eq!(status["annotations"]["readOnlyHint"], true);
    assert_eq!(status["annotations"]["destructiveHint"], false);
    assert_eq!(status["annotations"]["openWorldHint"], false);
    assert_eq!(status["inputSchema"]["required"], json!([]));

    let submit = serde_json::to_value(tool_descriptor("room.maintenance.submit"))?;
    assert_eq!(submit["annotations"]["readOnlyHint"], false);
    assert_eq!(submit["annotations"]["destructiveHint"], true);
    assert_eq!(submit["annotations"]["openWorldHint"], false);
    assert_eq!(submit["inputSchema"]["required"], json!(["items"]));
    let items = &submit["inputSchema"]["properties"]["items"];
    assert_eq!(items["minItems"], 1);
    assert_eq!(items["maxItems"], 5);
    assert_eq!(items["items"]["required"], json!(["slot", "payload"]));
    assert_eq!(
        items["items"]["properties"]["slot"]["enum"],
        json!([
            "diary.daily",
            "diary.weekly",
            "diary.monthly",
            "notebook",
            "entity"
        ])
    );
    assert_eq!(
        submit["inputSchema"]["properties"]["mode"]["enum"],
        json!(["local", "workflow"])
    );
    assert_eq!(
        submit["inputSchema"]["properties"]["waitSeconds"]["minimum"],
        0
    );
    assert_eq!(
        submit["inputSchema"]["properties"]["waitSeconds"]["maximum"],
        30
    );
    Ok(())
}

#[test]
fn room_maintenance_submit_rejects_unknown_nested_fields() {
    let error =
        serde_json::from_value::<agentic_gpt_protocol::RoomMaintenanceSubmitRequest>(json!({
            "items": [{
                "slot": "entity",
                "payload": {"entity": "project", "content": "ok"},
                "unexpected": true
            }]
        }))
        .expect_err("nested maintenance fields must be strict");
    assert!(error.to_string().contains("unknown field"));
}

#[tokio::test]
async fn deterministic_tool_contract_corpus_exercises_public_dispatch() -> anyhow::Result<()> {
    let cases: Vec<ToolContractCase> = serde_json::from_str(include_str!(
        "../../../../tests/tool-contract-cases/cases.json"
    ))?;
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let fixture_path = workspace.join("contract-fixture.txt");
    std::fs::write(&fixture_path, "prefix\nneedle\nsuffix\n")?;

    for case in cases {
        let descriptor = serde_json::to_value(tool_descriptor(&case.tool))?;
        if case.kind != "negative" {
            validate_stdio_arguments(&case.tool, &case.arguments).unwrap_or_else(|error| {
                panic!("{} descriptor arguments rejected: {error}", case.id)
            });
        }
        let expected = &case.expect;

        if case.kind == "descriptor" {
            for required in expected["required"].as_array().expect("required fields") {
                assert!(
                    descriptor["inputSchema"]["required"]
                        .as_array()
                        .is_some_and(|fields| fields.contains(required)),
                    "{} missing required field {required}",
                    case.id
                );
            }
            continue;
        }

        let arguments = case.arguments;
        if case.kind == "negative" {
            match server.dispatch(&case.tool, arguments).await {
                Ok(value) => {
                    if let Some(code) = expected["errorCode"].as_str() {
                        assert_eq!(value["error"]["code"], code, "{} error code", case.id);
                    }
                    if let Some(message) = expected["errorIncludes"].as_str() {
                        assert!(
                            value["error"]["message"]
                                .as_str()
                                .is_some_and(|text| text.contains(message)),
                            "{} error message missing {message:?}: {}",
                            case.id,
                            value
                        );
                    }
                }
                Err(error) => {
                    let expected_text = expected["errorIncludes"]
                        .as_str()
                        .expect("negative dispatch error phrase");
                    assert!(
                        error.to_string().contains(expected_text),
                        "{} dispatch error missing {expected_text:?}: {error}",
                        case.id
                    );
                }
            }
            continue;
        }

        let value = server
            .dispatch(&case.tool, arguments)
            .await
            .unwrap_or_else(|error| panic!("{} dispatch failed: {error}", case.id));
        if let Some(fields) = expected["resultFields"].as_array() {
            for field in fields {
                let field = field.as_str().expect("result field string");
                assert!(
                    value.get(field).is_some(),
                    "{} missing result field {field}",
                    case.id
                );
            }
        }
        if let Some(status) = expected["status"].as_str() {
            assert_eq!(value["status"], status, "{} status", case.id);
        }
        if expected["noRevision"].as_bool() == Some(true) {
            assert!(
                !serde_json::to_string(&value)?.contains("revision"),
                "{} unexpectedly exposes a revision",
                case.id
            );
        }
        if let Some(count) = expected["operationCount"].as_u64() {
            assert_eq!(
                value["results"].as_array().map_or(0, Vec::len),
                count as usize,
                "{} operation count",
                case.id
            );
        }
        if let Some(total) = expected["groupTotal"].as_u64() {
            assert_eq!(
                value["groupCounts"]["total"], total,
                "{} group total",
                case.id
            );
        }
        if let Some(status) = expected["groupStatus"].as_str() {
            assert!(
                value["groups"]
                    .as_array()
                    .is_some_and(|groups| groups.iter().any(|group| group["status"] == status)),
                "{} missing group status {status}",
                case.id
            );
        }
        if let Some(committed) = expected["committed"].as_bool() {
            assert!(
                value["groups"].as_array().is_some_and(|groups| groups
                    .iter()
                    .any(|group| group["committed"] == committed)),
                "{} missing committed={committed}",
                case.id
            );
        }
        if let Some(failed) = expected["failedGroups"].as_u64() {
            assert_eq!(
                value["groupCounts"]["failed"], failed,
                "{} failed groups",
                case.id
            );
        }
        if let Some(failure_count) = expected["failureCount"].as_u64() {
            assert_eq!(
                value["failureCount"], failure_count,
                "{} failure count",
                case.id
            );
        }
        if let Some(added) = expected["changedLinesAdded"].as_u64() {
            assert_eq!(
                value["changedLines"]["added"], added,
                "{} added lines",
                case.id
            );
        }
        if let Some(removed) = expected["changedLinesRemoved"].as_u64() {
            assert_eq!(
                value["changedLines"]["removed"], removed,
                "{} removed lines",
                case.id
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn absent_room_tools_are_rejected_when_room_toolset_disabled() {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    for name in [
        "bootstrap",
        "bootstrap.read",
        "room.diary.active",
        "room.diary.read",
        "room.maintenance.status",
        "room.maintenance.submit",
        "room.notebook.recent",
        "room.notebook.search",
        "room.notebook.read",
        "room.state.list",
        "room.state.read",
    ] {
        let error = server
            .call(CallToolRequestParams::new(name))
            .await
            .expect_err("Room-only tool must not be callable by Normal worker");
        assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }
}
#[tokio::test]
async fn changing_live_toolsets_updates_surface_and_authorization() {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let initial_names = server
        .current_tools()
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    assert!(!initial_names.iter().any(|name| name == "bootstrap"));
    assert!(!initial_names.iter().any(|name| name.starts_with("room.")));

    let error = server
        .call(CallToolRequestParams::new("bootstrap"))
        .await
        .expect_err("Room tools must be unavailable before enabling Room");
    assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);

    let mut room_config = server.state.config.read().await.clone();
    room_config.toolsets = ToolsetConfig::room();
    *server.state.config.write().await = room_config;
    let room_names = server
        .current_tools()
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    assert!(room_names.iter().any(|name| name == "bootstrap"));
    assert!(room_names.iter().any(|name| name == "room.diary.active"));
    let bootstrap = server
        .call(CallToolRequestParams::new("bootstrap"))
        .await
        .expect("enabled Room tool should reach the read path");
    assert!(bootstrap.get("entrypoint").is_some());

    let mut normal_config = server.state.config.read().await.clone();
    normal_config.toolsets = ToolsetConfig::normal();
    *server.state.config.write().await = normal_config;
    let error = server
        .call(CallToolRequestParams::new("bootstrap"))
        .await
        .expect_err("Room tools must disappear after disabling Room");
    assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
}

#[tokio::test]
async fn stdio_resumes_stale_logical_session_before_first_tool_call() -> anyhow::Result<()> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, mut client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });
    let mut client_read = BufReader::new(client_read);

    client_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await?;
    client_write
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"tools/call\",\"params\":{\"name\":\"agent.info\",\"arguments\":{}}}\n",
            )
            .await?;
    client_write.flush().await?;

    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client_read.read_line(&mut line),
    )
    .await??;
    let response: Value = serde_json::from_str(&line)?;
    assert_eq!(response["id"], 0);
    assert!(response.get("error").is_none());
    assert_eq!(
        response["result"]["structuredContent"]["identity"]["profile"],
        "normal"
    );
    assert!(
        !line.contains("agentic-gpt-internal-init-"),
        "private initialize response leaked into the tunnel stream"
    );

    client_write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n")
        .await?;
    client_write.flush().await?;
    line.clear();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client_read.read_line(&mut line),
    )
    .await??;
    let follow_up: Value = serde_json::from_str(&line)?;
    assert_eq!(follow_up["id"], 1);
    assert!(follow_up["result"]["tools"].is_array());

    drop(client_write);
    drop(client_read);
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn in_process_stdio_initialize_list_and_call() -> anyhow::Result<()> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });

    let client = ().serve((client_read, client_write)).await?;
    let tools = client.list_all_tools().await?;
    assert!(tools.iter().any(|tool| tool.name == "process.list"));
    let result = client
        .call_tool(CallToolRequestParams::new("process.list"))
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content.as_ref().unwrap()["processes"],
        json!([])
    );
    assert!(result
        .structured_content
        .as_ref()
        .unwrap()
        .get("jobs")
        .is_none());
    let skills = client
        .call_tool(CallToolRequestParams::new("skills.list"))
        .await?;
    assert_eq!(skills.is_error, Some(false));
    let bootstrap = client
        .call_tool(CallToolRequestParams::new("bootstrap"))
        .await;
    assert!(bootstrap.is_err());
    let _ = client.cancel().await;
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn in_process_room_stdio_initialize_list_and_call() -> anyhow::Result<()> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });

    let client = ().serve((client_read, client_write)).await?;
    let tools = client.list_all_tools().await?;
    assert!(tools.iter().any(|tool| tool.name == "room.diary.active"));
    let result = client
        .call_tool(CallToolRequestParams::new("room.diary.active"))
        .await?;
    assert_eq!(result.is_error, Some(false));
    assert!(result.structured_content.as_ref().unwrap()["daily"].is_object());
    let _ = client.cancel().await;
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn process_tools_reject_legacy_identity_and_confirmation_fields() {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let names = server
        .current_tools()
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<BTreeSet<_>>();
    for name in [
        "process.exec",
        "process.batch",
        "process.status",
        "process.list",
        "process.output",
        "process.result",
        "process.cancel",
    ] {
        assert!(names.contains(name), "missing process API {name}");
    }
    for name in ["job.get", "job.list", "job.cancel"] {
        assert!(!names.contains(name), "legacy API is advertised: {name}");
        let removed = server
            .call(CallToolRequestParams::new(name))
            .await
            .expect_err("Legacy lifecycle APIs must not be callable");
        assert_eq!(removed.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }

    let identity = server
        .call(
            CallToolRequestParams::new("process.exec").with_arguments(Map::from_iter([
                ("program".to_string(), Value::String("true".to_string())),
                (
                    "agentId".to_string(),
                    Value::String("stdio-test-agent".to_string()),
                ),
            ])),
        )
        .await
        .expect_err("Tunnel process schemas must reject agentId");
    assert_eq!(identity.code, rmcp::model::ErrorCode::INVALID_PARAMS);

    let confirmation = server
        .call(
            CallToolRequestParams::new("process.exec").with_arguments(Map::from_iter([
                ("program".to_string(), Value::String("true".to_string())),
                (
                    "confirmMethod".to_string(),
                    Value::String("hub".to_string()),
                ),
            ])),
        )
        .await
        .expect_err("Tunnel process schemas must reject confirmMethod");
    assert_eq!(confirmation.code, rmcp::model::ErrorCode::INVALID_PARAMS);

    for removed_name in [
        "session.start",
        "session.list",
        "session.inspect",
        "session.wait",
        "session.kill",
        "process.batchExec",
        "process.get",
        "process.kill",
    ] {
        let removed = server
            .call(CallToolRequestParams::new(removed_name))
            .await
            .expect_err("Removed lifecycle aliases must not be callable");
        assert_eq!(removed.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }
}

#[tokio::test]
async fn process_creation_status_cancel_and_batch_use_process_api() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let quick = server
        .dispatch("process.exec", json!({"program": "true", "waitSeconds": 5}))
        .await?;
    assert_eq!(quick["state"], "completed");
    assert_eq!(quick["status"], "completed");
    assert_eq!(quick["completedInline"], true);
    assert!(quick["processId"]
        .as_str()
        .is_some_and(|id| id.starts_with("process_")));
    assert!(serde_json::to_vec(&quick)?.len() <= 8 * 1024);
    let process_id = quick["processId"].as_str().unwrap().to_string();

    let status = server
        .dispatch(
            "process.status",
            json!({"processId": process_id, "waitSeconds": 0}),
        )
        .await?;
    assert_eq!(status["processId"], quick["processId"]);
    assert_eq!(status["kind"], "command");
    assert_eq!(status["state"], "completed");
    for body_field in [
        "stdout",
        "stderr",
        "output",
        "inlineOutput",
        "outputPreview",
        "result",
    ] {
        assert!(
            status.get(body_field).is_none(),
            "process.status unexpectedly included {body_field}"
        );
    }

    let long = server
        .dispatch(
            "process.exec",
            json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
        )
        .await?;
    let long_id = long["processId"].as_str().unwrap().to_string();
    for _ in 0..100 {
        let state = server
            .dispatch(
                "process.status",
                json!({"processId": long_id, "waitSeconds": 0}),
            )
            .await?;
        if state["state"] == "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let listed = server
        .dispatch("process.list", json!({"kind": "command"}))
        .await?;
    assert!(listed["processes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|process| process["processId"] == long_id));
    assert!(listed.get("jobs").is_none());
    let cancelled = server
        .dispatch("process.cancel", json!({"processId": long_id}))
        .await?;
    assert_eq!(cancelled["processId"], long_id);
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["cancelOutcome"], "cancelled");
    assert_eq!(
        cancelled["terminationEvidence"],
        "local_process_kill_completed"
    );

    let batch = server
        .dispatch(
            "process.batch",
            json!({
                "elements": [
                    {"program": "true"},
                    {"program": "false"}
                ],
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(batch["processes"].as_array().unwrap().len(), 2);
    assert_eq!(batch["status"], "completed_with_errors");
    assert_eq!(batch["processes"][0]["state"], "completed");
    assert_eq!(batch["processes"][1]["state"], "failed");
    assert!(batch.get("jobs").is_none());

    let rejected = server
        .dispatch(
            "process.batch",
            json!({
                "elements": [
                    {"program": "true"},
                    {"program": "true", "workingDirectory": "/missing"}
                ],
                "waitSeconds": 0
            }),
        )
        .await?;
    assert_eq!(rejected["error"]["code"], "process_batch_rejected");
    assert!(rejected.get("processes").is_none());
    Ok(())
}

#[tokio::test]
async fn process_status_omitted_waits_but_zero_is_nonblocking() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let started = server
        .dispatch(
            "process.exec",
            json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
        )
        .await?;
    let process_id = started["processId"].as_str().unwrap().to_string();

    for _ in 0..100 {
        let state = server
            .dispatch(
                "process.status",
                json!({"processId": process_id, "waitSeconds": 0}),
            )
            .await?;
        if is_active_process_state(state["state"].as_str().unwrap()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let explicit_zero = server
        .dispatch(
            "process.status",
            json!({"processId": process_id, "waitSeconds": 0}),
        )
        .await?;
    assert!(
        is_active_process_state(explicit_zero["state"].as_str().unwrap()),
        "explicit zero wait must return the still-running process"
    );

    let omitted = server
        .dispatch("process.status", json!({"processId": process_id}))
        .await?;
    assert_eq!(
        omitted["state"], "completed",
        "omitted wait must use the five-second default"
    );
    Ok(())
}

#[tokio::test]
async fn process_shapes_are_metadata_only_and_keep_group_filters() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let quick = server
        .dispatch(
            "process.exec",
            json!({
                "program": "true",
                "group": "  workstream  ",
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(quick["state"], "completed");
    assert_eq!(quick["group"], "workstream");
    assert_eq!(quick["kind"], "command");
    assert!(quick["processId"]
        .as_str()
        .is_some_and(|id| id.starts_with("process_")));
    let process_id = quick["processId"].as_str().unwrap().to_string();

    let ordinary = server
        .dispatch(
            "process.status",
            json!({"processId": process_id, "waitSeconds": 0}),
        )
        .await?;
    assert_eq!(ordinary["group"], "workstream");
    assert_eq!(ordinary["kind"], "command");
    assert_eq!(ordinary["state"], "completed");
    assert!(ordinary.get("createdAt").is_some());
    assert!(ordinary.get("finishedAt").is_some());
    assert!(ordinary.get("program").is_some());
    assert!(ordinary.get("result").is_none());
    assert!(ordinary.get("inlineOutput").is_none());
    assert!(ordinary.get("outputPreview").is_none());

    let listed = server
        .dispatch(
            "process.list",
            json!({"group": "workstream", "kind": "command", "limit": 1}),
        )
        .await?;
    assert!(listed.get("nextCursor").is_none());
    assert_eq!(listed["processes"].as_array().unwrap().len(), 1);
    assert_eq!(listed["processes"][0]["group"], "workstream");
    assert_eq!(listed["processes"][0]["kind"], "command");
    assert_eq!(listed["processes"][0]["processId"], ordinary["processId"]);
    assert!(listed["processes"][0]["createdAt"].is_string());
    assert!(listed["processes"][0].get("program").is_none());

    let running = server
        .dispatch(
            "process.exec",
            json!({"program": "sleep", "args": ["2"], "waitSeconds": 0}),
        )
        .await?;
    let running_id = running["processId"].as_str().unwrap().to_string();
    let wait_status = server
        .dispatch(
            "process.status",
            json!({"processId": running_id, "waitSeconds": 1}),
        )
        .await?;
    assert!(is_active_process_state(
        wait_status["state"].as_str().unwrap()
    ));
    let ordinary_zero = server
        .dispatch(
            "process.status",
            json!({"processId": running_id, "waitSeconds": 0}),
        )
        .await?;
    assert_eq!(ordinary_zero["processId"], running["processId"]);
    assert!(is_active_process_state(
        ordinary_zero["state"].as_str().unwrap()
    ));
    assert!(ordinary_zero.get("createdAt").is_some());
    let _ = server
        .dispatch("process.cancel", json!({"processId": running_id}))
        .await?;

    let rejected = server
        .dispatch("process.exec", json!({"program": "vim", "waitSeconds": 5}))
        .await?;
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(rejected["error"]["code"], "requires_tty_not_supported");
    assert!(rejected.get("rejectReason").is_none());
    assert!(rejected.get("durationMs").is_none());

    let failed = server
        .dispatch(
            "process.exec",
            json!({"program": "false", "waitSeconds": 5}),
        )
        .await?;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["exitCode"], 1);
    assert!(failed.get("error").is_none());
    Ok(())
}

#[tokio::test]
async fn process_output_pages_preserve_raw_byte_offsets_and_content() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    allow_test_printf(&server).await;
    let expected = "AéBC".repeat(20);
    let started = server
        .dispatch(
            "process.exec",
            json!({
                "program": "printf",
                "args": ["%s", expected],
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(started["state"], "completed");
    let process_id = started["processId"].as_str().unwrap().to_string();

    let mut cursor: Option<String> = None;
    let mut stdout_bytes = Vec::new();
    let mut stdout_offset = 0u64;
    let mut pages = 0usize;
    loop {
        let mut request = json!({"processId": process_id, "maxBytes": 16});
        if let Some(cursor) = &cursor {
            request["cursor"] = json!(cursor);
        }
        let page = server.dispatch("process.output", request).await?;
        assert_eq!(page["processId"], process_id);
        let stdout = &page["stdout"];
        let start = stdout["startOffset"].as_str().unwrap().parse::<u64>()?;
        let end = stdout["endOffset"].as_str().unwrap().parse::<u64>()?;
        assert_eq!(start, stdout_offset);
        let data = stdout["data"].as_str().unwrap();
        let decoded = match stdout["encoding"].as_str().unwrap() {
            "utf8" => data.as_bytes().to_vec(),
            "base64" => base64::engine::general_purpose::STANDARD.decode(data)?,
            encoding => panic!("unexpected process.output encoding {encoding}"),
        };
        assert_eq!(end - start, decoded.len() as u64);
        stdout_offset = end;
        if !decoded.is_empty() {
            pages += 1;
            assert!(pages < 50, "process.output cursor did not reach EOF");
        }
        stdout_bytes.extend(decoded);

        let next = page["nextCursor"].as_str().unwrap().to_string();
        if page["hasMore"] == true {
            assert_ne!(cursor.as_deref(), Some(next.as_str()));
            cursor = Some(next);
        } else if page["eof"] == true {
            break;
        } else {
            cursor = Some(next);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    assert!(pages > 1, "the output should require multiple pages");
    assert_eq!(stdout_offset, expected.len() as u64);
    assert_eq!(stdout_bytes.as_slice(), expected.as_bytes());
    Ok(())
}

#[tokio::test]
async fn process_creation_and_batch_responses_obey_inline_budget() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    allow_test_printf(&server).await;

    let large = server
        .dispatch(
            "process.exec",
            json!({
                "program": "printf",
                "args": ["%12000s", ""],
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(large["state"], "completed");
    assert_eq!(large["completedInline"], false);
    assert!(large.get("inlineOutput").is_none());
    assert_eq!(large["outputPreview"]["truncated"], true);
    assert!(large["outputPreview"]["stdout"].as_str().unwrap().len() <= 2048);
    assert!(large["outputPreview"]["stderr"].as_str().unwrap().len() <= 2048);
    assert!(serde_json::to_vec(&large)?.len() <= 8 * 1024);

    let batch = server
        .dispatch(
            "process.batch",
            json!({
                "elements": [
                    {"program": "printf", "args": ["%5000s", ""]},
                    {"program": "printf", "args": ["%5000s", ""]}
                ],
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(batch["processes"].as_array().unwrap().len(), 2);
    assert!(serde_json::to_vec(&batch)?.len() <= 8 * 1024);
    Ok(())
}

#[test]
fn process_input_schemas_advertise_wait_bounds_and_paging_limits() {
    for name in [
        "process.exec",
        "process.batch",
        "mcp.callTool",
        "mcp.batch",
        "skills.run",
    ] {
        let descriptor = serde_json::to_value(tool_descriptor(name)).unwrap();
        assert!(descriptor["inputSchema"]["properties"]["group"].is_object());
        assert_eq!(
            descriptor["inputSchema"]["properties"]["group"]["maxLength"],
            32
        );
    }
    let status = serde_json::to_value(tool_descriptor("process.status")).unwrap();
    assert_eq!(
        status["inputSchema"]["properties"]["waitSeconds"]["default"],
        5
    );
    assert_eq!(
        status["inputSchema"]["properties"]["waitSeconds"]["minimum"],
        0
    );
    assert_eq!(
        status["inputSchema"]["properties"]["waitSeconds"]["maximum"],
        30
    );
    assert!(status["inputSchema"]["properties"]
        .get("waitOnly")
        .is_none());
    let output = serde_json::to_value(tool_descriptor("process.output")).unwrap();
    assert_eq!(
        output["inputSchema"]["properties"]["maxBytes"]["default"],
        8192
    );
    assert_eq!(
        output["inputSchema"]["properties"]["maxBytes"]["maximum"],
        32768
    );
    let result = serde_json::to_value(tool_descriptor("process.result")).unwrap();
    assert_eq!(
        result["inputSchema"]["properties"]["maxBytes"]["default"],
        8192
    );
    assert_eq!(
        result["inputSchema"]["properties"]["maxBytes"]["maximum"],
        512 * 1024
    );
    let list = serde_json::to_value(tool_descriptor("process.list")).unwrap();
    for field in ["group", "kind", "state", "limit", "cursor"] {
        assert!(
            list["inputSchema"]["properties"][field].is_object(),
            "{field}"
        );
    }
}

#[test]
fn mcp_batch_projection_keeps_result_availability_without_bodies() -> anyhow::Result<()> {
    let mut retained_process = test_terminal_process();
    retained_process.process_id = "process_testboot_retained".to_string();
    retained_process.kind = agentic_gpt_protocol::ProcessKind::Mcp;
    retained_process.mcp_server_id = Some("local".to_string());
    retained_process.mcp_tool_name = Some("lookup".to_string());
    let retained_result = json!({"answer": 42});
    let retained_detail = ProcessDetail {
        process: retained_process,
        detail_available: true,
        result: Some(retained_result),
        error: None,
        result_available: true,
        result_bytes: Some(13),
        result_sha256: Some("sha256:retained".to_string()),
        result_preview: Some("{\"answer\":42}".to_string()),
    };

    let mut unavailable_process = test_terminal_process();
    unavailable_process.process_id = "process_testboot_unavailable".to_string();
    unavailable_process.kind = agentic_gpt_protocol::ProcessKind::Mcp;
    unavailable_process.mcp_server_id = Some("local".to_string());
    unavailable_process.mcp_tool_name = Some("lookup".to_string());
    let unavailable_detail = ProcessDetail {
        process: unavailable_process,
        detail_available: true,
        result: None,
        error: None,
        result_available: false,
        result_bytes: None,
        result_sha256: None,
        result_preview: None,
    };

    let value = slim_mcp_batch_response(
        McpBatchResponse {
            batch_id: "batch_test".to_string(),
            status: McpBatchStatus::Completed,
            completed_inline: true,
            poll_after_ms: 0,
            results: vec![
                McpBatchChildResponse {
                    index: 0,
                    id: Some("retained".to_string()),
                    result_omitted: false,
                    process: retained_detail,
                },
                McpBatchChildResponse {
                    index: 1,
                    id: Some("unavailable".to_string()),
                    result_omitted: false,
                    process: unavailable_detail,
                },
            ],
            aggregate_truncated: false,
            aggregate_bytes: None,
            error: None,
        },
        None,
    )?;
    assert!(value.get("batchId").is_none());
    assert!(value.get("completedInline").is_none());
    assert!(value.get("pollAfterMs").is_none());
    assert!(value.get("aggregateTruncated").is_none());
    assert!(value.get("aggregateBytes").is_none());
    assert_eq!(value["results"][0]["resultAvailable"], true);
    assert_eq!(value["results"][0]["resultStatus"], "complete");
    assert_eq!(value["results"][0]["resultBytes"], 13);
    assert_eq!(value["results"][1]["resultAvailable"], false);
    assert_eq!(value["results"][1]["resultStatus"], "unavailable");
    for child in value["results"].as_array().unwrap() {
        assert!(child.get("result").is_none());
        assert!(child.get("resultOmitted").is_none());
        assert!(child.get("index").is_none());
        assert!(child.get("id").is_none());
    }
    Ok(())
}

#[tokio::test]
async fn process_result_preserves_retained_mcp_payloads_and_reports_unavailable(
) -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let payload = json!({"answer": "retained"});
    let mut retained_process = test_terminal_process();
    retained_process.process_id = "process_testboot_result".to_string();
    retained_process.kind = agentic_gpt_protocol::ProcessKind::Mcp;
    retained_process.mcp_server_id = Some("local".to_string());
    retained_process.mcp_tool_name = Some("lookup".to_string());
    retained_process.capture_status = agentic_gpt_protocol::ProcessCaptureStatus::NotApplicable;
    let retained_detail = ProcessDetail {
        process: retained_process,
        detail_available: true,
        result: Some(payload.clone()),
        error: None,
        result_available: true,
        result_bytes: Some(serde_json::to_vec(&payload)?.len()),
        result_sha256: Some("sha256:retained".to_string()),
        result_preview: Some("{\"answer\":\"retained\"}".to_string()),
    };
    assert!(server
        .state
        .process_history
        .upsert_terminal(
            &retained_detail,
            &crate::process_history::ProcessOutputSnapshot::default(),
        )
        .is_persisted());

    let retained = server
        .dispatch(
            "process.result",
            json!({"processId": "process_testboot_result"}),
        )
        .await?;
    assert_eq!(retained["status"], "complete");
    assert_eq!(retained["resultAvailable"], true);
    assert_eq!(retained["result"], payload);

    let too_small = server
        .dispatch(
            "process.result",
            json!({"processId": "process_testboot_result", "maxBytes": 1}),
        )
        .await?;
    assert_eq!(too_small["status"], "too_large");
    assert_eq!(too_small["resultAvailable"], true);
    assert!(too_small.get("result").is_none());

    let mut unavailable_process = test_terminal_process();
    unavailable_process.process_id = "process_testboot_unavailable".to_string();
    unavailable_process.kind = agentic_gpt_protocol::ProcessKind::Mcp;
    unavailable_process.mcp_server_id = Some("local".to_string());
    unavailable_process.mcp_tool_name = Some("lookup".to_string());
    unavailable_process.capture_status = agentic_gpt_protocol::ProcessCaptureStatus::NotApplicable;
    let unavailable_detail = ProcessDetail {
        process: unavailable_process,
        detail_available: true,
        result: None,
        error: None,
        result_available: false,
        result_bytes: None,
        result_sha256: None,
        result_preview: None,
    };
    assert!(server
        .state
        .process_history
        .upsert_terminal(
            &unavailable_detail,
            &crate::process_history::ProcessOutputSnapshot::default(),
        )
        .is_persisted());
    let unavailable = server
        .dispatch(
            "process.result",
            json!({"processId": "process_testboot_unavailable"}),
        )
        .await?;
    assert_eq!(unavailable["status"], "unavailable");
    assert_eq!(unavailable["resultAvailable"], false);
    assert!(unavailable.get("result").is_none());
    Ok(())
}

#[tokio::test]
async fn managed_batch_uses_one_confirmation_for_all_elements() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    {
        let mut config = server.state.config.write().await;
        config.confirmation_provider =
            crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    }
    let (sender, mut receiver) = mpsc::unbounded_channel();
    *server.state.hub_sender.lock().await = Some(sender);
    let confirmation_count = Arc::new(AtomicUsize::new(0));
    let confirmation_count_clone = confirmation_count.clone();
    let response_state = server.state.clone();
    let responder = tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            if let AgentMessage::ConfirmationRequest { request_id, .. } = message {
                confirmation_count_clone.fetch_add(1, Ordering::SeqCst);
                if let Some(sender) = response_state
                    .pending_confirmations
                    .lock()
                    .await
                    .remove(&request_id)
                {
                    let _ = sender.send("allow_once".to_string());
                }
            }
        }
    });
    let batch = server
        .dispatch(
            "process.batch",
            json!({
                "elements": [{"program": "true"}, {"program": "true"}],
                "needConfirm": true,
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(batch["status"], json!("completed"));
    assert_eq!(confirmation_count.load(Ordering::SeqCst), 1);
    let audit = std::fs::read_to_string(
        server
            .state
            .config
            .read()
            .await
            .workspace_root
            .join(".agentic-gpt-audit.jsonl"),
    )?;
    assert_eq!(audit.lines().count(), 2);
    assert!(audit.lines().all(|line| {
        line.contains("\"policyDecision\":\"Confirm\"")
            && line.contains("\"confirmationResult\":\"allow_once\"")
    }));
    responder.abort();
    Ok(())
}

#[tokio::test]
async fn denied_process_batch_creates_no_processes() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    {
        let mut config = server.state.config.write().await;
        config.confirmation_provider =
            crate::config::ConfirmationProviderConfig::from_legacy("hub").unwrap();
    }
    let (sender, mut receiver) = mpsc::unbounded_channel();
    *server.state.hub_sender.lock().await = Some(sender);
    let response_state = server.state.clone();
    let responder = tokio::spawn(async move {
        if let Some(AgentMessage::ConfirmationRequest { request_id, .. }) = receiver.recv().await {
            if let Some(sender) = response_state
                .pending_confirmations
                .lock()
                .await
                .remove(&request_id)
            {
                let _ = sender.send("deny".to_string());
            }
        }
    });
    let batch = server
        .dispatch(
            "process.batch",
            json!({
                "elements": [{"program": "true"}, {"program": "true"}],
                "needConfirm": true,
                "waitSeconds": 5
            }),
        )
        .await?;
    assert_eq!(batch["error"]["code"], "process_batch_rejected");
    let processes = server.dispatch("process.list", json!({})).await?;
    assert_eq!(processes["processes"], json!([]));
    responder.abort();
    Ok(())
}

#[tokio::test]
async fn tmux_actions_reject_incompatible_fields() {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let invalid = [
        json!({"action": "list", "needConfirm": false}),
        json!({"action": "create", "name": "demo", "cwd": ".", "needConfirm": false}),
        json!({"action": "close", "name": "demo", "cwd": "."}),
    ];
    for arguments in invalid {
        assert!(server.dispatch("tmux.sessions", arguments).await.is_err());
    }
    for arguments in [
        json!({"action": "list", "target": "%0"}),
        json!({"action": "capture", "target": "%0", "session": "demo"}),
    ] {
        assert!(server.dispatch("tmux.panes", arguments).await.is_err());
    }
}

#[test]
fn managed_terminal_event_includes_duration() {
    let started_at = Utc::now() - chrono::Duration::milliseconds(42);
    let mut process = test_terminal_process();
    process.created_at = started_at;
    process.started_at = Some(started_at);
    process.updated_at = started_at + chrono::Duration::milliseconds(42);
    process.finished_at = Some(process.updated_at);
    process.args = vec!["sentinel-argument".to_string()];
    process.working_directory = Some("/sentinel/path".to_string());
    process.command_preview = Some("true sentinel-argument".to_string());
    let message = managed_terminal_event_message("normal", "tunnel:process.exec", &process);
    assert!(message.contains("durationMs=42"));
    let human_process_id = message
        .split("process=")
        .nth(1)
        .and_then(|value| value.split(';').next())
        .unwrap();
    assert_eq!(human_process_id.len(), 12);
    assert!(!message.contains("processId="));
    assert!(!message.contains("sentinel"));
}

#[test]
fn inline_terminal_tracker_discards_pending_terminal_event() {
    let emitted = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let tracker = HumanTerminalTracker::with_emitter(move |message| {
        sink.lock().push(message);
    });
    let process = test_terminal_process();
    tracker.record("normal", "tunnel:process.exec", &process);
    assert_eq!(tracker.state.lock().unwrap().pending.len(), 1);
    tracker.finish_response(true, None);
    assert!(tracker.state.lock().unwrap().pending.is_empty());
    assert!(matches!(
        tracker.state.lock().unwrap().response,
        HumanResponseState::Inline
    ));
    assert!(emitted.lock().is_empty());
}

#[test]
fn active_response_precedes_terminal_for_both_serial_orderings() {
    for terminal_first in [true, false] {
        let emitted = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = emitted.clone();
        let tracker = HumanTerminalTracker::with_emitter(move |message| {
            sink.lock().push(message);
        });
        let process = test_terminal_process();
        if terminal_first {
            tracker.record("normal", "tunnel:process.exec", &process);
            tracker.finish_response(false, Some("status=active".to_string()));
        } else {
            tracker.finish_response(false, Some("status=active".to_string()));
            tracker.record("normal", "tunnel:process.exec", &process);
        }
        let emitted = emitted.lock();
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0], "status=active");
        assert!(emitted[1].starts_with("managed_process;"));
    }
}

#[test]
fn concurrent_check_clear_enqueue_interleaving_is_linearizable() {
    let emitted = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let tracker = Arc::new(HumanTerminalTracker::with_emitter(move |message| {
        sink.lock().push(message);
    }));
    let process = test_terminal_process();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        let record_tracker = tracker.clone();
        let record_barrier = barrier.clone();
        scope.spawn(move || {
            record_barrier.wait();
            record_tracker.record("normal", "tunnel:process.exec", &process);
        });
        let response_tracker = tracker.clone();
        scope.spawn(move || {
            barrier.wait();
            response_tracker.finish_response(false, Some("status=active".to_string()));
        });
    });
    let emitted = emitted.lock();
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted[0], "status=active");
    assert_eq!(emitted[1].matches("managed_process;").count(), 1);
}

fn test_terminal_process() -> agentic_gpt_protocol::ProcessInfo {
    let now = Utc::now();
    agentic_gpt_protocol::ProcessInfo {
        agent_id: "agent".to_string(),
        process_id: "process_testboot_0123456789abcdef".to_string(),
        group: None,
        batch_id: None,
        batch_call_id: None,
        batch_index: None,
        kind: agentic_gpt_protocol::ProcessKind::Command,
        state: agentic_gpt_protocol::ProcessState::Completed,
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        finished_at: Some(now),
        program: Some("true".to_string()),
        args: Vec::new(),
        working_directory: None,
        command_preview: Some("true".to_string()),
        exit_code: Some(0),
        reject_reason: None,
        skill_id: None,
        skill_path: None,
        installed_digest: None,
        mcp_server_id: None,
        mcp_tool_name: None,
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
        capture_status: agentic_gpt_protocol::ProcessCaptureStatus::Complete,
        capture_error: None,
    }
}

#[test]
fn batch_lifecycle_detection_reads_process_envelopes() {
    let active = json!({
        "processes": [{"processId": "process_a", "state": "running"}]
    });
    assert!(value_has_active_process(&active));
    assert!(!value_has_terminal_failure(&active));

    let failed = json!({
        "processes": [{
            "processId": "process_b",
            "state": "failed",
            "rejectReason": "spawn_failed"
        }]
    });
    assert!(!value_has_active_process(&failed));
    assert!(value_has_terminal_failure(&failed));
    assert_eq!(
        human_failure_reason(&failed, None).as_deref(),
        Some("spawn_failed")
    );
}

#[tokio::test]
async fn tunnel_local_and_http_ingress_advertise_identical_surface() {
    let state = test_state(CapabilityProfile::Normal);
    let tunnel = AgentMcpServer::with_ingress(state.clone(), RequestIngress::TunnelStdio);
    let local = AgentMcpServer::with_ingress(state.clone(), RequestIngress::LocalUnix);
    let http = AgentMcpServer::with_ingress(state, RequestIngress::Http);
    let tunnel_tools = serde_json::to_value(tunnel.current_tools().await).unwrap();
    assert_eq!(
        tunnel_tools,
        serde_json::to_value(local.current_tools().await).unwrap()
    );
    assert_eq!(
        tunnel_tools,
        serde_json::to_value(http.current_tools().await).unwrap()
    );
    assert_eq!(tunnel.ingress.label(), "tunnel:stdio");
    assert_eq!(local.ingress.label(), "local:unix");
    assert_eq!(http.ingress.label(), "http:mcp");
    assert_eq!(http.ingress.source("agent.info"), "http:agent.info");
}

#[tokio::test]
async fn local_skill_audit_uses_local_request_source() -> anyhow::Result<()> {
    let server = AgentMcpServer::with_ingress(
        test_state(CapabilityProfile::Normal),
        RequestIngress::LocalUnix,
    );
    let workspace = server.state.config.read().await.workspace_root.clone();
    let scripts = workspace.join("skills/demo/scripts");
    std::fs::create_dir_all(&scripts)?;
    std::fs::write(workspace.join("skills/demo/SKILL.md"), "# Demo\n")?;
    let script = scripts.join("check.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf done\n")?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    crate::skills::activate(
        &server.state,
        SkillActivationRequest {
            id: "demo".to_string(),
        },
    )
    .await?;
    let result = server
        .dispatch(
            "skills.run",
            json!({"id": "demo", "path": "scripts/check.sh", "waitSeconds": 5}),
        )
        .await?;
    assert_eq!(result["state"], "completed");
    assert_eq!(result["kind"], "skill");
    let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
    assert!(audit.contains("\"requestSource\":\"local:skills.run\""));
    assert!(!audit.contains("\"requestSource\":\"tunnel:skills.run\""));
    Ok(())
}

#[tokio::test]
async fn local_mcp_call_audit_uses_local_request_source() -> anyhow::Result<()> {
    let server = AgentMcpServer::with_ingress(
        test_state(CapabilityProfile::Normal),
        RequestIngress::LocalUnix,
    );
    let workspace = server.state.config.read().await.workspace_root.clone();
    let result = server
        .dispatch(
            "mcp.callTool",
            json!({
                "serverId": "missing",
                "toolName": "noop",
                "arguments": {}
            }),
        )
        .await?;
    assert_eq!(result["state"], "rejected");
    assert_eq!(result["kind"], "mcp");
    assert_eq!(result["error"]["code"], "mcp_server_not_found");
    let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
    assert!(audit.contains("\"requestSource\":\"local:mcp.callTool\""));
    assert!(audit.contains("\"terminalState\":\"rejected\""));
    assert!(audit.contains("\"terminationEvidence\":\"server_config_validation\""));
    assert!(!audit.contains("\"requestSource\":\"hub:mcp\""));
    Ok(())
}

#[tokio::test]
async fn tunnel_skill_audit_uses_tunnel_request_source() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let scripts = workspace.join("skills/demo/scripts");
    std::fs::create_dir_all(&scripts)?;
    std::fs::write(workspace.join("skills/demo/SKILL.md"), "# Demo\n")?;
    let script = scripts.join("check.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf done\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::skills::activate(
        &server.state,
        SkillActivationRequest {
            id: "demo".to_string(),
        },
    )
    .await?;
    let result = server
        .dispatch(
            "skills.run",
            json!({"id": "demo", "path": "scripts/check.sh", "waitSeconds": 5}),
        )
        .await?;
    assert_eq!(result["state"], "completed");
    assert_eq!(result["kind"], "skill");
    let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?;
    assert!(audit.contains("\"requestSource\":\"tunnel:skills.run\""));
    Ok(())
}

#[tokio::test]
async fn wp2_hub_normal_direct_skills_require_capability() -> anyhow::Result<()> {
    let mut state = test_state(CapabilityProfile::Normal);
    state.runtime = RuntimeModel::hub(CapabilityProfile::Normal);
    let server = AgentMcpServer::new(state);

    let listed = server.dispatch("skills.list", json!({})).await?;
    assert_eq!(listed["error"]["code"], "room_agent_required");
    assert!(listed.get("skills").is_none());
    let read = server
        .dispatch("skills.read", json!({"id": "missing"}))
        .await?;
    assert_eq!(read["error"]["code"], "room_agent_required");
    assert!(read.get("skill").is_none());
    Ok(())
}

#[tokio::test]
async fn room_profile_dispatches_room_memory_tools() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
    let bootstrap = server.dispatch("bootstrap", json!({})).await?;
    assert!(bootstrap.get("entrypoint").is_some());
    let skills = server.dispatch("skills.list", json!({})).await?;
    assert!(skills["skills"].is_array());
    let notebook = server.dispatch("room.notebook.recent", json!({})).await?;
    assert!(notebook["documents"].is_array());
    let diary = server.dispatch("room.diary.active", json!({})).await?;
    assert!(diary["daily"].is_object());
    let state = server.dispatch("room.state.list", json!({})).await?;
    assert!(state["entities"].is_array());
    Ok(())
}

#[tokio::test]
async fn every_room_adapter_rejects_unknown_identity_fields() {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Room));
    let cases = [
        ("room.diary.active", json!({"agentId":"foreign"})),
        (
            "room.diary.read",
            json!({"layer":"daily","period":"current","agentId":"foreign"}),
        ),
        ("room.notebook.recent", json!({"agentId":"foreign"})),
        (
            "room.notebook.search",
            json!({"query":"x","agentId":"foreign"}),
        ),
        (
            "room.notebook.read",
            json!({"path":"Notebook/topic.md","agentId":"foreign"}),
        ),
        ("room.state.list", json!({"agentId":"foreign"})),
        (
            "room.state.read",
            json!({"entity":"project","agentId":"foreign"}),
        ),
    ];
    for (name, arguments) in cases {
        assert!(
            server.dispatch(name, arguments).await.is_err(),
            "{name} must reject unknown identity fields"
        );
    }
}

#[tokio::test]
async fn file_read_dispatch_supports_content_and_metadata_modes() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("read-me.txt"), "first\nsecond\n")?;
    let content = server
        .dispatch(
            "file.read",
            json!({"path":"read-me.txt", "startLine": 2, "endLine": 2}),
        )
        .await?;
    assert_eq!(content["content"], "second\n");
    assert!(content.get("metadata").is_none());
    let metadata = server
        .dispatch("file.read", json!({"path":"read-me.txt", "metadata": true}))
        .await?;
    assert_eq!(metadata["content"], "first\nsecond\n");
    assert_eq!(metadata["metadata"]["totalLines"], 2);
    assert!(metadata.get("revision").is_none());
    let missing = server
        .dispatch("file.read", json!({"path":"missing.txt"}))
        .await?;
    assert_eq!(missing["error"]["code"], "file_not_found");
    Ok(())
}

#[tokio::test]
async fn in_process_stdio_file_read_batch_descriptor_and_call_contract() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("stdio-batch.txt"), "needle\n")?;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });

    let client = ().serve((client_read, client_write)).await?;
    let tools = client.list_all_tools().await?;
    let read_descriptor = tools.iter().find(|tool| tool.name == "file.read").unwrap();
    let read_schema = serde_json::to_value(read_descriptor)?;
    assert!(read_schema["inputSchema"]["properties"]["requests"].is_object());
    assert!(read_schema["inputSchema"].get("oneOf").is_none());
    let removed_tool = ["file", "batch"].join(".");
    assert!(!tools.iter().any(|tool| tool.name == removed_tool));

    let result = client
        .call_tool(
            CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                "requests".to_string(),
                json!([{"path":"stdio-batch.txt"},{"path":"missing.txt"}]),
            )])),
        )
        .await?;
    let value = result.structured_content.unwrap();
    assert_eq!(value["results"][0]["status"], "completed");
    assert_eq!(value["results"][0]["result"]["content"], "needle\n");
    assert_eq!(value["results"][1]["status"], "failed");
    assert!(value["results"][0]["result"].get("revision").is_none());
    let _ = client.cancel().await;
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn in_process_stdio_file_read_projects_static_and_gif_images() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let static_fixtures = [
        ("sniffed-png.txt", image::ImageFormat::Png, "image/png"),
        ("sniffed-jpeg.txt", image::ImageFormat::Jpeg, "image/jpeg"),
        ("sniffed-webp.txt", image::ImageFormat::WebP, "image/webp"),
    ];
    let mut expected_static = Vec::new();
    for (path, format, mime_type) in static_fixtures {
        let bytes = encoded_test_image(format)?;
        std::fs::write(workspace.join(path), &bytes)?;
        expected_static.push((path, mime_type, bytes));
    }
    std::fs::write(workspace.join("image-batch.txt"), "between images\n")?;
    let delays = (1..=12).collect::<Vec<_>>();
    let gif_bytes = gif_fixture((1, 1), (1, 1), &delays)?;
    std::fs::write(workspace.join("duration-sampled.gif"), gif_bytes)?;

    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });
    let client = ().serve((client_read, client_write)).await?;

    for (path, mime_type, expected_bytes) in &expected_static {
        let result = client
            .call_tool(
                CallToolRequestParams::new("file.read")
                    .with_arguments(Map::from_iter([("path".to_string(), json!(path))])),
            )
            .await?;
        let structured = result.structured_content.as_ref().unwrap();
        assert_eq!(structured["image"]["mimeType"], json!(mime_type));
        assert!(structured["image"].get("data").is_none());
        let content = result
            .content
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["mimeType"], json!(mime_type));
        let data = content[1]["data"].as_str().unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data)?
                .as_slice(),
            expected_bytes.as_slice()
        );
        assert!(!structured.to_string().contains(data));
    }

    let batch = client
        .call_tool(
            CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                "requests".to_string(),
                json!([
                    {"path": "sniffed-png.txt"},
                    {"path": "image-batch.txt"},
                    {"path": "missing-image-batch.txt"},
                    {"path": "sniffed-webp.txt"}
                ]),
            )])),
        )
        .await?;
    let structured = batch.structured_content.as_ref().unwrap();
    assert_eq!(structured["results"][0]["index"], 0);
    assert_eq!(structured["results"][1]["index"], 1);
    assert_eq!(structured["results"][2]["index"], 2);
    assert_eq!(structured["results"][3]["index"], 3);
    assert_eq!(
        structured["results"][0]["result"]["image"]["mimeType"],
        "image/png"
    );
    assert_eq!(
        structured["results"][1]["result"]["content"],
        "between images\n"
    );
    assert_eq!(structured["results"][2]["status"], "failed");
    assert_eq!(
        structured["results"][3]["result"]["image"]["mimeType"],
        "image/webp"
    );
    let batch_content = batch
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(
        batch_content
            .iter()
            .map(|block| block["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["text", "image", "text", "text", "text", "image"]
    );
    assert_eq!(
        serde_json::from_str::<Value>(batch_content[0]["text"].as_str().unwrap())?["index"],
        0
    );
    assert_eq!(
        serde_json::from_str::<Value>(batch_content[2]["text"].as_str().unwrap())?["index"],
        1
    );
    assert_eq!(
        serde_json::from_str::<Value>(batch_content[3]["text"].as_str().unwrap())?["index"],
        2
    );
    assert_eq!(
        serde_json::from_str::<Value>(batch_content[4]["text"].as_str().unwrap())?["index"],
        3
    );
    assert!(structured["results"]
        .to_string()
        .find(batch_content[1]["data"].as_str().unwrap())
        .is_none());

    let gif = client
        .call_tool(
            CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                "path".to_string(),
                json!("duration-sampled.gif"),
            )])),
        )
        .await?;
    let gif_value = gif.structured_content.as_ref().unwrap();
    assert_eq!(gif_value["image"]["sourceMimeType"], "image/gif");
    let frames = gif_value["image"]["frames"].as_array().unwrap();
    let timestamps = frames
        .iter()
        .map(|frame| frame["timestampMs"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(timestamps, [0, 100, 210, 280, 360, 550, 660]);
    assert_eq!(gif.content.len(), 1 + frames.len());
    for (block, frame) in gif.content.iter().skip(1).zip(frames) {
        let block = serde_json::to_value(block)?;
        assert_eq!(block["type"], "image");
        assert_eq!(block["mimeType"], frame["mimeType"]);
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(block["data"].as_str().unwrap())?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
        assert_eq!(decoded.width(), frame["width"].as_u64().unwrap() as u32);
        assert_eq!(decoded.height(), frame["height"].as_u64().unwrap() as u32);
    }

    let _ = client.cancel().await;
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn in_process_stdio_file_read_enforces_image_pixel_and_response_bounds() -> anyhow::Result<()>
{
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(
        workspace.join("over-pixel-limit.png"),
        oversized_png_header()?,
    )?;
    std::fs::write(workspace.join("invalid.png"), b"\x89PNG\r\n\x1a\nnot-a-png")?;
    let mut oversized_invalid = b"\x89PNG\r\n\x1a\n".to_vec();
    oversized_invalid.resize(6 * 1024 * 1024 + 3, 0);
    std::fs::write(workspace.join("oversized-invalid.png"), oversized_invalid)?;
    let mut response_limited = encoded_test_image(image::ImageFormat::Png)?;
    let iend_start = response_limited.len() - 12;
    let trailer = response_limited.split_off(iend_start);
    let mut text_chunk = b"Comment\0".to_vec();
    text_chunk.resize(6 * 1024 * 1024 - response_limited.len() - 24, b'x');
    append_png_chunk(&mut response_limited, b"tEXt", &text_chunk);
    response_limited.extend_from_slice(&trailer);
    std::fs::write(workspace.join("response-limit.png"), response_limited)?;
    let cumulative_gif = gif_fixture((4096, 4096), (1, 1), &[1, 1, 1, 1, 1])?;
    std::fs::write(workspace.join("cumulative-limit.gif"), cumulative_gif)?;
    std::fs::write(
        workspace.join("range.png"),
        encoded_test_image(image::ImageFormat::Png)?,
    )?;

    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = split(client_io);
    let (server_read, server_write) = split(server_io);
    let server_task = tokio::spawn(async move {
        let transport = AsyncRwTransport::<RoleServer, _, _>::new_server(server_read, server_write);
        let running = server
            .serve(ResumableStdioTransport::new(transport))
            .await?;
        let _ = running.waiting().await?;
        anyhow::Result::<()>::Ok(())
    });
    let client = ().serve((client_read, client_write)).await?;

    for (arguments, expected_code) in [
        (
            json!({"path":"over-pixel-limit.png"}),
            "file_image_too_large",
        ),
        (json!({"path":"invalid.png"}), "file_invalid_image"),
        (
            json!({"path":"oversized-invalid.png"}),
            "file_image_response_too_large",
        ),
        (
            json!({"path":"response-limit.png"}),
            "file_image_response_too_large",
        ),
        (
            json!({"path":"cumulative-limit.gif"}),
            "file_image_too_large",
        ),
        (
            json!({"path":"range.png","startLine":1,"endLine":1}),
            "file_invalid_line_range",
        ),
    ] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("file.read")
                    .with_arguments(serde_json::from_value(arguments)?),
            )
            .await?;
        assert_eq!(
            result.structured_content.as_ref().unwrap()["error"]["code"],
            expected_code
        );
        assert!(!result.content.iter().any(|block| {
            serde_json::to_value(block)
                .ok()
                .is_some_and(|value| value["type"] == "image")
        }));
    }

    let overflow_text = format!("{}\n", "x".repeat(2_000)).repeat(100);
    let mut requests = vec![json!({"path":"response-limit.png"})];
    for index in 0..4 {
        let path = format!("overflow-text-{index}.txt");
        std::fs::write(workspace.join(&path), &overflow_text)?;
        requests.push(json!({"path":path}));
    }
    let batch = client
        .call_tool(
            CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                "requests".to_string(),
                Value::Array(requests),
            )])),
        )
        .await?;
    let batch_value = batch.structured_content.as_ref().unwrap();
    assert_eq!(batch_value["status"], "completed_with_errors");
    assert_eq!(batch_value["results"].as_array().unwrap().len(), 5);
    assert_eq!(batch_value["results"][0]["index"], 0);
    assert_eq!(batch_value["results"][0]["status"], "failed");
    assert_eq!(
        batch_value["results"][0]["error"]["code"],
        "file_image_response_too_large"
    );
    for index in 0..4 {
        let result = &batch_value["results"][index + 1];
        assert_eq!(result["index"], index + 1);
        assert_eq!(result["status"], "completed");
        assert_eq!(
            result["result"]["content"].as_str(),
            Some(overflow_text.as_str())
        );
    }

    assert!(!batch.content.iter().any(|block| {
        serde_json::to_value(block)
            .ok()
            .is_some_and(|value| value["type"] == "image")
    }));

    std::fs::write(
        workspace.join("tiny-after-limit.png"),
        encoded_test_image(image::ImageFormat::Png)?,
    )?;
    let recovered = client
        .call_tool(
            CallToolRequestParams::new("file.read").with_arguments(Map::from_iter([(
                "requests".to_string(),
                json!([
                    {"path": "response-limit.png"},
                    {"path": "tiny-after-limit.png"}
                ]),
            )])),
        )
        .await?;
    let recovered_value = recovered.structured_content.as_ref().unwrap();
    assert_eq!(recovered_value["status"], "completed_with_errors");
    assert_eq!(recovered_value["results"][0]["status"], "failed");
    assert_eq!(
        recovered_value["results"][0]["error"]["code"],
        "file_image_response_too_large"
    );
    assert_eq!(recovered_value["results"][1]["status"], "completed");
    assert_eq!(
        recovered_value["results"][1]["result"]["image"]["mimeType"],
        "image/png"
    );
    let recovered_images = recovered
        .content
        .iter()
        .filter_map(|block| serde_json::to_value(block).ok())
        .filter(|block| block["type"] == "image")
        .collect::<Vec<_>>();
    assert_eq!(recovered_images.len(), 1);
    assert_eq!(recovered_images[0]["mimeType"], "image/png");
    assert!(serde_json::to_vec(&recovered)?.len() <= crate::file_ops::MAX_IMAGE_RESPONSE_BYTES);

    let _ = client.cancel().await;

    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn file_search_dispatch_supports_literal_and_regex_queries() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("search.rs"), "Alpha\nBeta 42\n")?;
    let clipped = server
        .dispatch(
            "file.search",
            json!({"path":"search.rs", "query":"Beta", "contextLines":8}),
        )
        .await?;
    assert_eq!(clipped["contextLines"], 5);
    assert_eq!(
        clipped["warnings"][0],
        "context_lines_clipped_to_configured_limit"
    );
    assert_eq!(clipped["matches"].as_array().map(Vec::len), Some(1));

    server
        .state
        .config
        .write()
        .await
        .limits
        .max_file_search_context_lines = 20;
    let expanded = server
        .dispatch(
            "file.search",
            json!({"path":"search.rs", "query":"Beta", "contextLines":8}),
        )
        .await?;
    assert_eq!(expanded["matches"].as_array().map(Vec::len), Some(1));
    assert!(expanded.get("contextLines").is_none());
    assert!(expanded.get("warnings").is_none());

    server
        .state
        .config
        .write()
        .await
        .limits
        .max_file_search_context_lines = 0;
    let disabled_context = server
        .dispatch(
            "file.search",
            json!({"path":"search.rs", "query":"Beta", "contextLines":5}),
        )
        .await?;
    assert_eq!(disabled_context["contextLines"], 0);
    assert!(disabled_context["warnings"].is_array());

    let literal = server
        .dispatch(
            "file.search",
            json!({"path":".", "query":"alpha", "caseSensitive":false, "include":["**/*.rs"]}),
        )
        .await?;
    assert_eq!(literal["matches"].as_array().map(Vec::len), Some(1));
    let regex = server
        .dispatch(
            "file.search",
            json!({"path":"search.rs", "query":"Beta \\d+", "mode":"regex"}),
        )
        .await?;
    assert_eq!(regex["matches"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn file_read_and_search_batches_preserve_order_and_isolate_failures() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("batch-a.txt"), "needle-a\n")?;
    std::fs::write(workspace.join("batch-b.txt"), "needle-b\n")?;

    let reads = server
        .dispatch(
            "file.read",
            json!({"requests":[
                {"path":"batch-a.txt"},
                {"path":"missing.txt"},
                {"path":"batch-b.txt","metadata":true}
            ]}),
        )
        .await?;
    assert_eq!(reads["results"][0]["index"], 0);
    assert_eq!(reads["results"][0]["result"]["content"], "needle-a\n");
    assert_eq!(reads["results"][1]["status"], "failed");
    assert_eq!(reads["results"][2]["index"], 2);
    assert_eq!(reads["results"][2]["result"]["content"], "needle-b\n");
    assert!(reads["results"][2]["result"]["metadata"].is_object());

    let searches = server
        .dispatch(
            "file.search",
            json!({"requests":[
                {"path":"batch-a.txt","query":"needle-a"},
                {"path":"missing.txt","query":"needle"},
                {"path":"batch-b.txt","query":"needle-b"}
            ]}),
        )
        .await?;
    assert_eq!(
        searches["results"][0]["result"]["matches"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(searches["results"][1]["status"], "failed");
    assert_eq!(
        searches["results"][2]["result"]["matches"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );

    assert!(server
        .dispatch(
            "file.read",
            json!({"path":"batch-a.txt","requests":[{"path":"batch-b.txt"}]}),
        )
        .await
        .is_err());
    assert!(server
        .dispatch(
            "file.search",
            json!({"path":"batch-a.txt","query":"x","requests":[{"path":"batch-b.txt","query":"needle"}]})
        )
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn file_edit_add_creates_nested_parents_after_whole_patch_preflight() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let result = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: nested/deep/created.txt\n+created\n*** Add File: ghost/../normalized/deep/created.txt\n+normalized\n*** End Patch"}),
        )
        .await?;
    assert_eq!(result["status"], "completed");
    assert_eq!(
        std::fs::read_to_string(workspace.join("nested/deep/created.txt"))?,
        "created\n"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("normalized/deep/created.txt"))?,
        "normalized\n"
    );
    assert!(!workspace.join("ghost").exists());

    let rejected = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: preflight/nested/new.txt\n+new\n*** Update File: missing-source.txt\n@@\n-old\n+new\n*** End Patch"}),
        )
        .await?;
    assert_eq!(rejected["error"]["code"], "file_not_found");
    assert!(!workspace.join("preflight").exists());

    std::fs::write(workspace.join("move-source.txt"), "move\n")?;
    let move_rejected = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Update File: move-source.txt\n*** Move to: missing-move-parent/moved.txt\n@@\n-move\n+moved\n*** End Patch"}),
        )
        .await?;
    assert_eq!(move_rejected["error"]["code"], "file_parent_not_found");
    assert!(!workspace.join("missing-move-parent").exists());

    server
        .state
        .config
        .write()
        .await
        .confirmation_provider
        .channels
        .clear();
    let confirmation_rejected = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: confirmation/nested/new.txt\n+new\n*** End Patch","needConfirm":true}),
        )
        .await?;
    assert_eq!(
        confirmation_rejected["error"]["code"],
        "file_confirmation_unavailable"
    );
    assert!(!workspace.join("confirmation").exists());

    let external_path = workspace.join("external/nested/target.txt");
    crate::file_ops::inject_external_change(&external_path, b"external\n");
    let raced = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: external/nested/target.txt\n+agent\n*** End Patch"}),
        )
        .await?;
    assert_eq!(raced["error"]["code"], "file_already_exists");
    assert_eq!(std::fs::read_to_string(&external_path)?, "external\n");

    let failed_path = workspace.join("commit-failure/nested/target.txt");
    crate::file_ops::inject_commit_failure(&failed_path);
    let commit_failed = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: commit-failure/nested/target.txt\n+agent\n*** End Patch"}),
        )
        .await?;
    assert_eq!(commit_failed["error"]["code"], "file_write_failed");
    assert!(!workspace.join("commit-failure").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn file_edit_add_reserves_audit_path_under_symlinked_workspace() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let alias = workspace.parent().unwrap().join("workspace-alias");
    std::os::unix::fs::symlink(&workspace, &alias)?;
    let mut config = server.state.config.read().await.clone();
    config.workspace_root = alias.clone();
    config.path_policy.write_roots = vec![alias];
    *server.state.config.write().await = config;

    let audit_path = workspace.join(".agentic-gpt-audit.jsonl");
    assert!(!audit_path.exists());
    let denied = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: .agentic-gpt-audit.jsonl/keep.txt\n+keep\n*** End Patch"}),
        )
        .await?;
    assert_eq!(denied["error"]["code"], "file_reserved_path");
    assert!(audit_path.is_file());
    assert!(!audit_path.is_dir());
    assert!(!audit_path.join("keep.txt").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn file_edit_add_keeps_path_policy_for_missing_parents_and_symlinks() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let real = workspace.join("real");
    let alias = workspace.join("alias");
    std::fs::create_dir(&real)?;
    std::os::unix::fs::symlink(&real, &alias)?;
    server.state.config.write().await.path_policy.deny_roots = vec![alias.join("blocked/nested")];

    let denied = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: real/blocked/nested/new.txt\n+new\n*** End Patch"}),
        )
        .await?;
    assert_eq!(denied["error"]["code"], "path_denied");
    assert!(!real.join("blocked").exists());

    let outside = workspace.parent().unwrap().join("outside");
    std::fs::create_dir(&outside)?;
    std::os::unix::fs::symlink(&outside, workspace.join("escape"))?;
    let escaped = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: escape/new/nested.txt\n+new\n*** End Patch"}),
        )
        .await?;
    assert_eq!(escaped["error"]["code"], "path_denied");
    assert!(!outside.join("new").exists());
    Ok(())
}

#[tokio::test]
async fn file_edit_apply_patch_supports_multi_file_changes_and_slim_response() -> anyhow::Result<()>
{
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("update.txt"), "old\n")?;
    std::fs::write(workspace.join("delete.txt"), "gone\n")?;
    std::fs::write(workspace.join("move.txt"), "move\n")?;
    let patch = "*** Begin Patch\n*** Add File: add.txt\n+added\n*** Update File: update.txt\n@@\n-old\n+new\n*** Delete File: delete.txt\n*** Update File: move.txt\n*** Move to: moved.txt\n@@\n-move\n+moved\n*** End Patch";
    let result = server.dispatch("file.edit", json!({"patch":patch})).await?;
    assert_eq!(result["status"], "completed");
    assert_eq!(result["changed"], 4);
    assert_eq!(result["changes"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        std::fs::read_to_string(workspace.join("add.txt"))?,
        "added\n"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("update.txt"))?,
        "new\n"
    );
    assert!(!workspace.join("delete.txt").exists());
    assert_eq!(
        std::fs::read_to_string(workspace.join("moved.txt"))?,
        "moved\n"
    );
    assert!(!workspace.join("move.txt").exists());
    assert_eq!(
        result["changes"][0],
        json!({"path":"add.txt","action":"created"})
    );
    assert_eq!(
        result["changes"][1],
        json!({"path":"update.txt","action":"updated"})
    );
    assert_eq!(
        result["changes"][2],
        json!({"path":"delete.txt","action":"deleted"})
    );
    assert_eq!(
        result["changes"][3],
        json!({"path":"move.txt","action":"moved","destination":"moved.txt"})
    );
    assert!(result.get("summary").is_none());
    Ok(())
}

#[tokio::test]
async fn file_edit_later_commit_failure_reports_prior_and_skipped_changes() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("unchanged.txt"), "same\n")?;
    crate::file_ops::inject_commit_failure(&workspace.join("failed.txt"));
    crate::file_ops::inject_commit_failure(&workspace.join("second_failed.txt"));
    let result = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch
*** Update File: unchanged.txt
@@
-same
+same
*** Add File: committed.txt
+committed
*** Add File: failed.txt
+failed
*** Add File: skipped.txt
+skipped
*** End Patch"}),
        )
        .await?;
    assert_eq!(result["status"], "completed_with_errors");
    assert_eq!(result["changes"][0]["status"], "unchanged");
    assert_eq!(result["changes"][1]["status"], "created");
    assert_eq!(result["changes"][2]["status"], "failed");
    assert_eq!(result["changes"][2]["error"]["code"], "file_write_failed");
    assert_eq!(result["changes"][3]["status"], "skipped-not-attempted");
    assert_eq!(result["summary"]["committed"], 1);
    assert_eq!(result["summary"]["failed"], 1);
    assert_eq!(result["summary"]["skipped"], 1);
    assert!(workspace.join("committed.txt").is_file());
    assert!(!workspace.join("failed.txt").exists());
    assert!(!workspace.join("skipped.txt").exists());

    let audit_lines = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["tool"] == "file.edit")
        .collect::<Vec<_>>();
    assert_eq!(audit_lines.len(), 4);
    assert_eq!(audit_lines[0]["outcome"], "unchanged");
    assert_eq!(audit_lines[0]["committed"], false);
    assert_eq!(audit_lines[1]["outcome"], "created");
    assert_eq!(audit_lines[2]["outcome"], "failed");
    assert_eq!(audit_lines[2]["errorCode"], "file_write_failed");
    assert_eq!(audit_lines[3]["outcome"], "skipped-not-attempted");

    let second_failure = server
        .dispatch(
            "file.edit",
            json!({"patch":"*** Begin Patch\n*** Add File: second_failed.txt\n+second\n*** End Patch"}),
        )
        .await?;
    assert_eq!(second_failure["error"]["code"], "file_write_failed");
    assert!(!workspace.join("second_failed.txt").exists());
    Ok(())
}

#[tokio::test]
async fn file_edit_apply_patch_context_mismatch_and_confirmation_do_not_write() -> anyhow::Result<()>
{
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    std::fs::write(workspace.join("good.txt"), "good\n")?;
    std::fs::write(workspace.join("bad.txt"), "actual\n")?;
    let mismatch = server
            .dispatch("file.edit", json!({"patch":"*** Begin Patch\n*** Update File: good.txt\n@@\n-good\n+changed\n*** Update File: bad.txt\n@@\n-wrong\n+never\n*** End Patch"}))
            .await?;
    assert_eq!(mismatch["error"]["code"], "file_patch_conflict");
    assert_eq!(
        std::fs::read_to_string(workspace.join("good.txt"))?,
        "good\n"
    );

    server
        .state
        .config
        .write()
        .await
        .confirmation_provider
        .channels
        .clear();
    let denied = server
            .dispatch("file.edit", json!({"patch":"*** Begin Patch\n*** Update File: good.txt\n@@\n-good\n+confirmed\n*** End Patch","needConfirm":true}))
            .await?;
    assert_eq!(denied["error"]["code"], "file_confirmation_unavailable");
    assert_eq!(
        std::fs::read_to_string(workspace.join("good.txt"))?,
        "good\n"
    );
    Ok(())
}

#[tokio::test]
async fn file_edit_apply_patch_revalidates_external_change_before_commit() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    let path = workspace.join("race.txt");
    std::fs::write(&path, "before\n")?;
    crate::file_ops::inject_external_change(&path, b"external\n");
    let result = server
            .dispatch(
                "file.edit",
                json!({"patch":"*** Begin Patch\n*** Update File: race.txt\n@@\n-before\n+agent\n*** End Patch"}),
            )
            .await?;
    assert_eq!(result["error"]["code"], "file_revision_conflict");
    assert_eq!(std::fs::read_to_string(&path)?, "external\n");
    Ok(())
}

#[tokio::test]
async fn file_lock_registry_prunes_released_paths() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let workspace = server.state.config.read().await.workspace_root.clone();
    {
        let _guard = crate::file_ops::lock_target(&server.state, &workspace.join("one.txt")).await;
    }
    {
        let _guard = crate::file_ops::lock_target(&server.state, &workspace.join("two.txt")).await;
    }
    assert_eq!(server.state.file_locks.lock().await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn compact_mcp_skills_and_tmux_adapters_preserve_result_envelopes() -> anyhow::Result<()> {
    let server = AgentMcpServer::new(test_state(CapabilityProfile::Normal));
    let mcp = server.dispatch("mcp.list", json!({})).await?;
    assert!(mcp["servers"].is_array());
    let skills = server.dispatch("skills.list", json!({})).await?;
    assert!(skills["skills"].is_array());
    assert!(skills.get("activeSkills").is_none());
    assert!(skills.get("warnings").is_none());
    let panes = server
        .dispatch("tmux.panes", json!({"action": "list"}))
        .await?;
    assert!(panes.get("panes").is_some() || panes.get("error").is_some());
    let sessions = server
        .dispatch("tmux.sessions", json!({"action": "list"}))
        .await?;
    assert!(sessions.get("sessions").is_some() || sessions.get("error").is_some());
    Ok(())
}

fn test_state(profile: CapabilityProfile) -> AppState {
    let root = std::env::temp_dir().join(format!("agentic-stdio-{}", Uuid::new_v4()));
    let workspace_root = root.join("workspace");
    let mut config = Config::default_config().expect("default config");
    config.agent_id = "stdio-test-agent".to_string();
    config.toolsets = if profile == CapabilityProfile::Room {
        ToolsetConfig::room()
    } else {
        ToolsetConfig::normal()
    };
    config.workspace_root = workspace_root.clone();
    config.path_policy.write_roots = vec![workspace_root.clone()];
    config.ensure_workspace().expect("workspace");
    let bootstrap_root = workspace_root.join("bootstrap");
    std::fs::create_dir_all(&bootstrap_root).expect("bootstrap directory");
    std::fs::write(
            bootstrap_root.join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room Bootstrap\ndescription: Test bootstrap\nschemaVersion: 1\n---\n",
        )
        .expect("bootstrap entrypoint");
    let private_state =
        crate::private_state::PrivateStatePaths::for_test(root.join("private-state"));
    let process_history = crate::process_history::ProcessHistoryStore::open(&private_state);
    AppState {
        config_path: PathBuf::from("stdio-test-config.json"),
        config: Arc::new(RwLock::new(config)),
        private_state,
        process_history,
        browser_runtime: None,
        runtime: RuntimeModel::tunnel(profile, false),
        started_at: chrono::Utc::now(),
        boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        supervised: true,
        file_locks: Arc::new(Mutex::new(HashMap::new())),
        processes: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        reporting_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
        room_repository_writes: Arc::new(Mutex::new(())),
        skills_writes: Arc::new(Mutex::new(())),
        skill_leases: Arc::new(SkillLeaseManager::new()),
        skill_installs: Arc::new(InstallManager::new()),
    }
}

async fn allow_test_printf(server: &AgentMcpServer) {
    server
        .state
        .config
        .write()
        .await
        .policy
        .allow
        .push(crate::config::Rule {
            program: "printf".to_string(),
            args_prefix: Vec::new(),
        });
}

fn state_with_browser_runtime(docs_root: PathBuf, result: CallToolResult) -> AppState {
    let mut state = test_state(CapabilityProfile::Normal);
    state.browser_runtime = Some(Arc::new(crate::state::BrowserRuntimeContext {
        descriptor: crate::browser_runtime::BrowserRuntimeDescriptor {
            app_version: "test-browser".to_string(),
            channel: "test".to_string(),
            node_repl_path: PathBuf::from("/runtime/node-repl"),
            node_path: PathBuf::from("/runtime/node"),
            browser_client_path: PathBuf::from("/runtime/browser-client.mjs"),
            browser_service_path: PathBuf::from("/runtime/browser-service.mjs"),
            codex_home: PathBuf::from("/runtime/codex"),
            codex_cli_path: Some(PathBuf::from("/runtime/codex-cli")),
            node_module_dirs: Vec::new(),
            trusted_code_paths: Vec::new(),
            docs_root,
        },
        manager: crate::browser_manager::test_manager_with_result(result),
    }));
    state
}
