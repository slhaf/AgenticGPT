# 已确认事实

- 用户确认只保留 executable/cacheDir，删除 version/autoDownload/downloadUrl/sha256；自定义 executable 也使用内置客户端 SHA 校验。
- 官方 v0.0.19：amd64 归档 SHA `9ee9448592345a9dd1517fb79791ab54d24f051142eb92bcbe8cd355effc9142`，可执行文件 SHA `56d0120886ce91bfe31e7a5600f6a6594199a02e9b3eac671d5da0d7480c97a3`；arm64 归档 SHA `5a19e556c4a8736ab7a6fde6f0568f6e7411fa2dff21186fc15bc30e9b98a603`，可执行文件 SHA `8a50b25d6055bc6e3e3a5c18c098637ab2be63f6e453b8ebebd02433deadba04`。从下载的官方归档核验并提取计算。
- 官方新包包含 cloudflared、许可证及 SBOM；当前只允许单文件的提取器会拒绝。
- 当前 Agentic supervisor SIGKILL 后，真实 v0.0.10 客户端仍存活；同配置重启曾发生 `local_mcp_socket_in_use`。doctor 在停止信号处理器安装前执行，也有子进程残留窗口。
- v0.0.19 新增 worker 失败自动重启策略；选 `--mcp.stdio-failure-policy=exit` 将恢复交给 Agentic supervisor。
