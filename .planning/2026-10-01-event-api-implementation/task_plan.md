# 持久异步事件系统实现

目标已启用；本任务目录 .planning/2026-10-01-event-api-implementation，不覆盖研究计划或切换其他任务 active 指针。用户已授权开始实现。

## 定稿约束
- 每个 Agent 一份共享 inbox，各客户端、入口和 Hub 访问同一份；跨 Agent 隔离。
- 保留业务结果，仅附 events: { current: "low: N | medium: N | high: N", new: [{ "eventId | 摘要": "severity | 日期" }] }。
- 统计全部 pending 且未过期事件，包含隐藏事件；最多5条，摘要32 Unicode字符（31+省略号），等级时间ID不截断。
- low曝光1次、medium3次后隐藏，high持续候选；low默认24小时TTL可配置，medium/high不自动过期。
- list/get/mark独立事件API；get完整message，mark只handled且幂等，不管理进程。
- source创建时固定：process/skill_install内部指定对应ID；外部入口固定external，调用者只提供ref。
- 内部全部默认low，按稳定事件类型配置low/medium/high/off。原响应已包含终态结果则不产生事件；未包含才异步通知，不依据线程或时间先后。
- 仅下一次调用捎带，无主动推送；handled/expired保留7天。
- TUI仅配置覆写，复用现有组件，不新增事件管理界面、不补齐未完成控制功能。
- 外部独立CLI接受stdin JSON，提交现有Unix MCP；不污染服务stdin。

## 阶段一：合同与所有者
Status: complete
归档最终合同，核对CodeGraph/LSP、配置TUI与入口/消费者，确定共享接口；提交本阶段记录。

## 阶段二：核心与配置
Status: in_progress
持久store/DTO/计次/过期/分页/API核心与回归测试；配置和配置TUI；阶段提交。

## 阶段三：生产与跨入口
Status: pending
生产者响应仲裁/恢复、外部注入、Agent全MCP、HubMCP/API及wire/OpenAPI和消费者；阶段提交。

## 阶段四：文档与验收
Status: pending
同步相关文档；真实入口、配置TUI；fmt/workspace tests/build/live parity全部exit0；阶段提交。

## 验收与停止
- cargo fmt --all -- --check
- cargo test --workspace
- cargo build -p agentic-gpt -p agentic-gpt-hub
- python3 scripts/check_contract_parity.py（隔离状态与所需依赖，真实覆盖新合同）
- 隔离配置TUI实际操作覆写，并验证真实生产。
- 最多3轮验收失败→修复→重验。定稿未覆盖的实质公共契约歧义、破坏性用户数据迁移、真实凭据/部署需求立即停报。
- 不改apply-patch/browser-host/发布部署；Console仅必要合同适配；不改用户已有改动。

## 错误与验收轮次
URI脚本调用错误已解决；完整验收修复轮次0/3。

## 用户补充确认的Hub例外
- 无单一目标Agent的Hub调用不附events。
- Agent离线/请求超时后的Hub错误或缓存业务结果不附events，不报假零、不新增陈旧快照缓存。
- 其余有明确在线目标Agent的响应仍覆盖同一收件箱。
- 已确认，继续实现；共享代码接口见local://event-implementation-interfaces.md。
