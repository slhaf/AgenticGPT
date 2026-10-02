# 已确定的实现合同

## 公共入口
- 模型 Process 工具仅 process.exec / process.batch / process.read / process.list / process.cancel。
- HubCommand 用 ProcessRead 替换 ProcessStatus/ProcessOutput/ProcessResult，wire 名 process.read。
- HTTP 新读取路径 GET /v1/process/{processId}/read；移除旧 GET /v1/process/{processId} 元数据读取及 /output、/result。list 与 cancel/exec/batch 保持用途。
- Hub cache-only hub.process.status 不可伪装为实时 read；独立缓存查询命名与消费者由 Hub 负责人核实后提出，须明确只读缓存、新鲜度，不新增执行权威。

## 请求
ProcessReadRequest: process_id:String, wait_seconds:Option<u64>, view:ProcessReadView(default Auto; JSON auto|status), cursor:Option<String>, max_bytes:Option<usize>。wait 默认5/max30/0立即，超30按30。maxBytes 省略用当前配置；显式范围4096..1048576，非法拒绝。status view 不返回产物，cursor 与 status 不可组合；MCP 不接受输出 cursor。
配置 key limits.processResponseBytes，Rust process_response_bytes:usize，默认8192，范围4096..1048576。单一常量来源置 protocol process.rs；config 引用。响应预算不是存储保留预算。MCP现有512KiB保留及输出ring不变。

## 统一响应
公开 ProcessResponse 紧凑 JSON：processId, kind, state, captureStatus；可选 group/batchId/batchIndex、exitCode、waitElapsedMs、error、cancelOutcome、terminationEvidence、captureError、output、mcpResult。不返回重复 status/completedInline/pollAfterMs/inlineOutput/outputPreview/resultAvailable。避免重复命令参数/路径/全部时间戳。
ProcessOutputPage: stdout/stderr 使用现有 data/encoding/startOffset/endOffset/gap segment；nextCursor/hasMore/eof；captureStatus 以外层为准。第一页和续读同形态。
ProcessMcpResult: status enum pending|included|deferred|unavailable|not_retained；可选 bytes, sha256, value, preview。included 有完整 value；deferred 为仍可取但本次预算不足；not_retained 不可恢复。pending 不是失败。完整下游 CallToolResult 保留原 JSON 结构。
ProcessBatchResponse: batchId,status(批次聚合状态),processes:[ProcessResponse]；删除 completedInline/pollAfterMs。批次预算为整个 ProcessBatchResponse，不是每子项各享一份。
保留 ProcessInfo/ProcessDetail 持久化结构。内部完整 process snapshot 不应被紧凑公开响应取代；Core 负责人可以用 agent-local ManagedProcessResponse { response:ProcessResponse, process:ProcessInfo } 和对应批次包装维持 operation_result 的 Hub 快照收集，不引入公开兼容类型。

## 等待与读取
exec/batch 首次保持短等终态语义，返回时使用统一观察；read auto 有 backlog 立即返回，否则有界等待内容、终态/采集结算或期限；终态但仍 capturing 且无内容可等采集结束。status 只等执行终态/期限，不因输出提前返回。捕获 incomplete 不无限等 eof；显式 gap 不得掩盖。游标只推进实际返回数据，因预算删减必须同步游标，重试/多消费者独立。
MCP 终态带结果即按预算返回，完整对象不可切碎；deferred 可提高预算。

## 预算与传输
预算计算序列化 ProcessResponse/ProcessBatchResponse 的 UTF-8 JSON bytes（含对应输出/结果元数据、JSON转义/Base64）；与传输封套及独立 event panel 的各自预算分开。不得 stdout/stderr/preview 各自占一份。先保留身份/状态/退出错误/取消证据，再给产物；错误代码不可丢，错误说明可明确截断。不为预算改变真实生命周期或取消证据。批次若必要状态装不下需在执行前拒绝，不能执行后隐藏孩子。

## 文件所有权与验证
父代理集成、计划、验证/提交；实现子任务不得中途 build/lint/test/formatter。对外符号变动先用LSP（当前已查无配置），依赖定位用CodeGraph。后续实现细节变化须在此同步并通知全部相关负责人。

## 独立设计复核补充
- 批次准入固定本次预算，首次响应沿用；不得在等待期间重新读到更小配置。预检使用最终DTO必要身份/状态/exitCode/错误代码/取消结果与终止证据的受控最坏序列化上界，不能沿用旧版“删掉证据后每孩子预留64bytes”。自由文本说明可明确截断。先为所有孩子保留证据，再分配正文。
- 输出数据、偏移、captureStatus/captureError 与 eof 来自同一组ring观察；内部ProcessInfo快照与公开响应同次观察。遵守 processes→stdout→stderr 锁序，休眠不持锁。MCP状态与结果同次detail观察。
- 待报告gap即有可返回内容；incomplete已结算，不等待不可能出现的eof。每次请求只有一个deadline，exec/batch短等后生成首观察不得再加一段默认等待。
