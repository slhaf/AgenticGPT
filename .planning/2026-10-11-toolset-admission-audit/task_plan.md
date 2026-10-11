# Toolset 准入门控核查

## 目标与范围
核查用户引用对话中的问题是否仍存在于当前项目，并给出实际调用链、既有例外及最小修复范围。前期为调查；用户确认修复范围后，现进入实现与验证。

## 阶段
### 阶段 1：核对上下文与共享准入
- Status: complete
- 读取引用对话及项目开发、文档约束。
- 当前源码确认 Hub authorize 分支提前放行普通工具集。
- 阶段结束提交调查记录。

### 阶段 2：追踪消费者与特殊限制
- Status: complete
- 核查 Hub、本地 MCP、Unix、HTTP 的实际调用链。
- 核查 Skills 内部 Process、Room profile、基础 Event 例外及动态配置语义。
- 汇总可复核证据，阶段结束提交。

## 验证方式
前期静态核查已完成；实现阶段采用有保留价值的第④⑤档调用链回归，在独立临时 HOME/XDG/TMPDIR 中运行定点测试与适当 Cargo 验证。先确认回归在旧代码失败，再应用最小修复。

## 错误记录
- 初次 rg 查询包含不存在的 docs glob，退出码 2；改为按明确文件范围查询。

## 追加核查
### 阶段 3：Profile 的其他用途
- Status: complete
- 追踪 Normal/Room profile 的直接及派生消费者，区分准入、执行策略、运行身份、事件与配置行为。
- 仅静态核查，不修改实现；阶段结束提交。

### 阶段 4：统一共享 Toolset 门控
- Status: complete
- 普通操作统一解析 namespace、检查 live toolsets，再执行既有 Hub Room 与 Skills capability 条件。
- 保留基础 Event、CLI、通知及入口专属例外；不改变 Profile 定义、执行策略或底层 Process。
- 添加经长期价值评估的链路回归，先验证旧代码可捕捉该缺陷，更新实际受影响文档。
- 阶段结束提交实现、回归与文档。

### 阶段 5：验证与审查
- Status: complete
- 运行格式、相关 crate/工作区验证，审查差异，汇总兼容性与验证限度。
- 阶段结束提交验证记录。

## 完成结果
修复已在 eb40eb6 完成：普通 Hub/Local 操作共享实时 Toolset 门控，保留既有 Event、CLI、入口和能力例外。格式、工作区编译、严格 Clippy、Agent bin 与其余 Rust 测试均通过，独立审查未发现可行动问题；具体验证范围见 findings.md。
