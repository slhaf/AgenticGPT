## 精确源码补充

### 1. Response 的 `store_result` 返回值被忽略

`crates/agentic-gpt-hub/src/agents.rs:329-340`：

```rust
AgentMessage::Response {
    run_id,
    request_id,
    data,
} => {
    if let Err(error) =
        runs::store_result(state, agent_id, run_id.as_deref(), &request_id, &data)
    {
        warn!(%agent_id, %request_id, %error, "failed to store agent result");
    }
    if let Some(sender) = state.pending.lock().await.remove(&request_id) {
        let _ = sender.send(data);
    }
}
```

这里处理的是 `Result<(), error>` 之外的匹配结果：`runs::store_result` 返回的 bool（是否找到/修改匹配 run）没有被接收或判断；只有真正的 `Err` 会日志告警。无论 run/request/agent 匹配是否成功，后续都会执行 `pending.remove(&request_id)`，并把入站 `data` 发送给等待者。此前报告中“pending waiter 未完整绑定所有权”的问题可用这段作为最小证据。

### 2. Agent 侧 legacy Room 拒绝函数及匹配

`crates/agentic-gpt/src/local_service.rs:132-141`：

```rust
HubCommand::RoomNotebookAppend { .. }
| HubCommand::RoomNotebookRecent { .. }
| HubCommand::RoomNotebookSelectExact { .. }
| HubCommand::RoomNotebookSearch { .. }
| HubCommand::RoomNotebookCurrent { .. }
| HubCommand::RoomNotebookUpdate { .. }
| HubCommand::RoomNotebookRemove { .. }
| HubCommand::RoomDiaryAppend { .. }
| HubCommand::RoomDiaryRecent { .. }
| HubCommand::RoomDiarySelectExact { .. } => Ok(legacy_room_surface_removed_error()),
```

`crates/agentic-gpt/src/local_service.rs:275-281`：

```rust
fn legacy_room_surface_removed_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_legacy_surface_removed",
            "message": "legacy Room JSONL commands are reserved for Hub parity and are not available on the Agent"
        }
    })
}
```

因此不是“命令类型不存在”或“路由不可达”，而是 Agent 当前 dispatch 对这些命令有明确、统一的拒绝结果。

### 3. 旧 Room 工具确实在 Full profile 的 `tools/list` 中暴露

证据链如下：

1. `crates/agentic-gpt-hub/src/mcp_server.rs:57-63` 的 `AgenticMcpServer::new` 调用 `Self::tool_router()`，该 router 包含 `#[tool]` 方法；`mcp_server.rs:69-72` 的 profile gate 是：

```rust
fn allows_tool(&self, name: &str) -> bool {
    self.profile == McpProfile::Full || COORDINATOR_TOOLS.contains(&name)
}
```

Full 对任意注册工具名返回 true。

2. `mcp_server.rs:535-540` 的 `app_tool_descriptors` 对 `server.tool_router.list_all()` 逐项执行 `server.allows_tool(...)`；Full 下旧 Room 工具全部通过过滤。

3. 旧 Room 方法使用 `#[tool(name = ...)]` 明确注册，例如：
   - `mcp_server.rs:1239-1243`：`room.notebook.append`
   - `mcp_server.rs:1284-1288`：`room.notebook.recent`
   - `mcp_server.rs:1319-1323`：`room.notebook.selectExact`
   - `mcp_server.rs:1349-1353`：`room.notebook.search`
   - `mcp_server.rs:1379-1383`：`room.notebook.current`
   - `mcp_server.rs:1407-1411`：`room.notebook.update`
   - `mcp_server.rs:1443-1447`：`room.notebook.remove`
   - `mcp_server.rs:1469-1473`：`room.diary.append`
   - `mcp_server.rs:1498-1502`：`room.diary.recent`
   - `mcp_server.rs:1527-1531`：`room.diary.selectExact`

4. `mcp_server.rs:198-204` 的 GET MCP metadata 返回 `"tools": app_tool_descriptors(&server)`；`mcp_server.rs:231-233` 的 JSON-RPC `tools/list` 同样返回 `app_tool_descriptors(&server)`。所以 Full profile 的客户端可以发现这些旧工具。

5. 调用 dispatch 也可达：`mcp_server.rs:339-386` 按这些名称选择对应方法；例如 append 方法在 `mcp_server.rs:1269-1278` 构造 `HubCommand::RoomNotebookAppend`，diary append 在 `mcp_server.rs:1483-1492` 构造 `HubCommand::RoomDiaryAppend`。这些命令经过 active Room 转发后，在 Agent `local_service.rs:132-141` 命中拒绝函数。

结论：此前 P1 的表述应精确为“Full profile 暴露并可调用的 Hub 旧 Room 工具，在当前 Agent 跨进程 dispatch 上统一返回 `room_legacy_surface_removed`”；不是把一个不可达的 legacy 分支误判成 API 断裂。Coordinator profile 例外：`COORDINATOR_TOOLS` 仅含 Hub info/list、run/job 查询和 notify，旧 Room 工具不会在该 profile 的 descriptor 中出现；`mcp_server.rs:2827-2860` 的 `coordinator_profile_exposes_only_native_tools` 测试也固定了这一点。