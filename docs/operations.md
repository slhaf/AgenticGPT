# 运维指南

本文记录可复现部署所需的最低限度检查。Standalone 是主要部署路径；Hub 是可选组件，因此 Hub 检查单独列出。

## 仓库验证

按 `.github/workflows/ci.yml` 指定的顺序运行 Rust CI 检查：

```bash
cargo fmt -p agentic-gpt -p agentic-gpt-hub -p agentic-gpt-protocol -p agentic-apply-patch -- --check
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

真实 Agent 集成测试为每个夹具设置短且唯一的临时 `HOME`、`XDG_CONFIG_HOME`、`XDG_DATA_HOME`、`XDG_CACHE_HOME`、`XDG_RUNTIME_DIR` 和 `TMPDIR`，子进程显式继承这些目录；不得通过修改测试进程的全局 `HOME` 隔离并行用例。runtime/socket 仍位于 `HOME/.agentic_gpt/runtime/agent/<agentId>/`，完整套接字路径不得超过 100 字节，长度应按夹具主目录计算。

清理时先停止并等待拥有状态的进程退出，再删除临时根；失败和提前返回也应执行清理。

## 私有状态布局

当前布局统一使用 `~/.agentic_gpt/state/` 保存 `events.sqlite3`、`process.sqlite3`、`email.sqlite3`、`email/`、`active-skills.json` 和 `skill-installs/`；这些状态不绑定 Agent ID。runtime/socket 路径仍按 `agentId` 分隔，Hub 的连接身份、路由及传输账本所有者约束保持原有协议语义。

启动过程不会扫描、合并或自动改写旧版私有状态目录。当前仓库不提供把旧配置或旧状态转换为
现行格式的通用迁移脚本；需要保留旧部署数据时，应先停止使用该 `HOME` 的全部 Agent，完整
备份配置、数据库及 sidecar，并按该旧版本对应的版本化发布说明处理。不要把历史迁移步骤当作
当前命令接口，也不要让新实例直接覆盖尚未核验的旧目录。

邮件图片由 Obscura 渲染原始 HTML 页面，并按页面内容加载资源，包括远程资源和 CID 内嵌资源；运行需要可用系统字体。邮件本地内容受保存天数与容量上限控制；导出到 workspace 的附件不参与该清理。账户连接、图片视图的用法、首次导入与重启后的提醒行为见[Room 邮件使用指南](email.zh-CN.md)。服务器邮件编号变化导致 `recoveryRequired` 时，先停止 Agent，再使用 `agentic-gpt email --config PATH recover --notify-existing` 或 `--skip-existing-notifications` 显式选择恢复提醒方式；旧副本保留，新导入生成新 ID。详情和续读失效处理见[邮件恢复与读取](email.zh-CN.md)。

## 本地/Standalone 冒烟检查（主要路径）

Tunnel 凭据不可用时设置 `mode=local`；要验证完整的推荐路径则使用
`mode=standalone`。两种模式都从 `agentic-gpt run` 启动。

下面示例使用一次性目录检查 Local 入口；在子 shell 中执行，退出时先等待服务停止再删除目录。Standalone 冒烟也应使用相同隔离方式，并显式配置测试用 tunnel 凭据和客户端。

```bash
(
  smoke_root=$(mktemp -d /tmp/as.XXXXXX)
  smoke_pid=
  cleanup() {
    if [ -n "$smoke_pid" ]; then
      kill "$smoke_pid" 2>/dev/null || true
      wait "$smoke_pid" 2>/dev/null || true
    fi
    rm -rf "$smoke_root"
  }
  trap cleanup EXIT
  export HOME="$smoke_root/h" XDG_CONFIG_HOME="$smoke_root/c"
  export XDG_DATA_HOME="$smoke_root/d" XDG_CACHE_HOME="$smoke_root/k"
  export XDG_RUNTIME_DIR="$smoke_root/r" TMPDIR="$smoke_root/t"
  mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$TMPDIR"
  chmod 700 "$HOME" "$XDG_RUNTIME_DIR"
  agentic-gpt --version
  agentic-gpt config init --mode local --profile normal --non-interactive || exit
  agentic-gpt run &
  smoke_pid=$!
  smoke_ready=false
  for attempt in $(seq 1 100); do
    if agentic-gpt local list-tools; then smoke_ready=true; break; fi
    kill -0 "$smoke_pid" 2>/dev/null || exit 1
    sleep 0.05
  done
  "$smoke_ready" || exit 1
  agentic-gpt local call agent.info --arguments '{}'
)
```

预期结果：

- `agent.info.connections.localMcp.status` 为 `ready`。
- 运行时目录权限为 `0700`、套接字权限为 `0600`，且只接受相同 UID。
- 存在 `event.list`、`event.get`、`event.mark`，以及 `process.exec`、`process.batch`、`process.read`、`process.list`、`process.cancel`、`mcp.callTool` 和 `mcp.batch`。
- 事件收件箱工具独立于工具集开关；隐藏事件不因此视为已处理。它们管理收件箱状态，不是进程控制接口。
- Process 历史使用 `~/.agentic_gpt/state/process.sqlite3`；事件历史使用独立的 `~/.agentic_gpt/state/events.sqlite3`。现有 `jobs.sqlite3` 会有意保持原样，不能通过 Process 接口访问；运行时不会自动迁移或备份旧 Job 历史，升级前应自行保留并备份旧数据库。
- [持久事件收件箱](interfaces.md#持久事件收件箱)说明同一私有状态根的共享收件箱和提醒范围。
- 已移除的 v0.8 受管理生命周期名称不存在。

Standalone 还应确认：

- tunnel-client `doctor` 和 loopback 就绪检查通过；
- ChatGPT connector 可以调用 `agent.info`；
- 一台机器断开连接不会影响其他机器的 tunnel；
- 重启后恢复的首次调用不会产生 `expect initialized request`，也不会重启 worker 进程对；
- 仅在需要恢复时才会出现 `mcp_stdio_session_resume` / `mcp_stdio_session_resumed`。

## Hub 冒烟检查（可选的集中式模式）

示例使用 loopback Hub 的固定测试值 `test-key`；这是公开且不含保密信息的本地测试值，只适用于此本地示例。它会出现在服务环境和 `curl` argv 中，绝不可将真实部署密钥替换到这里；真实密钥不要作为 argv 传递。

先按仓库验证步骤构建二进制，再执行隔离示例；构建工具继续使用开发环境的 HOME，只有服务子 shell 使用临时 HOME。

```bash
(
  smoke_root=$(mktemp -d /tmp/hs.XXXXXX)
  smoke_pid=
  cleanup() {
    if [ -n "$smoke_pid" ]; then
      kill "$smoke_pid" 2>/dev/null || true
      wait "$smoke_pid" 2>/dev/null || true
    fi
    rm -rf "$smoke_root"
  }
  trap cleanup EXIT
  export HOME="$smoke_root/h" XDG_CONFIG_HOME="$smoke_root/c"
  export XDG_DATA_HOME="$smoke_root/d" XDG_CACHE_HOME="$smoke_root/k"
  export XDG_RUNTIME_DIR="$smoke_root/r" TMPDIR="$smoke_root/t"
  mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$TMPDIR"
  chmod 700 "$HOME" "$XDG_RUNTIME_DIR"
  target/debug/agentic-gpt-hub --db "$smoke_root/hub.sqlite3" --config "$smoke_root/hub.json" init || exit
  AGENTIC_GPT_API_KEY=test-key target/debug/agentic-gpt-hub \
    --db "$smoke_root/hub.sqlite3" --config "$smoke_root/hub.json" serve --bind 127.0.0.1:18787 &
  smoke_pid=$!
  smoke_ready=false
  for attempt in $(seq 1 100); do
    if curl -fsS -H 'Authorization: Bearer test-key' http://127.0.0.1:18787/v1/info; then
      smoke_ready=true
      break
    fi
    kill -0 "$smoke_pid" 2>/dev/null || exit 1
    sleep 0.05
  done
  "$smoke_ready"
)
```

预期 JSON 包含 `service`、`version`、`remoteConfirmation`、`agents`、`counts` 和 `generatedAt`。

### Process Bash 输入切换检查

部署采用新 Process 输入契约的配对 Agent/Hub 工件时，应先更新所有调用方：`process.exec` 使用原始 `command` 脚本与可选 `cwd`；`process.batch` 的每项使用 `command`，批次级 `cwd` 可被元素覆盖。`program`、`args`、`workingDirectory` 不再是执行输入，客户端若需表达 argv，必须自行按 shell 规则显式引用参数并构造脚本。

保留在历史 `ProcessInfo` 中的旧元数据不构成输入兼容性。此次切换不改写数据库；旧未完成记录不会重放，已完成记录保留其历史结果。不要把此部署说明解释为发布或推送已发生。

对已授权的本地实例，可用只含字面量 `printf` 的无副作用脚本做定向冒烟，例如 `process.exec` 的 `command` 为 `printf '%s\n' 'process smoke'`。确认进程退出码为 0 时状态为 `completed`，再通过 `process.read` 检查捕获输出；非零退出码应为 `failed` 并保留实际退出码。读取时分别观察执行终态与 `captureStatus`/EOF；`waitSeconds` 只结束本次等待，不取消执行。

需验证取消时另启动获准的长时无害任务并显式调用 `process.cancel`：只有 `process_group_sigterm_observed` 或 `process_group_sigkill_observed` 表示观察到对应停止信号；unknown、detached 或缺少证据不能作为停止证明。普通同组管道和后台子进程属于目标范围；不要将该检查外推到脱离进程组的后代。

Shell `initFile` 是本地可信配置，不由模型输入控制；部署前按[配置说明](configuration.md)核对 Default/Disabled/显式路径以及文件所有权和可写性。策略预检针对提交脚本，不审计 init 文件，且配置快照不冻结文件内容或可执行身份。相对 init 路径若随模型可选 cwd 或可写位置变化，属于信任风险；优先使用用户所有、受保护的绝对路径。此为运维建议，不是新增执行限制。

### WP1 收尾证据边界（历史记录）

本节保留 Process 切换前、使用 Job 接口时的 WP1 观测，不是 0.10.0 的验收结果。该阶段证据范围有限：最终清理阶段通过了 `cargo fmt --all -- --check`、集成的 `cargo test -p agentic-gpt-hub` 套件，以及无警告的 `cargo build -p agentic-gpt-hub`。一次隔离的 Hub HTTP/SSE 冒烟检查使用模拟 Agent 对等端，覆盖了六种确认、替换、回调、超时和迟到回执场景。

此后，针对同一临时 SQLite 数据库执行的真实 Hub 重启保留了两条状态为 `completed` 的 `/v1/runs` 记录和 `sessions[]`；`/v1/info` 则报告 `pendingRequestCount`、`pendingConfirmationCount` 和 `cachedJobCount` 均为零。

`cachedJobCount` 是当时的观测字段；当前 `/v1/info` 使用 `cachedProcessCount`。这里保留历史结果，不作为当前字段清单。

这只证明 Hub 侧的回执/会话清理和保留行为；没有执行真实 Agent 或传输账本重启，也不能证明历史路线图中的所有场景都完成了端到端验证。外部 ntfy 提供方行为仍未验证；本地/mock 回调路径不能作为等价证据。

## Standalone 部署检查

1. 确认 `agentic-gpt --version` 是预期版本。
2. 确认 tunnel secret 使用受保护的 `file:` 或 `env:` 引用。
3. 确认 `agentic-gpt run` 以 `mode=standalone` 启动后达到就绪状态，并在重启预算重置间隔之后仍保持稳定。
5. 启动一个无害进程，并使用 `process.read`（auto/status）检查输出和状态；只有明确要求取消时才调用 `process.cancel`。
6. 重启一个 Agent，并确认其他机器的 connector 仍可用。
7. 确认审计 JSONL 位于 `workspaceRoot` 下，且不含原始 tunnel/MCP secret。

## 配置重载与重启诊断

三种 Agent 运行模式共用配置应用协调。API 更新等待本次保存与局部应用尝试后返回；CLI/TUI 写入由运行实例监听处理。先查 `config.manual`，再用 `config.get` 核对 `value/appliedValue/applyStatus/error`；不能把磁盘写入或 `saved=true` 当作运行组件已就绪。工具集 `config` 默认关闭，可通过本地 `config toolset enable config` 开启；API 写入默认要求明确确认，本地 CLI/TUI 不受该 API 策略限制。

| 观察结果 | 处理 |
| --- | --- |
| `applied` | 对应设置已应用；外部凭据、网络和真实调用仍须独立观察。 |
| `restart_required` | 只有 mode/profile/agentId/workspaceRoot、Tunnel 身份／凭据／client 启动参数保留该边界；安排显式重启实例。 |
| `applying` | 查看对应连接状态；不要重复提交原用户副作用。 |
| `failed` | 查看逐项 error 并修复组件环境；已保存配置不整份回滚，旧 owner 可保留时继续服务。 |

Hub、HTTP、Browser、邮件账户、Secret 后端和 Skills 并发由具体所有者协调，不重建整个 AppState。重配置不会清空 Process 注册表、持久事件、安装账本或发送结果；不会自动重发邮件、重写密钥或重放网页操作。降低安装并发不取消现有任务；关闭 Skills 保留恢复记录，重新启用时恢复排队安装，不重复启动已被 worker 占有的记录。Browser 切换会清理旧租约与 JS 会话，Tmux 已有会话不因工具集关闭而删除。

Standalone HTTP MCP 按请求认证。更换端点会丢弃原监听器的 OAuth 状态，原地更新 bearer 凭据也会撤销 OAuth 记录。认证引用不可用时拒绝访问，端口绑定失败由组件重试，不影响其他入口。邮件/Secret 等排空繁忙时保留原 owner 并报告失败；静态校验不检验远程登录可用性。

`tunnel.hubReporting.detail` 和 `tunnel.hubReporting.enabled` 更新后续报告策略，不要求重建 Tunnel，也不会断开确认连接；辅助连接单独由 `hub.enabled` 控制。连接关闭时会保留报告设置并暂停发送。旧配置若报告开启但 `hub.enabled=false`，需要明确打开 `hub.enabled` 才会继续连接并上报。配置的待重启 mode/profile 不会阻止无关热更新。Standalone supervisor 在私有 runtime 目录用 0600 启动快照保存本次启动契约；自动重拉 worker 保留原身份并读取最新可热字段，显式重启整个实例才采用新的启动值。快照由 supervisor 清理，不覆盖 desired 文件。

`config.update` 传输超时不能证明未保存，先用 get 核对；普通校验失败未提交，rename 后的目录同步失败则明确报告文件已替换。Hub 对 update 的等待上限为 180 秒，不自动重试修改。完整字段分类见[配置生效边界](configuration.zh-CN.md#热加载与重启边界)。

`agentic-gpt local` 命令是仅所有者可用的 Unix MCP 客户端，并保留 `local:` 审计来源。
它不同于 `agentic-gpt tmux` 本地管理员 CLI；后者只暴露四个命令：`list`、
`attach`、`create` 和 `close`（`RequestContext` 中的操作名称为 `tmux.listSessions`、
`tmux.attach`、`tmux.createSession` 和 `tmux.closeSession`）。

这些 CLI 调用使用
`localadmin:` 来源，不增加远程审批语义，也不伪造 AppState。其他接入来源仍为
`tunnel:`、`http:` 和 `hub:`。

## Hub 部署检查

1. 记录 `agentic-gpt --version`、`agentic-gpt-hub --version` 输出、工件路径和校验和，
   以确认 Hub 与 Agent 二进制是预期配对工件。不得用版本号或服务名称代替配对检查。
2. 确认 `/v1/info` 可通过公共 HTTPS 响应。
3. 确认 `/v1/agents` 显示预期的、具备命令能力且在线的 Agent。
4. 通过 `/v1/process/exec` 运行一条无害命令。
5. 通过 `GET /v1/process` 检查进程元数据；从 `GET /v1/process/{processId}/read` 读取输出或状态，并通过 `POST /v1/process/{processId}/cancel` 请求取消。
   - read 的 `waitSeconds` 默认 5 秒、上限 30 秒、0 表示立即。
   - `maxBytes` 省略时使用 `limits.processResponseBytes`（默认 8192 字节），显式范围为 4096..1048576。
   - cursor 非消费且不共享，仅用于 command/skill；`captureStatus`、`gap`、`eof` 和 `hasMore` 由读取响应报告。
   - MCP CallToolResult 使用 `pending`/`included`/`deferred`/`unavailable`/`not_retained`，完整对象不切碎。
6. 验证 `/mcp`；契约发生变化时刷新 GPT Actions schema。
7. 如果启用了 Standalone 上报，确认仅上报的连接会拒绝 Hub 执行请求。

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

Agent 进程历史持久存储在 `~/.agentic_gpt/state/process.sqlite3` 中，并遵循该存储自己的保留规则。没有迁移既有 Process 历史时，该存储会全新创建；旧 `jobs.sqlite3` 保持原样，不导入新 Process 数据库，也不能通过新的进程生命周期访问。需要保留旧布局中的 Process 数据时，按[私有状态布局](#私有状态布局)所述备份并使用匹配的版本化资料处理；当前仓库不提供布局转换脚本，启动也不会自动转换。

Agent 配置写入会在 `backups/` 下保留有界私有备份，并使用 setup journal 替换 secret 引用。如果 journal 中的哈希或文件类型不对应任何预期的变更前/变更后状态，启动会 fail closed 并报告恢复冲突。停止 Agent，保留 journal 和当前文件，并从经过验证的副本解决冲突；不要删除 journal、强行选择一侧，也不要在诊断信息中暴露 secret 内容。

传输账本绑定所有者并使用文件锁。记录损坏或最后一行被截断时会 fail closed，并将原始字节保存在私有 `.recovery` sidecar 中；压缩可能会在 `.backup` 中留下先前账本。保留这些原始文件以供诊断，不要为了让启动通过而丢弃或截断账本。

旧的无所有者记录继续保持 `LegacyUnowned`：不会自动协调、执行，也不会用于披露结果。恢复须由运维人员检查保留的原始记录和更新的所有者绑定证据；绝不能为绕过所有权错误而删除或截断去重证据。

Agent 审计 JSONL 采用尽力写入，并在达到 8 MiB 时轮转，保留当前文件和一个 `.1` 备份。轮转或写入失败意味着审计记录丢失，不能证明命令、结果或副作用不存在。

任何恢复之后，重新连接拥有这些状态的进程并检查进程状态；没有独立终止证据时，不得将 `unknown_after_restart`、`unknown`、`detached` 或本地等待者超时转成 `cancelled`。

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
- Agent 本地确认被明确拒绝或取消后中止操作；主动确认超时、过期或渠道不可用时按配置策略回退，仅 Allow 可继续，Confirm 不重复请求。这与 Hub/HTTP/MCP 等待者超时不同。
- 长时间工作使用受管理 Process 和有界等待。
- 有界的 Hub/HTTP/MCP 等待超时只结束本地等待，不会停止远程 Process。迟到的远程信息仍可能更新运行回执；`not_sent` 仅表示已证实的通道发送失败，且不会重放。
- Standalone 上报功能为可选项，且仅用于上报；它绝不是隐藏的共享命令依赖。
- 无效的实时配置会保留最后一个有效子集；启动身份变更需要重启。

## 策略配置兼容性

当前配置只接受统一的 `policy.enabled`、`policy.command`、`policy.path` 与 `policy.bubblewrap`。旧 `pathPolicy`、`sandbox`、扁平命令 `policy`、`needConfirm`、
`confirmMethod` 及更早的规则对象不会被自动转换。升级前保留原配置，根据
[配置说明](configuration.zh-CN.md#命令与路径策略)显式重建具体路径根与命令规则，再通过隔离的
`HOME` 验证。请求调用方使用含必填 `reason` 的 `acquireConfirmation`，删除旧 `acquire` 字段；不需要主动确认时省略整个对象。Agent 与 Hub 须同步升级，详见[调用方升级](configuration.zh-CN.md#调用方升级)。下游 MCP 审批合同保持原有语义。
