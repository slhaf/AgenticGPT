# 实现进度

- 目标已创建，用户授权实现；最多3轮完整验收失败修复。
- 阶段一进行中：读取技能/开发指南/文档规范，初始工作区干净，CodeGraph已定位核心所有者。
- 尚未修改实现代码，尚未运行构建/测试/格式化。

## 阶段一调查与阻塞
- 三个只读scout完成Agent/Hub/配置TUI映射，主线程CodeGraph核对Hub无目标及离线cache分支。
- 共享接口候选已记录；尚无实现代码改动。
- 公开响应合同歧义触发用户批准的停止条件：无目标Hub调用及Agent离线如何呈现events。
- 待用户确认后继续；完整验收修复轮次0/3。

## 恢复实现
- 两项Hub响应例外已由用户确认；解除合同阻塞。
- 核心共享接口已钉住，准备并行实现真实切片；所有任务跳过中途build/test/formatters，由主线程统一验收。

## 并行实现启动
- EventCore/EventConfig/EventProducer/EventAgentIngress/EventHubIngress/EventParity六个独立owner已启动。
- 已发送config reload和producer原响应仲裁helper精确接口协调消息。
- 系统python依赖检查失败，现有隔离venv依赖检查通过；此为前置环境检查，不计完整验收修复轮次。

## 最终入口仲裁扩展
- 按停止条件问询并获得用户选择“最终入口返回”；Hubtimeout/迟到结果成为显式验收场景。
- 共享新接口已记录、发给Core/Producer/Agent/Hub/Parity；追加EventHubFeedback独立实现Hub决策/outbox模块，其他owner仅做各自接线。
- 目标文本更新保留全部旧合同/边界/最多3轮，完整验收修复轮次仍0/3。

## 全部实现暂停
- goal更新过程中drop后工具不可用，重建/读取的多条可用工具路由均失败，宿主问题已报告。
- 已向所有owner广播HOLD；Core/Config/Producer/AgentIngress/HubIngress/HubFeedback/Parity全部确认停止编辑和检查。
- 保存objective.md（五段目标）及pause_handoff.md（精确未完成状态），只提交planning记录，source不做完成提交。
- 当前无active goal可用；等待宿主恢复工具后继续，不重做已确认访谈。验收修复0/3。

## 用户恢复目标
- goal(create)成功返回active，五项授权/验收/范围/3轮/停止条件此前已确认，本次不重复访谈。
- 读取本任务四份记录与pause handoff、解析固定PLAN_ID目录，初始paused diff已记录。
- 共享接口复制到interfaces.md及response_feedback.md，成为可提交依据；现有local文件保持辅助，不是唯一规范。
- 准备解除全部外部工具阻塞，并通知原owner RESUME；验收修复0/3。

## 恢复执行接线
- goal(get)确认active；七个原owner收到RESUME（EventConfig由parked恢复），沿原文件所有权继续。
- 所有暂停todo已解除，core任务active；缺失origin helpers、ConfigValueKind旧variant和producer旧草案优先补齐。
- 本次无build/test/formatter执行；完整验收修复仍0/3。

## 静态风险复核与实测准备
- 将remote origin绑定提前到实体副作用前写入共享合同；原响应helper只提取metadata，防崩溃窗口误变local仲裁。
- 启动EventCoreReview只读复核已知DTO/store并直达Core反馈，禁止中途检查或编辑。
- 隔离venv具yaml/jsonschema，无pexpect/websocket/websockets；实际TUI用stdlib pty，parity复用stdlib WebSocket支持。

## 核心切片待审
- Core报告DTO/store实现和15类行为测试已就绪，含并发曝光、origin sticky、身份拒绝、tombstone/重开；尚未执行任何检查，不视为验收成功。
- 独立CoreReview继续检查其稳定源；新增ConfigReview只读核对typed配置/导入/optional TUI保存边界，修正由原owner负责。
- safe summary不变已明确在共享合同。一次edit因最新snapshot未显示锚点被拒绝，重读精确范围后成功；无source影响。

## Core审阅修正完成
- Core报告三项独立发现已修正：锁内migration版本读取与双opener回归、fallible duration/checked date与越界回归、remote_policy解除helper遮蔽。
- 持久store和紧凑panel实现todo已完成；API真实入口仍在集成中，所有新测试仍未执行。
- CLI私有注入ack不取panel，防low曝光被注入者吞掉；AgentIngress/Parity已确认真实下一公开调用回归。

## 配置切片实现完成
- Config报告typed字段/12个CLI键/strict import/稀疏load与show、配置Optional及Review/save/outcome已完成；review缺brace修正、超大TTL不panic校验已同步。
- 新增CLI拒绝不写盘、inherit恢复、Review四等级及导入备份保留的行为测试；尚未运行。
- 保存了owner给出的source-derived PTY按键顺序供实际核对，不能当作真实TUI证据。live reload/生产和入口仍待集成，验收修复0/3。

## 生产与反馈切片待集成审阅
- Producer交付managed process/MCP/batch/install准入origin、ProcessHistory小outbox及install JSON marker；独立ProducerReview检查所有者终态/恢复/原响应解析。
- 要求补齐有意义eligibility边界测试（terminal预算裁剪、mixedbatch、dedup及remote Awaiting），不以单纯outbox/marker测试替代完整规则。
- HubFeedback模块已交付，FeedbackReview检查durable decision/outbox/可靠ack/reader调度。HubIngress报告实际装配/prepare/metadata/finalize/flush已接入，仍未验收。
- Parity已覆盖各入口基础事件行为；正在补真实Agent的stdlib WebSocket延迟relay与双服务重启receipt恢复，未执行脚本。

## 独立生产/反馈审阅修正中
- ProducerReview七项完整发现已取得并分配：三个语法/字段编译问题、管道未EOF时终态证据延迟、install live marker重试、history损坏阻塞eventAPI、local panel失败前错误suppress。
- FeedbackReview四项分配：run/intent崩溃窗口、Agent pre-reply crash缺source发现、Returned持久失败仍返成功、普通公开调用不重试反馈。
- 新内部event.sources仅恢复exact origin的source身份；与原reply终态flags分开，避免活跃重连抢先写false。不伪造业务Response、不生成panel/模型推送。
- HTTP/OpenAPI未完成切片完整转给EventHubHttp；路径与Parity已钉住/v1/events GET、/v1/events/{eventId} GET、/v1/events/mark POST，不缩scope。
- 上述均为静态审阅修正，未执行验收，计数仍0/3。

## 阶段二source checkpoint
- Core/Config/Agent公开API与external Unix CLI、Producer审阅修正source已交付；阶段二source完成，跨入口/实际验收继续阶段三，不宣称功能已验收。
- 独立installed-Chrono throwaway probe实际输出+33715年期限，现有jsonschema date-time checker拒绝；Core/Config已改为RFC3339 0..9999并保留1e11大TTL，拒绝1e12。probe已清理；这是依赖边界repro，不是workspace验收轮次。
- Root特殊Browser/file结果同逻辑panel将增加紧凑text fallback，同时保留原content块/_meta/isError；待AgentIngress补齐真实image路径回归。
- Room HTTP room_value_response完整Json(value)保留error及events，无需重复修改。HTTP process list/status实际在线forward Agent（非cache-only），已纠正之前推断，绝不再取第二个panel。
- Source提交延后至完整ABI/入口集成；本阶段仅planning checkpoint提交，所有命令/TUI完整验收仍0/3。
