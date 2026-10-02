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
Status: complete
持久store/DTO/计次/过期/分页/API核心与行为测试、配置和配置TUI source实现已交付并修正独立审阅发现；尚未运行验收。此阶段提交planning checkpoint，source在跨入口集成后统一提交，避免不完整ABI切换提交。

## 阶段三：生产与跨入口
Status: complete
生产者响应仲裁/恢复、外部注入、Agent全MCP、HubMCP/API、wire/OpenAPI及全部消费者已集成；五轮授权第2轮完整验收通过，真实跨入口/恢复/Room业务错误与缓存例外均通过。本阶段源代码提交c6f3b17。

## 阶段四：文档与验收
Status: complete
fmt/check/strict Clippy/workspace761 tests/1 ignored/build/live parity全部exit0；真实TUI保存/读回/同配置生产证据保留且相关代码未改变。文档已按实际合同更新，提交本阶段文档与最终证据。历史3/3、追加3/3、独立一次失败保留；最新五轮授权新增失败1/5，第2轮完整成功。

## 验收与停止
- cargo fmt --all -- --check
- cargo test --workspace
- cargo build -p agentic-gpt -p agentic-gpt-hub
- python3 scripts/check_contract_parity.py（隔离状态与所需依赖，真实覆盖新合同）
- 隔离配置TUI实际操作覆写，并验证真实生产。
- 最多3轮验收失败→修复→重验。定稿未覆盖的实质公共契约歧义、破坏性用户数据迁移、真实凭据/部署需求立即停报。
- 不改apply-patch/browser-host/发布部署；Console仅必要合同适配；不改用户已有改动。

## 错误与验收轮次
完整验收失败轮次3/3：首轮解析错误；第二轮typed Content新断言编译错误；第三轮fmt通过、workspace tests 758通过/1忽略、Agent/Hub build通过，但live parity的Agent事件多传输启动失败（tunnel_config_required）。达到上限，停止，不重置、不宣称完成；继续修复和重验需要用户明确授权。

## 用户补充确认的Hub例外
- 无单一目标Agent的Hub调用不附events。
- Agent离线/请求超时后的Hub错误或缓存业务结果不附events，不报假零、不新增陈旧快照缓存。
- 其余有明确在线目标Agent的响应仍覆盖同一收件箱。
- 已确认，继续实现；共享代码接口见interfaces.md。

## 用户补充确认：最终入口响应为权威
- 用户选择“最终入口返回”：Hub原调用超时而未包含终态，迟到Agent终态仍须产生事件；不是以Agent已生成terminal值为准。不承诺客户端阅读确认。
- Hub持久记录每original run的Returned/NoTerminal判定并可靠反馈Agent；Agent远程起源Awaiting不自行suppress或恢复false。
- 新内部wire metadata、来源origin绑定与反馈outbox接口见response_feedback.md；不改变公开三个event API，不增加主动模型推送。
- 验收新增真实Hubtimeout→lateAgentterminal→下一次在线调用一条event，以及Hub/Agentrestart未决仲裁恢复；其他已定合同、范围和3轮上限不变。

## 外部工具阻塞：goal无法重新激活
- 更新目标时goal(drop)后goal工具被宿主移除；create/get/read/xd/eval均不可用，已报告。公开CLI/RPC/SDK文档未提供恢复路由。
- 所有7个实现owner已确认HOLD；当前代码有未完成接线和符号，未编译/验收，不能当作可用实现。
- 最新完整五段目标保存在objective.md，详细暂停交接pause_handoff.md；需宿主恢复goal工具后重建active目标，再继续。
- 此为外部工具阻塞，完整验收修复轮次仍0/3；未提交未完成source作为完成交付。

## 目标与执行已恢复
- 用户重新启用guided-goal并明确要求载入已确认目标、无需重新访谈；goal(create)已成功返回active。
- 已完整恢复objective.md五段目标（排除过时工具阻塞说明），原定3轮上限及0/3计数不变。
- interfaces.md、response_feedback.md已保存为本任务可提交的共享合同，local://仅作会话辅助。
- 解除goal工具阻塞，通知原文件owner恢复，不重做已完成设置、不运行未集成代码的验收。

## 阶段二交接
- Core已补resolved policy tombstone压缩、RFC3339年份上限，Config同步边界；Agent公开event API及外部Unix CLI接入source已实现。
- 初步producer/Agent/Hub wire source齐备，Hub反馈coordinator/HTTP/OpenAPI与live parity仍在阶段三集成；完整命令/真实TUI尚未运行，验收0/3。

## 用户追加阶段：警告修复与问题解释
Status: complete
按用户新授权修复rustc warning及strict Clippy诊断，不压制warning。fmt/check/clippy -D warnings/workspace tests/Agent与Hub build均通过且无warning；新binary隔离Unix进程事件smoke通过。已定位并解释问题1fixture tunnel=null与Standalone run不匹配，以及问题2聚合只投影servers导致events隐藏计次。原live parity失败3/3仍blocked，未重跑、未修问题1/2；本阶段仅planning证据提交，source集成提交继续暂缓。

## 用户授权继续：两项修复与完整验收（历史轮次）
Status: complete
用户选择“修复两项，再允许最多3轮完整验收”。历史3次失败不重置；新增完整失败0/3。两个不重叠owner分别修parity多传输worker启动及live曝光覆盖、Rust内部聚合McpListServers不生成panel机制/所有caller与行为回归。主线程唯一fmt/check/clippy/test/build/live/TUI证据和commit owner；全部原目标及安全边界保留。

## 授权后验收计数
新增完整失败3/3：第一轮policy-reload临时socket路径过长；第二轮Hub delayed-response receipt replay检查误要求第3个settle；第三轮fmt/check/strict Clippy无warning通过，但workspace test的file_edit_apply_patch_revalidates_external_change_before_commit断言失败（Null != file_revision_conflict），命令链未到build/live parity。历史3次失败不重置，按授权上限停止，未标目标完成。

## 用户追加：失败原因调查
Status: complete
用户要求“调查”，仅诊断file.edit外部改写回归的Null错误断言；允许隔离执行因果诊断，不修改实现、不新增永久测试、不重启完整验收。历史3/3及新增3/3保持；事件系统完成任务仍blocked。主线程记录结论，scout只读追溯hook与调用链。
诊断已实际证明test-only单槽注入覆盖机制，另观察真实HTTP MCP无改写时的成功封套。结论、证据范围和未捕获原调度的限制见findings.md；实现未改，原全验收仍blocked。

## 用户追加：测试注入隔离修复
Status: complete
用户要求“可以，尝试修一下”。仅修复cfg(test)外部改写注入的单槽覆盖；增加确定性双Agent路径隔离的真实dispatch回归，先观察修前失败、再运行修后相关定点回归。生产代码与原断言不变，不重启完整验收；历史3/3及新增3/3保留。此内部测试修复不改变公共文档合同。
已取得新真实dispatch回归修前失败/修后通过证据，8项相关并行回归及fmt/strict Clippy通过；本次局部修复完成。没有重启全验收，原目标仍blocked。

## 用户再次授权完整验收（历史单次失败已归档）
Status: complete
该次完整验收已执行并失败；fmt/check/strict Clippy、workspace761 tests/1 ignored、Agent/Hub build通过，live parity在Hub inline process.exec event suppression期望全零而实际low1/medium1时exit1。当时依单次授权停止，历史结果不改写；随后按最新五轮授权修复并在第2轮全通过。

## 用户授权：检查问题与五轮完整验收
Status: complete
inline fixture pending泄漏及Room错误schema遗漏已修复；最新完整第2轮全部exit0，新增完整失败1/5。历史3/3、追加3/3和独立一次失败保留。阶段三源代码已提交c6f3b17，阶段四文档/最终证据提交后完成原目标；TUI实际证据及delta/tunnel验证限制保持准确。
