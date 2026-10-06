# Process Shell 切换

## 目标
完整执行已设置goal与用户全部澄清；普通系统Bash，command/cwd，可信initFile（默认~/.agentic_gpt/.bashrc、null禁用、显式缺失报错、默认open ENOENT跳过含悬空symlink），白名单只分析提交command，不追踪init环境；进程组取消。无patched Shell/agentsh/新沙箱/PTY/cgroup。所有相关消费者一次迁移。详尽需求存contract.md。

## 阶段
1. 所有者与调用方定位、固定接口及共享契约（complete）；已提交e399179。
2. 实现执行/解析/配置/生命周期，迁移所有入口与消费者（complete）；已提交0fdf5da。
3. 真实入口验收、文档同步与完整验证（complete）；第13轮完整六项验收均exit0，额外真实Shell/TUI及恢复checkpoint场景通过，阶段修复/文档/证据完成并由最终本地聚焦提交收束。
4. 补齐agent.info的shell配置漂移观测（complete）；live_subset加入完整ShellConfig，无新增永久回归测试。格式检查、新Agent构建及临时HOME真实local MCP smoke均通过，七次三态/路径漂移逐次false→reload后true，临时Agent/连接/目录/driver均清理，修复/双语文档/证据由本地单阶段提交收束。
5. 对照项目指南审查Process工具描述（complete）；主代理亲读项目引用的OpenAI原文，核对Agent与Hub MCP描述/schema/annotations及真实工具表面。确认batch串行暗示与local必填elements遗漏，建议描述先说明目标与选择时机、再说明关键状态/风险。未改工具或API；仅清理单个pyc并完成隔离真实MCP审查，证据与建议按阶段记录和提交。
6. 实施Process工具定义调整（complete）；主代理直接完成、不调用子代理。两侧五个描述改为目标/选择时机优先，保留必要状态/风险；local batch elements必填标记与既有DTO对齐，执行行为/API名称不变。批次顺序只承诺逐项进程信息按输入顺序。字段约束与既有用户文档同步。用户补充要求描述改动不测试、不加测试、无API结构变化不smoke；新增corpus条目已原样撤回，不再运行测试或smoke。提前发出的验证链在中止请求前已完成，事实单独记录；本阶段单独提交。
7. 审查Event工具描述（complete）；主代理亲读local/Hub描述、schema、annotations及存储/分发行为。确认摘要/详情/处理选择流程、Hub默认/输出/上限缺失、cursor筛选绑定、expired的notFoundIds、不清理历史的过强说法及mark破坏性注解两侧不一致；仅报告建议，未改Event源码/API，不调用子代理、不新增或运行测试/真实smoke，审查记录单独提交。

## 验证
最多20轮完整验收；fmt/check/strict clippy/workspace test/build Agent+Hub/live parity均退出0；额外隔离真实入口覆盖shell/init/白名单/取消边界。并行实现子代理不得中途运行build/tests/formatter，由父代理收敛后统一运行。记录每轮结果。

## 边界与停止
保留无关改动；只改必要实现/配置/契约/消费者/测试/文档。按阶段提交、不push/release/tag，不碰真实HOME/配置/数据；未约定公共行为/权限变化或需真实数据等操作暂停确认，普通失败继续修复，达到20轮报告停止。

## 记录
初始git status干净；有.codegraph，代码探索先CodeGraph。LSP status：未配置语言服务器，采用CodeGraph/文本工具。规划脚本位于~/.agents/skills而非~/.omp/agent/skills，后者glob未命中。
