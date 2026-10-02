# 运维指南

本文记录可复现部署所需的最低限度检查。Standalone 是主要部署路径；Hub 是可选组件，因此 Hub 检查单独列出。

## 仓库验证

按 `.github/workflows/ci.yml` 指定的顺序运行 Rust CI 检查：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m venv target/contract-venv
source target/contract-venv/bin/activate
python -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
python3 scripts/check_contract_parity.py
```

契约一致性检查器是跨接口 schema/实时行为门禁。不带选项时，它使用
`target/debug/agentic-gpt` 和 `target/debug/agentic-gpt-hub`；
`--agent-bin PATH` 和 `--hub-bin PATH` 用于指定二进制文件。它会启动隔离的
loopback/私有主目录进程并在结束时清理；schema 验证与实时行为是两项独立要求。
该检查序列用于验证包含 Agent 事件工具在内的契约；本文不表示 Hub parity 门禁已通过。
通过一致性门禁不能替代前面的严格 Clippy 检查。

Standalone HTTP MCP 集成测试夹具使用较短且 UUID 唯一的 Agent ID：其本地 Unix MCP
套接字位于 `HOME/.agentic_gpt` 下，运行时拒绝长度超过 100 字节的套接字路径。
修改测试夹具时，应按 CI 运行器的主目录检查完整套接字路径。

## 本地/Standalone 冒烟检查（主要路径）

Tunnel 凭据不可用时设置 `mode=local`；要验证完整的推荐路径则使用
`mode=standalone`。两种模式都从 `agentic-gpt run` 启动。

```bash
agentic-gpt --version
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt run
```

在另一个 shell 中运行：

```bash
agentic-gpt local list-tools
agentic-gpt local call agent.info --arguments '{}'
```

预期结果：

- `agent.info.connections.localMcp.status` 为 `ready`。
- 运行时目录权限为 `0700`、套接字权限为 `0600`，且只接受相同 UID。
- 存在 `event.list`、`event.get`、`event.mark`，以及 `process.exec`、`process.batch`、`process.status`、`process.list`、`process.output`、`process.result`、`process.cancel`、`mcp.callTool` 和 `mcp.batch`。
- 事件收件箱工具独立于工具集开关；隐藏事件不因此视为已处理。它们管理收件箱状态，不是进程控制接口。
- Process 历史使用 `process.sqlite3`；事件历史使用独立的 `events.sqlite3`。现有 `jobs.sqlite3` 会有意保持原样且不可访问，不迁移或备份 Job 历史。
- [持久事件收件箱](interfaces.md#持久事件收件箱)说明同一 Agent 的共享收件箱和提醒范围。
- 已移除的 v0.8 受管理生命周期名称不存在。

Standalone 还应确认：

- tunnel-client `doctor` 和 loopback 就绪检查通过；
- ChatGPT connector 可以调用 `agent.info`；
- 一台机器断开连接不会影响其他机器的 tunnel；
- 重启后恢复的首次调用不会产生 `expect initialized request`，也不会重启 worker 进程对；
- 仅在需要恢复时才会出现 `mcp_stdio_session_resume` / `mcp_stdio_session_resumed`。

## Hub 冒烟检查（可选的集中式模式）

示例使用 loopback Hub 的固定测试值 `test-key`；这是公开且不含保密信息的本地测试值，只适用于此本地示例。它会出现在服务环境和 `curl` argv 中，绝不可将真实部署密钥替换到这里；真实密钥不要作为 argv 传递。

```bash
tmp=$(mktemp -d)
cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" init
AGENTIC_GPT_API_KEY=test-key \
  cargo run -q -p agentic-gpt-hub -- --db "$tmp/hub.sqlite3" --config "$tmp/hub.json" serve --bind 127.0.0.1:18787
curl -fsS -H 'Authorization: Bearer test-key' http://127.0.0.1:18787/v1/info
```

预期 JSON 包含 `service`、`version`、`remoteConfirmation`、`agents`、`counts` 和 `generatedAt`。

### WP1 收尾证据边界

当前 WP1 收尾证据范围有限：最终清理阶段通过了 `cargo fmt --all -- --check`、集成的 `cargo test -p agentic-gpt-hub` 套件，以及无警告的 `cargo build -p agentic-gpt-hub`。一次隔离的 Hub HTTP/SSE 冒烟检查使用模拟 Agent 对等端，覆盖了六种确认、替换、回调、超时和迟到回执场景。此后，针对同一临时 SQLite 数据库执行的真实 Hub 重启保留了两条状态为 `completed` 的 `/v1/runs` 记录和 `sessions[]`；`/v1/info` 则报告 `pendingRequestCount`、`pendingConfirmationCount` 和 `cachedJobCount` 均为零。这只证明 Hub 侧的回执/会话清理和保留行为；没有执行真实 Agent 或传输账本重启，也不能证明历史路线图中的所有场景都完成了端到端验证。外部 ntfy 提供方行为仍未验证；本地/mock 回调路径不能作为等价证据。

## Standalone 部署检查

1. 确认 `agentic-gpt --version` 是预期版本。
2. 确认 tunnel secret 使用受保护的 `file:` 或 `env:` 引用。
3. 确认 `agentic-gpt run` 以 `mode=standalone` 启动后达到就绪状态，并在重启预算重置间隔之后仍保持稳定。
4. 分别通过 ChatGPT tunnel 和 Local Unix MCP 调用 `agent.info`。
5. 启动一个无害进程，并使用 `process.status`、`process.output` 和 `process.result` 检查；只有明确要求取消时才调用 `process.cancel`。
6. 重启一个 Agent，并确认其他机器的 connector 仍可用。
7. 确认审计 JSONL 位于 `workspaceRoot` 下，且不含原始 tunnel/MCP secret。

## 配置重载与重启诊断

实时更新/重启边界适用于 Standalone、Local 和 Hub 连接模式下的 Agent worker，
并非 Standalone 独有。Agent worker 运行时，对 `policy`、`limits`、`mcpServers`、
`toolsets.enabled` 和 `events` 的有效变更会以原子方式应用于后续准入、调用和工具发现。
`events` 配置变更影响后续 process/skill-install 准入；已准入的 process 和安装任务保留准入时的配置快照。
若 `workspaceRoot` 未变更，`pathPolicy` 也会重载。候选配置若变更
`workspaceRoot`，则旧 `workspaceRoot` 和 `pathPolicy` 会作为一个原子配置对继续
生效，直到重启；不得将部分生效的候选配置视为活动配置。

对于 Standalone HTTP MCP 监听器，应通过受控变更验证协调器处理
`httpMcp.enabled`、`host`、`port`、`publicUrl`、`allowHosts` 以及 bearer token
引用或其解析值。启用/禁用和端点变更会在不重启 worker 的情况下协调生效。
监听器身份发生变化时，会关闭有状态 HTTP session 并丢弃监听器本地 OAuth 状态；
token 变更则会就地更新直接身份验证，并在吊销 OAuth 记录的同时保留现有 session。
凭据无法解析时会 fail closed；候选配置无效时保留上一个有效监听器；端口绑定冲突
会重试，不影响 tunnel 或 Unix 执行。

`mode`、`profile`、`agentId`、`workspaceRoot`、`browser`、Room 设置、
tunnel/客户端设置、上报模式和 Skill 安装并发度等由启动时拥有的字段
发生变化时，需要重启。共享 watcher 会记录日志
`config changes require restart; fields=...`，包含已变更字段名，但绝不包含密钥值。
Standalone supervisor 还会发出 `restart_required`；Hub 模式没有 supervisor 事件。
编辑文件不会切换现有子进程树。实时启用 Room 命名空间时使用当前活动 Room 根目录；
需要重启的 Room 配置变更在重启前不会移动该根目录。

`agentic-gpt local` 命令是仅所有者可用的 Unix MCP 客户端，并保留 `local:` 审计来源。
它不同于 `agentic-gpt tmux` 本地管理员 CLI；后者只暴露四个命令：`list`、
`attach`、`create` 和 `close`（`RequestContext` 中的操作名称为 `tmux.listSessions`、
`tmux.attach`、`tmux.createSession` 和 `tmux.closeSession`）。这些 CLI 调用使用
`localadmin:` 来源，不增加远程审批语义，也不伪造 AppState。其他接入来源仍为
`tunnel:`、`http:` 和 `hub:`。

## Hub 部署检查

1. 记录 `agentic-gpt --version`、`agentic-gpt-hub --version` 输出、工件路径和校验和，
   以确认 Hub 与 Agent 二进制是预期配对工件。不得用版本号或服务名称代替配对检查。
2. 确认 `/v1/info` 可通过公共 HTTPS 响应。
3. 确认 `/v1/agents` 显示预期的、具备命令能力且在线的 Agent。
4. 通过 `/v1/process/exec` 运行一条无害命令。
5. 通过 `GET /v1/process` 和 `GET /v1/process/{processId}` 检查进程元数据；从
   `GET /v1/process/{processId}/output` 获取输出，从
   `GET /v1/process/{processId}/result` 获取结果，并通过
   `POST /v1/process/{processId}/cancel` 请求取消。
   状态端点仅返回元数据。Standalone `process.status` 等待默认 5 秒、上限为 30 秒；
   HTTP 的 `waitSeconds` 默认 5 秒、上限为 30 秒。未提供 `cursor` 时，输出从字节 0 开始；
   `maxBytes` 默认 8 KiB、上限为 32 KiB。MCP 结果 `maxBytes` 默认 8 KiB、上限
   为 512 KiB；HTTP 结果读取使用相同范围。
6. 验证 `/mcp`；契约发生变化时刷新 GPT Actions schema。
7. 如果启用了 Standalone 上报，确认仅上报的连接会拒绝 Hub 执行请求。

### WP-R 当前 Room 契约切换

采用当前九项 Room 契约时，使用以下清洁切换流程。它改变 Hub/Protocol/Agent 的请求和响应投影；不会将 Room 内容迁移到 Hub，也不会添加兼容别名。

1. **盘点配对工件。** 记录 `agentic-gpt --version`、`agentic-gpt-hub --version`、准确的二进制路径和校验和。将 Hub 与 Agent 工件作为一对已验证的版本配对。使用部署当前采用的同一进程调用方式重启（Agent 使用 `agentic-gpt run`，Hub 使用 `agentic-gpt-hub ... serve`）；本流程不会臆造 system service 命令。
2. **记录有效配置和所有者。** 将 `AGENT_CONFIG` 设为实际 Agent 配置路径（默认是 `~/.agentic_gpt/config.json`），并检查 `agentic-gpt config show --config "$AGENT_CONFIG"` 中的 `workspaceRoot`、`room.repositoryRoot`、`mode` 和 `profile`。Room 仓库路径为 `room.repositoryRoot` 或 `<workspaceRoot>/room`。记录实际 Hub `--db` 和 `--config` 路径（默认值为 `~/.agentic_gpt/hub.sqlite3` 和 `~/.agentic_gpt/hub.json`；部署可设置 `AGENTIC_GPT_HUB_DB` 和 `AGENTIC_GPT_HUB_CONFIG`）。
3. **暂停两侧工作。** 暂停新的 Hub HTTP/MCP 调用，排空进行中的调用，并记录尚未排空调用的 `runId` 及对应的 Agent 传输账本条目。使用部署现有流程停止 Hub 和受影响的 Agent。等待者超时不等于取消；不得只因 HTTP 等待结束就重放命令。
4. **替换前备份。** 保留 Agent 配置及其文件权限、完整的已配置 Room 仓库（包括 `.git`），以及部署所需的 Agent 审计/传输文件。将所有 refs 打包为 Git bundle，并同时复制 Room 根目录的文件系统副本，有助于验证。备份 Hub 配置和 SQLite 数据库，并一并保留匹配的 `-wal`、`-shm` 文件（若存在）。备份应设为只读并标注对应工件配对；绝不能用旧副本覆盖较新的数据库或 Room 仓库。

具体备份时，将 `BACKUP_DIR` 设为受保护的目标目录，并使用已记录的路径，而不是猜测服务目录布局：

```bash
mkdir -p "$BACKUP_DIR"
cp -a "$AGENT_CONFIG" "$BACKUP_DIR/agent-config.json"
cp -a "$ROOM_ROOT" "$BACKUP_DIR/room"
git -C "$ROOM_ROOT" bundle create "$BACKUP_DIR/room.git.bundle" --all
cp -a "$HUB_CONFIG" "$BACKUP_DIR/hub.json"
cp -a "$HUB_DB" "$BACKUP_DIR/hub.sqlite3"
test ! -e "$HUB_DB-wal" || cp -a "$HUB_DB-wal" "$BACKUP_DIR/hub.sqlite3-wal"
test ! -e "$HUB_DB-shm" || cp -a "$HUB_DB-shm" "$BACKUP_DIR/hub.sqlite3-shm"
```

如果存在，也应复制 `<workspaceRoot>/.agentic-gpt-audit.jsonl`，以及绑定所有者的 `~/.agentic_gpt/transport-runs.jsonl` 和其 `.lock`、`.recovery`、`.backup` 证据。如果 `HOME` 或 Agent home 被重定位，应使用部署中的实际 `agentic_home` 路径；绝不要从 Hub 数据库推断传输状态。
5. **部署并重新连接。** 将 Hub 和 Agent 二进制作为一对替换，保留现有配置路径，并沿用现有参数和密钥引用重启。首先重新连接具备命令能力的 Room Agent；Normal、ReportingOnly、过期或未就绪的连接不得成为 Room 目标。不要添加版本字段、feature flag、别名或双重执行路径。
6. **验证当前契约。** 确认 `GET /v1/info` 和 `/v1/agents`；随后检查 Full MCP `tools/list` 中存在当前九个 Room 名称（且没有已退役名称），Coordinator `tools/list` 中不存在这些名称。通过已验证身份的 HTTP API，使用当前 camelCase DTO 请求体且不带 `agentId`，分别调用 `/v1/room/diary/active`、`/v1/room/diary/read`、`/v1/room/notebook/recent`、`/v1/room/notebook/search`、`/v1/room/notebook/read`、`/v1/room/state/list`、`/v1/room/state/read`、`/v1/room/maintenance/status` 和 `/v1/room/maintenance/submit`。检查缺少必需 JSON 字段时提取器返回 422 `text/plain`、语义验证错误返回 400 JSON、没有活动 Room 时返回 404、租约冲突返回 409、传输等待超时返回 504。确认成功读取的内容由 Agent 所有且有界；Hub 回执只是有界结果投影。
7. **验证维护所有权。** 仅在可丢弃或明确获准的 Room 仓库上，通过 `room.maintenance.submit` 以 `local` 模式提交一项已文档化的语义 slot/payload，并确认返回的状态、revision、请求残留清理和 Git 变更都限于声明目标。可丢弃的本地冒烟检查可以使用现有 Notebook payload 形状，例如：
   `{"items":[{"slot":"notebook","payload":{"path":"Notebook/wp-r-smoke.md","title":"WP-R 冒烟","body":"有界冒烟测试"}}],"mode":"local","waitSeconds":0}`;
   仓库和目标必须明确可丢弃或已获准。workflow 模式下，应核验 `submitted`/`pending` 与已观察到的同步结果，并确认 `waitSeconds` 0–30 仅限定等待时间，绝不会取消请求。维护测试不得使用已退役的 append/update/remove 形状。
8. **迁移调用方并完成切换。** 将导入的 OpenAPI 使用方和内部调用方更新为当前九项操作。`recent` 和 `search` 保留原名称，但消费当前 Markdown 预览/结果；旧 passage/JSONL 格式以及旧 append/update/remove/日期选择语义都不会自动转换。只有当前路由、MCP 描述符、Agent 派发和仓库所有者检查均通过后，才能移除旧调用方。

**回滚边界。** 如果验证失败，暂停新调用、停止新版本配对工件、保留所有当前 Hub/Agent/Room 证据，只恢复先前已验证的二进制（必要时保留未变更的配置）。保留最新 Hub 数据库及其 `-wal`/`-shm`、Room `.git`、维护日志、审计和传输账本；不要用旧数据库或 Room 副本覆盖较新结果。重新连接旧版本配对工件，并复核 `/v1/info`、`/v1/agents`、活动 Room 租约安全以及已记录的 `runId`。回滚二进制无法撤回维护提交或已经产生的外部效果；任何内容更正都必须使用现有 Agent/Git 和受控维护权限。没有需要撤销的破坏性数据迁移。

### Hub Response 所有权切换

将 Hub 迁移到包含 Response 所有权修复的构建时，使用以下流程：

1. 确认新 Hub 工件以及与其配对且已验证的 Agent 工件/commit。未验证该配对前不得继续。
2. 暂停新请求并等待进行中的请求完成。如果无法排空，记录相关 `runId` 并保留对应的 Agent 账本条目。
3. 停止 Hub。在 Hub 停止期间，备份实际 Hub 配置和 SQLite 数据库；若存在，连同数据库备份保留匹配的 `-wal` 和 `-shm` 文件。
4. 部署配对工件，重启 Hub 并重新连接 Agent。
5. 执行一项匹配的请求，然后确认 `GET /v1/runs/{runId}` 返回匹配的 `agentId`、`runId` 和 `requestId`，且 `status=completed` 并包含预期 `result`。

不需要 SQL、数据库/schema、配置格式或 Room 文件迁移。之前曾宽松接受的缺失 `runId` 现在会被拒绝。拒绝时使用 `error.code=agent_message_rejected`，以及以下固定原因之一：`response_run_id_required`、`response_run_mismatch`、`response_result_conflict`、`response_result_store_failed` 或 `response_waiter_owner_mismatch`。

上述 `runId` 要求适用于可靠命令 `Response` 消息，不适用于确认消息。确认请求不携带 run id；其决策归属于已捕获的连接/请求和原始发送方。本地控制面等待超时不等于远程取消：匹配的迟到 ACK/status/Response 仍可能推进运行回执；`not_sent` 只表示 channel-send 失败，且永不重放。

按已接受的 D06 行为，Hub 重启会丢失同步等待者、OAuth session 和待处理 session。不得为恢复 HTTP 等待而重新执行命令。持久化 SQLite 结果和 Agent 账本会保留。

如需回滚，应暂停新请求，只恢复旧 Hub 二进制，并继续使用当前数据库和 Agent 账本。绝不能用旧数据库备份覆盖新结果。旧二进制会重新暴露 Response 所有权缺陷，因此回滚并非没有风险。

### Hub 连接代际切换

迁移到按连接代际安全准入 WS/SSE 连接的 Hub 构建时，使用以下流程：

1. 暂停新的 Hub 调用并排空进行中的请求。若某项请求无法排空，记录其 `runId` 并保留对应的 Agent 传输账本条目。
2. 停止 Hub。备份实际配置和 SQLite 数据库；若存在，连同数据库一并保留匹配的 `-wal` 和 `-shm` 文件。
3. 部署已验证的 Hub 工件并重启。重新连接每个 Agent；每次 SSE 重连都必须使用全新且非空的 `connectionId`。
4. 验证 `/v1/info`、`/v1/agents`、一条当前 Heartbeat/ack 路径、一项过期生命周期消息拒绝，以及一项匹配的过期可靠结果。Hub 重启可能丢失内存连接、等待者和 session；不得为恢复 HTTP 等待而重新执行命令。

不需要 SQL、数据库/schema、wire 或配置格式迁移。每个已注册 Agent 在 Hub 中只保留当前内存连接；在同一代际边界内，替换连接会使先前的 Room 租约和 stream 退役。

如需回滚，应暂停新的调用、停止新 Hub，只将 Hub 二进制替换为先前已验证的工件，并继续使用最新数据库和 Agent 账本。绝不能用旧数据库备份覆盖较新结果。旧二进制会重新暴露连接代际机制之前的竞态，因此回滚会恢复该风险。

### WP3 存储恢复与回滚

存储恢复不是副作用回滚流程。恢复任何 Hub 数据库、Agent 配置、传输账本或审计文件前，都要停止拥有该文件的进程（Hub 和受影响的 Agent），并保留当前数据的只读副本。不得用旧副本覆盖较新的去重、结果、冲突或未知证据。

Hub SQLite 会在 `user_version=1` 执行事务式 schema 迁移。在迁移或压缩已完成结果的保留数据前，Hub 会在数据库旁创建私有一致性快照，文件名为 `.pre-migration.bak` 或 `.pre-retention.bak`。进行任何手动操作前，都应保留当前数据库及匹配的 `-wal`/`-shm` 文件。优先只回滚二进制，同时继续使用最新数据库；只有在比较 run 身份并保留较新数据库作为证据后，才可有意将旧快照用于数据恢复。恢复快照无法撤回已经发送给 Agent 的命令，也无法撤销外部副作用。

Agent 进程历史持久存储在私有 `process.sqlite3` 中，并遵循该存储自己的保留规则。该进程存储会全新创建：旧 `jobs.sqlite3` 会有意保持原样，并且对新的进程生命周期不可访问。没有旧 Job 历史迁移或 `jobs.sqlite3` 迁移备份流程。

Agent 配置写入会在 `backups/` 下保留有界私有备份，并使用 setup journal 替换 secret 引用。如果 journal 中的哈希或文件类型不对应任何预期的变更前/变更后状态，启动会 fail closed 并报告恢复冲突。停止 Agent，保留 journal 和当前文件，并从经过验证的副本解决冲突；不要删除 journal、强行选择一侧，也不要在诊断信息中暴露 secret 内容。

传输账本绑定所有者并使用文件锁。记录损坏或最后一行被截断时会 fail closed，并将原始字节保存在私有 `.recovery` sidecar 中；压缩可能会在 `.backup` 中留下先前账本。保留这些原始文件以供诊断，不要为了让启动通过而丢弃或截断账本。旧的无所有者记录继续保持 `LegacyUnowned`：不会自动协调、执行，也不会用于披露结果。恢复须由运维人员检查保留的原始记录和更新的所有者绑定证据；绝不能为绕过所有权错误而删除或截断去重证据。Agent 审计 JSONL 采用尽力写入，并在达到 8 MiB 时轮转，保留当前文件和一个 `.1` 备份。轮转或写入失败意味着审计记录丢失，不能证明命令、结果或副作用不存在。任何恢复之后，重新连接拥有这些状态的进程并检查进程状态；没有独立终止证据时，不得将 `unknown_after_restart`、`unknown`、`detached` 或本地等待者超时转成 `cancelled`。


## v0.9 验收清单（历史记录）

本节是 v0.9 版本的历史发布证据，不是当前契约或当前验证入口。查阅该版本时应保留其记录的版本号/数量；当前工件请使用上方的仓库验证和当前冒烟检查章节。

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

agentic-gpt --version
agentic-gpt local list-tools --config ~/.agentic_gpt/config.json
agentic-gpt local call agent.info \
  --config ~/.agentic_gpt/config.json \
  --arguments '{}'
```

该版本的预期契约：

- `agentic-gpt` 报告 `0.9.0`；Hub 模式还要求 `agentic-gpt-hub 0.9.0`。
- Normal 本地/tunnel 接口面暴露 23 个工具，Room 暴露 34 个。
- 存在 `mcp.batch`、`mcp.callTool`、`job.get`、`job.list` 和 `job.cancel`。
- 不存在 `process.batchExec`、受管理的 `session.*` 和 `process.get/list/kill`。
- 存在 `agent.info.execution.jobs` 和 `agent.info.mcp.concurrency`。
- MCP 并发数报告全局上限为 8、每服务器上限为 2。
- 本地 Unix MCP 使用权限为 `0700` 的运行时目录和权限为 `0600` 的套接字。
- 本地 Unix 与 tunnel 的描述符/schema revision 一致。
- 新启动的隐藏 worker 会在 `initialize` 前恢复已续接调用，保留原始 ID 并继续运行。
- `config.example.json` 能严格解析、通过 Standalone 验证，且不包含可用凭据。

无副作用的 `mcp.batch` 冒烟检查可以使用重复的调用 ID。它必须在确认/连接下游之前失败，并写入一条聚合 `validation_rejected` 审计记录，且不能启动子调用：

```bash
agentic-gpt local call mcp.batch \
  --config ~/.agentic_gpt/config.json \
  --arguments '{
    "calls": [
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}},
      {"id":"dup","serverId":"configured-server","toolName":"probe","arguments":{}}
    ],
    "waitSeconds": 0
  }'
```

预期错误：`mcp_batch_failed`，消息前缀为 `mcp_batch_call_id_duplicate`。

创建 Hub release tag 前，使用指定的 Actions importer 验证 [`openapi/hub.yaml`](../openapi/hub.yaml)。打 tag、部署、迁移和重启 connector 是彼此独立的操作。

## 安全不变量

- `SafeConfigSummary`、`/v1/info` 等安全概要不会返回 Tunnel、Hub、Agent 或 ntfy 凭据值；报告和审计负载也不应包含这些值。部分受支持的 CLI 配置选项（例如 `agentic-gpt config init --agent-secret`）可以从 argv 接收凭据值；以明文传入时，本地进程检查和 shell 历史可能会暴露它。可用时优先采用该设置支持的隐藏交互输入；仅在相应设置支持时使用受保护的 `file:`/`env:` 引用（见[配置说明](configuration.md)）。不要假定所有设置都有通用安全的命令行格式，也不要在命令示例或 argv 中放入真实凭据。
- OpenAPI 只公开 GPT Actions 端点；OAuth 和确认回调不在其中。
- 安全概要只包含计数/粗粒度模式，不包含密钥或完整私有路径列表。
- Agent 本地拒绝确认或确认决策超时后，操作即为最终拒绝；这与 Hub/HTTP/MCP 等待者超时不同。
- 长时间工作使用受管理 Process 和有界等待。
- 有界的 Hub/HTTP/MCP 等待超时只结束本地等待，不会停止远程 Process。迟到的远程信息仍可能更新运行回执；`not_sent` 仅表示已证实的通道发送失败，且不会重放。
- Standalone 上报功能为可选项，且仅用于上报；它绝不是隐藏的共享命令依赖。
- 无效的实时配置会保留最后一个有效子集；启动身份变更需要重启。
