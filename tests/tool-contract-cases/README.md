# 确定性工具契约用例

`cases.json` 是 Phase E 阶段使用的与特定模型服务商无关的用例集。每个用例记录一种真实的模型误用形态、公开工具及参数形式，以及预期的描述符或分发结果。仓库内的 Agent 测试会加载该文件，并实际执行描述符、serde、分发/试运行路径。这套确定性运行时用例与下方可选的预测形态探针相互独立。

`cases.json` 会作为静态 JSON 直接解析；测试框架不会展开 `$fixtureRevision` 等变量或占位符，公开的 `file.edit` 请求也不接受模型提供的 revision guard。不要把变量插值或该 guard 当作受支持的用例语法。若测试需要动态创建临时 fixture 或读取其 revision，应在测试框架中实现相应准备与断言，并仅在用例文件中填写公开请求支持的字段。用例应保持精简，不包含凭据、机器路径、网络 URL 或原始密钥。

添加回归用例：

1. 使用公开工具调用复现无效选择、参数或结果，并记录最小且安全的 JSON 形态。
2. 添加具有稳定 `id`、`kind` 和预期类型化代码/字段的用例。
3. 仅在确实需要新的准备或断言形态时扩展测试框架；优先复用现有描述符、serde 和分发断言。
4. 运行聚焦的契约测试以及对应的软件包/工作区检查门禁。

## 可选的预测形态探针

`scripts/evaluate_tool_contracts.py` 是与特定模型服务商无关的**预测形态探针**，不是运行时或 schema 门禁。它只读取预测结果；不会调用模型服务商、读取凭据、校验 JSON Schema 或分发工具。它的通配符（`$...`）以及对象子集/列表前缀匹配语义是有意设计的。`--strict` 表示预测缺失或不匹配时返回退出状态 1；它不会启用严格 JSON Schema 或运行时校验。探针输出会标记为 `probe: prediction-shape` 和 `runtimeValidation: not-performed`。

确定性 Agent 用例集与实时行为/schema parity 门禁分别提供相应保障。`scripts/check_contract_parity.py` 是受支持的跨 surface 门禁：默认使用 `target/debug/agentic-gpt` 与 `target/debug/agentic-gpt-hub`；也可用 `--agent-bin` 和 `--hub-bin` 指定明确的构建产物。它会校验受支持的 OpenAPI artifact，并在隔离的 loopback/private-home 环境中启动实时进程。Schema 校验与实时行为是独立检查；预测探针不提供其中任何一种保障。
