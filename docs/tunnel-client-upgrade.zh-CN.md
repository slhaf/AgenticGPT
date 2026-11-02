# tunnel-client 升级说明

**适用范围：本次升级开始。** 本页说明 Agentic 将托管 tunnel-client 固定版本升级到 OpenAI 官方
`v0.0.19` 后，Standalone 用户需要手动完成的配置和进程检查。

## 升级前检查

升级前停止正在运行的 Agentic 实例，并检查是否有它启动的旧 `tunnel-client` 或 stdio worker 仍在运行。
先通过对应的 service manager 停止旧实例；确认仍有遗留进程时，核对进程命令行、父进程和所属配置，确保它
属于这台 Agentic 实例后再停止。不要只凭旧 PID 文件终止进程，也不要在旧客户端仍活动时启动同一个
Tunnel ID 的新实例。

## 更新配置

从 `tunnel.client` 中手动删除以下配置项：

- `version`
- `autoDownload`
- `downloadUrl`
- `sha256`

保留 `cacheDir` 和 `executable`。例如，将：

```json
"client": {
  "version": "0.0.10",
  "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
  "autoDownload": true,
  "executable": "/opt/tunnel-client",
  "downloadUrl": null,
  "sha256": null
}
```

调整为：

```json
"client": {
  "cacheDir": "~/.agentic_gpt/cache/tunnel-client",
  "executable": "/opt/tunnel-client"
}
```

如果不使用本地 executable，可将 `executable` 设为 `null` 或移除；Agentic 会使用托管缓存。修改配置时，
保留 `tunnelId`、`apiKey` 和其他 Tunnel 设置。

本地 `executable` 必须与本机平台内置的官方 `v0.0.19` 可执行文件 SHA-256 完全一致。自行构建的二进制、
其他版本，或内容有改动的副本都无法通过校验；请换成对应平台的官方文件，或移除 `executable` 使用托管
副本。文件缺失或摘要不匹配会导致启动失败，Agentic 不会改用托管缓存。

## 预下载并启动

可在首次启动前下载并验证托管客户端。此命令不需要 Tunnel ID 或 API key：

```bash
agentic-gpt tunnel-client download
```

若配置文件不在默认位置，显式指定它：

```bash
agentic-gpt tunnel-client download --config /path/to/config.json
```

该命令使用配置中的 `tunnel.client.cacheDir`；有效缓存会直接报告并成功退出，缺失或损坏时会下载或修复。
CLI 在 stderr 定期显示进度，并在开始、重试和结束时更新；有总大小时显示已下载字节数和百分比，未知时显示已下载字节数。
终端使用单行刷新，非终端逐行输出。指定了 `executable` 也不影响此命令，它仍会准备托管副本。

完成配置和进程检查后，再启动 Agentic。托管模式下，后续启动会在缓存缺失或损坏时自动下载或修复。

## 客户端恢复行为

`v0.0.19` 的 stdio worker 意外退出时，Agentic 启动客户端时传入 `--mcp.stdio-failure-policy=exit`，使
tunnel-client 以非零状态退出，由 Agentic supervisor 按其重启策略重新启动客户端。上游说明，worker 故障
时进行中的请求可能失败且不会重放；调用方需要按自身协议恢复或重试。

正常停止和重拉时，Agentic 会对 doctor 或 tunnel-client 所在进程组发送 SIGTERM；等待超时后发送 SIGKILL，
并检查进程组是否还有运行中的进程。Linux 上，Agentic 被强制终止时，直接启动的 doctor 或 tunnel-client 会收到父进程
退出信号；受管理的 stdio worker 发现 tunnel-client 退出时也会终止所属进程组，以释放本地 socket。已经脱离
该进程组的后代不受此机制约束；升级前仍应检查旧实例遗留的客户端和 worker。

## 相关说明

- [Standalone 运行与隧道客户端](standalone-runtime.md#隧道客户端版本与下载)
- [配置参考：Tunnel 配置](configuration.zh-CN.md#tunnel-配置)
- [OpenAI tunnel-client v0.0.19 发布说明](https://github.com/openai/tunnel-client/releases/tag/v0.0.19)
