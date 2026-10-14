# 本地 stdio MCP 桥接

## 目标与合同
新增 `agentic-gpt local stdio --config PATH`，由客户端启动一进程一连接的透明 stdio→Unix MCP 桥接。运行时仍由 `run` 持有；同 UID、LocalUnix 来源和准入不变。只连接已有 Agent；不自动启动、重连、重放或代握手。任一方向 EOF/错误即关闭连接退出，stdout 仅协议。独立连接共享 Agent 状态和现有 16 条容量。

## 阶段
1. [complete] 实现 CLI、桥接和真实集成回归；定点验证后提交。
2. [complete] 同步用户文档，按 GitHub CI workflow 模拟 migrations、工具链、fmt/check/clippy/test、runtime build、Python dependencies 和 live contract parity，并完成隔离链路验证、独立审查；提交并汇报。

3. [complete] 删除既有 CI 失效迁移测试步骤，重新按 workflow 完整模拟并提交。

## 测试保留价值
新增⑤链路测试保护真实 public CLI 的握手透传、双向完整消息、多客户端隔离及共享 daemon 状态、stdin EOF/daemon disconnect 释放资源、服务缺失不启动 runtime。现有 local_control 集成和 Tunnel stdio 测试都不经过这个新桥接入口，因此不能发现桥接预握手、响应串线或标准输入阻塞退出回归。拒绝①②③固定文本/helper 测试。

## 执行边界
测试外层与派生进程必须隔离 HOME/XDG/TMP/D-Bus，停止等待后清理。root 集成、验证、分阶段提交。tests/docs 工作者各编辑自己的文件。

## 设计决策
桥接使用进程专用标准流线程避免 Tokio stdin 阻塞退出；显式8KiB read/write_all避开Linux splice交互死锁。所有者仍是已有 Agent。
