# 已核实事实
- 初始 git status --short 为空，工作树干净。
- ProcessResponse 混合 status/state/completedInline/pollAfterMs/inlineOutput/outputPreview/result*；managed.rs creation_response 会因输出预算改 completedInline=false，即使 state 已终结。
- 现有 status 元数据-only，wait_for_process 等终态/期限；output 显式双流 cursor、gap、hasMore/eof/captureStatus；MCP result 保存完整 CallToolResult。
- 输出每流 64KiB ring；MCP 完整结果保留上限 512KiB。响应预算须与保留预算分离。
- 现有 ProcessInfo 被持久化、Hub 快照/缓存消费者依赖。统一紧凑响应不能丢失内部快照所有权；需保留内部 ProcessInfo 或独立投影。
- 进程完成事件持久化后搭载后续响应，不代表外部 MCP host 自动唤醒。
- 研究阶段已实际运行隔离 local Unix MCP 短命令/status/output 两页；本次改动后仍需重新验证新接口。

- 消费者调查：Console 未发现旧 process.status/output/result 调用，Rust Process TUI 仅用 process.list，故目前不涉及 Console 平台改动。
- 配置放现有 LimitsConfig；live reload 已整体更新 limits。TUI 需改 SetupField/LimitsDraft/validation/review/pages/app，不另建配置写入路径。
- 响应预算固定为 Process JSON 主体，MCP/HTTP封套及独立 event panel 不重复计入该领域预算；4096..1048576，默认8192。上限留足512KiB MCP保留结果及元数据，避免最大保留结果永久无法领取。
