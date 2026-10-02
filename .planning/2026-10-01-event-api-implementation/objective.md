## Objective
在 AgenticGPT 完整实现已定稿的持久异步事件系统：Agent stdio/Unix/HTTP MCP、Hub MCP/API、外部本地注入、配置及配置 TUI。原响应是否包含终态，以最终入口返回为权威；Hub超时未返终态时，迟到Agent终态仍需事件。该目标替换旧Agent-only判据，验收计数保持0/3，不重置上限。

## Success criteria
- 每Agent独立持久inbox，所有客户端/入口/Hub共享事件、处理状态、展示次数；跨Agent隔离。Hub无单一目标或离线/超时错误/cache回退不附events，不伪造零计数或陈旧面板。
- 保持原业务结构，只追加events={current:"low: N | medium: N | high: N",new:[{"eventId | 摘要":"severity | 日期"}]}。current统计全部pending且未过期，包含隐藏事件；无事件全零和空new，不附冗长额外模块。
- 面板最多5条，摘要最多32Unicode字符（超出31+…），ID/等级/日期不计入摘要；high→medium→low，同级创建时间旧到新。
- low曝光1次后隐藏、默认24小时TTL可配置；medium3次后隐藏不过期；high一直候选到handled。只给实际new条目计次，同逻辑响应多种呈现不双计；隐藏不代表处理。handled/expired保留7天后清理，pending medium/high不自动清理。
- event.list(status?,severity?,limit?,cursor?)默认pending/20条，返回{items,nextCursor}，含已隐藏pending，条目含ID、32字摘要、等级、时间、状态。event.get({eventId})返回完整记录不自动处理。event.mark({eventIds})返回{handledIds,notFoundIds}，幂等，仅handled，不管理进程或安装。三个API也附events骨架。
- 完整event字段eventId/message/severity/createdAt/status(pending|handled|expired)/source(kind/ref)/shownCount/expiresAt。来源创建固定：内部process/skill_install及其实体ID由生产者填写；外部接入固定external且只允许调用者提供ref标签。模型不能改source，传输入口/Hub查询不改source。内部保存Agent归属和生产去重信息。
- 所有内部事件默认low，按稳定类型配置low/medium/high/off，off不创建。仅原初始响应未包含终态才通知；terminal state及终态说明已返回则抑制，completedInline=false/预算裁剪/TooLarge不等于异步。batch逐实体；后续status/get/list、deduplicated安装重试不得改原资格。
- process与skill_install真实终态生产，event API/过期/清理生命周期独立。协调完成早/晚于响应、并发、去重和重启恢复。业务owner同事务小outbox或同次JSONpending证据支撑补录，不被历史retention/cap丢失；7天只清事件正文，source/origin/emitted小tombstone防旧receipt复活。
- Agent本地以自身最终结果出口判定；Hub由最终入口原run Returned/NoTerminal作持久单次决定、可靠反馈匹配Agent/run/request/hash及source origin。Hubtimeout→迟到terminal→下一在线调用恰一event；正常返回terminal无completionevent；远程Awaiting不由Agent原value自行suppress。Hub/Agent重启和receipt replay恢复未决反馈，不重复、不误抑制。内部反馈不公开模型工具，不消耗panel曝光，不增加模型主动推送。
- 外部独立CLI接收stdin JSON，经existing受权Unix MCP注入，source.kind固定external；服务MCP stdin不混裸事件，不新增私有裸socket协议。
- typed Config、CLI registry、模板及live reload支持TTL/通知覆写，已准入操作使用snapshot。复用配置TUI选择low/medium/high/off并保存读回；仅配置UI，不实现事件查看/标记UI、不补齐未完成TUI控制。
- 保留Agent普通和Browser/file多模态业务/content/meta/error，Hub在线target/Room lease/错误投影保留同Agent events。更新protocol/wire/OpenAPI/工具描述/必要实际消费者/相关中英文对应文档，遵循planning每阶段一提交。
- 上述消费者可观察行为、边界、状态、权限、配置和恢复有有效回归测试及真实入口验证；下列命令全exit0，不能空跑或只helper/构建代替入口/TUI操作。

## Verification
- cargo fmt --all -- --check
- cargo test --workspace
- cargo build -p agentic-gpt -p agentic-gpt-hub
- python3 scripts/check_contract_parity.py：使用已有target/contract-venv PATH和已构建二进制，扩充Agent stdio/Unix/HTTP MCP、Hub MCP/HTTP真实事件检查，涵盖共享计数/list/get/mark、截断/cap/排序、TTL/source隔离、配置off/等级、inline抑制/async事件、Hubtimeout-late结果、Hub/Agent未决反馈重启、receipt不双计。全部一次性HOME/XDG/config/db/workspace，无真实凭据或状态。
- 隔离运行 target/debug/agentic-gpt config init --config TEMP --mode local --profile normal --language en，实际操作配置TUI通知覆写、保存、config show读回默认low及四选项，再验证真实生产配置生效。

## Boundaries
允许三核心crate agentic-gpt/agentic-gpt-hub/protocol、直接相关配置/schema/模板/测试/契约脚本/OpenAPI/docs/必要README及独立.planning；Console仅必要合同适配不新增UI。禁止修改apply-patch/browser-host/发布部署流程，不建主动用户推送、无必要的新总线或event驱动任务控制，不覆盖用户原改动，不提交秘密/真实私有状态/构建产物。按开发指南/文档标准/CodeGraph/可用LSP与既有组件实现；明确文件owner，主线程唯一集成/planning/提交/验证owner，worker禁止中途build/lint/tests/formatters。

## Stop conditions
最多3轮完整验收失败→修复→重验，计数不因目标澄清重置。达到上限未全过时停止并报告证据和未解决项，不宣称完成。定稿未覆盖且影响公开契约的实质歧义、破坏性用户数据迁移/删除、真实凭据/部署需求立即停报。常规选择沿既有模式保守处理，不因小问题停工；运行工具/环境不可用在尝试可行替代后明确报告阻塞。只有全部标准及真实证据满足才完成。


## 用户追加授权
- 用户明确选择修复事件多传输fixture启动和无目标MCP聚合发现吞曝光两项，并再允许最多3轮完整验收失败→修复→重验。历史3次失败保留，不重置；新增计数从0/3开始，达到新增上限仍停止报告。
- 上述全部成功标准、验证范围、安全边界及目标保持不变；CI补充严格执行cargo clippy --workspace --all-targets -- -D warnings，无warning，不通过压制warning绕过。

## 用户再次授权完整验收
- 测试注入隔离修复后，用户明确要求“授权完整验收”。执行一次fmt/check/strict Clippy/workspace test/Agent与Hub build/live parity完整链；不自动追加失败修复轮次。历史3/3及前次新增3/3保留，本次验收单独记录。
- 原全部成功标准及安全边界不变。既有真实隔离TUI保存/读回及同配置生产证据仍保留；相关配置/TUI代码未因测试hook修复改变。若本次失败，记录并停止，不自行重试；若全通过，完成原目标集成和各阶段提交。

## 用户授权检查问题并调整重试上限为五次
- 用户明确“检查问题，重试次数调整为5次”。继续查明/修复当前inline抑制gate并允许本次新增最多五轮完整验收失败→修复→重验；新计数0/5，历史3/3、前次新增3/3及独立一次失败全部保留，不改写历史。
- 全部原成功标准、CI/TUI/实际入口验证及安全边界保持。达到本次5次失败仍未全过则停止报告；成功后完成原目标源代码集成和文档阶段提交。
