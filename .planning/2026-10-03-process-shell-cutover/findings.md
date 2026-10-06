# 发现
初始工作树干净。LSP未配置，采用CodeGraph定位与引用。三名只读scout定位核心所有者/所有入口/配置消费者。
初始实现：exec::preflight为程序名+参数路径检查，cwd canonicalize writeRoots；build_command统一bwrap或direct。managed::cancel_command_process原为child.kill、local_process_kill_completed；已由核心owner替换为进程组取消。
初始Skill使用ProcessExecRequest；已拆分内部argv spec，Skill与固定内部程序保持直接argv启动，不转换为Shell。
Context7本轮两次fetch failed；后续parser源码/API使用官方可读GitHub文档，不能假称Context7成功。
旧argv执行DTO也嵌入Agent transport-runs.jsonl及Hub command_json。新DTO严格拒绝旧字段会令历史scan/replay查询失败；已分派PersistedArgvCutover沿既有retired读取机制保留原始历史身份/哈希/结果，退休未完成旧请求，不转换或重放成Shell。该路径只处理持久旧记录，不接受live旧输入。
target/contract-venv/bin/python已存在且实际import yaml/jsonschema成功；完整验收直接使用此环境，无全局安装。Agent/Hub二进制已存在，需完成实现后重建才能验收，不能用旧二进制证明新行为。

## 独立静态审查（不是运行证据）
ReviewShellPolicyInput发现：quoted string AST namedchildren漏尾部字面$和真实换行；tree-sitter-bash将反斜杠CRLF错误视作续行；leading escaped whitespace作为extras丢弃。三者造成argv失真/漏deny或partialAllow，已交ShellPolicy修正与回归。另stdio_schema required仍program/旧description漏迁，已交PublicProcessInput修正。
ReviewShellConfigRecovery发现：Config import known列表漏shell，坏shell会污染其他有效字段的恢复；已由ShellConfiguration补known及import三态/坏field隔离回归。stored旧argv matcher不应接受缺args坏record，空elements+旧workingDirectory应可退休，已交PersistedArgvCutover。新增recovery测试placeholder receipt hash断言不是持久identity证据，删除它并保留stored hash/raw/result；压缩test需要真正多版本记录而非期望no-op生成backup。
这些发现都尚未通过编译/运行；没有将静态预测当作实际失败输出。用户真实HOME init不能进入Rust fixture：各process测试helper显式Disabled，专门init测试用私有路径。

ReviewShellLifecycle 保留7项静态问题（全部 [INFERENCE]，不是已执行失败）：函数内 source 丢失普通 declare/typeset 变量；无斜杠 init 路径被 source 搜 PATH；R 控制标记写失败仍 eval；eval 未用 -- 截止选项；leader 终态后无条件2秒 reader timeout 截断合法后台输出；保留 live group 不计 active 容量导致 runtime 无界；旧 terminal leader 取消成功后 prune 令 capture wait 返回 not_found。已交 CoreExecution 单所有者修正并保留独立行为回归；真实 smoke owner补相应路径。另核对 init 后台子进程继承FD3时 startup reader 收到R能立即返回而非等待EOF。
Reviewer 已撤回 zombie-only 成功证据 finding：当前代码保守报告 termination_unverified，未拿发出信号或EOF冒充组终止证明。没有据此新增 reaper/cgroup。

## 实际验收结论
- 已完成普通非零状态、bootstrap参数界限、FD3隔离、初始化作用域/cwd、后台持续捕获、live group容量与retention、取消capture pin等修正。真实Agent/Hub Shell Smoke全场景exit0；实际配置TUI三态surface与save exit0。
- 进程组数字ID由已有processes-map互斥锁拥有；所有观察/信号重新进入owner，observed-absent即永久撤销slot；EOF后仍观察quiet group消失，避免后续调用信号到复用的数字ID。
- 容量拒绝等synthetic terminal ProcessInfo不一定有internal event source。response feedback只引用EventStore实际注册source，数据库错误传播而非empty fallback；真实active-limit拒绝后cancel及capture EOF已通过。
- Supervisor socket由HOME而非config root决定：`$HOME/.agentic_gpt/runtime/agent/<agent_id>/mcp.sock`。fixture只缩短唯一agent ID至精确100byte路径预算内；生产上限/路径保持不变。四个supervised smoke共享配置原无allow，须显式授权fixture自己的`/usr/bin/printf`，不扩大生产默认权限。
- 第12轮workspace tests全部通过：Agent615（1 ignored）、Hub138、Protocol23、配置CLI24、Unix control1、HTTP MCP4、Supervisor6、apply-patch2、browser-host10。最终完整验收结果见progress.md；既有ignored项未冒充执行成功。
- Live parity恢复夹具需区分relay观测、Hub持久化与parent线程时序。失败response同一lock内记录稳定boundary并arm双holds；EventSources另等待现有Hub DB sources持久化后保持精确identity比较。真实隔离after场景故意delay parent4s，replay已在parent醒前发生（count1→2），全部原barrier/结果/outbox/去重断言仍通过；未改生产恢复行为或增加超时上限。
- 阶段4只修agent.info::live_subset：新增整个config.shell投影，外层ShellConfig序列化保留Default省略与Disabled null、Path字符串（含显式默认路径文本）的差别。apply_live_config_subset已经copy shell，无需改热加载。用户明确不用永久回归测试，父统一构建后通过真实local MCP临时smoke验收。
- 阶段4真实smoke已exit0：七次Default/Disabled/Path A/Path B/显式默认路径文本转换均先false并含config_live_subset_not_applied，真实watcher加载后true且该issue消失；MCP连接关闭，临时Agent PID136521正常exit0已wait，临时root不存在。该验证只证明配置值的观测，不检查init文件内容或更改执行语义。
