# 进度

用户已授权实现并要求每阶段提交。初始工作区干净。已阅读开发和文档规范。
阶段1：契约与文档由独立agent分文件处理；主agent迁移语义/调用/原有链路测试输入。旧agent线程followup失败（active writer），已新建独立任务。一次tmux补丁因调用格式已变化未应用，后续按当前内容重写。
阶段1验证：cargo check --workspace --offline通过（3m39s）；cargo fmt、git diff --check、Python parity脚本语法通过。文档并行编辑但留在阶段3提交。
阶段1已提交eea7703。阶段2 cargo check --workspace --all-targets通过；其后清除两个无用符号。扩展现有真实执行测试覆盖8组主动/自动/规则/解析兜底长命令，定点测试1 passed（其余filter无匹配未计为验证）。已准备真实Agent/Hub/假ntfy及隔离D-Bus/tmux验证脚本。一次提交因无Git身份失败，已仅本次命令使用仓库最近作者身份完成；未修改全局配置。临时Python依赖安装在/tmp。
真实隔离链路首轮：ntfy 5组主动/默认/规则/解析兜底长命令、非法参数、聚合批次、文件读取均通过；freedesktop markup支持与不支持两组、自动30字符预览通过。tmux在通知前失败，正在获取实际错误（前面已通过部分不重复）。Clippy发现旧notify测试probe调用遗漏，已仅迁移调用并保留原有能力探测断言。
tmux失败已定位为既有shell-pane保护（tmux_shell_paste_forbidden），发生在确认前，未有写入。将临时fixture从单一shell command改为直接argv启动cat并显式检查pane，避免把shell pane当作TUI。其余已通过的发送链路不重复。
阶段2验证完成：扩展真实执行测试1 passed覆盖8组；cargo build Agent/Hub通过；cargo clippy --workspace --all-targets --offline -- -D warnings通过。真实隔离发送路径：ntfy5组+参数错误+批次+文件、freedesktop markup两组+自动预览均通过；tmux改用direct argv的cat fixture后自动Confirm/30字符/审批前不写入通过。错误属临时fixture启动了shell，未修改shell-pane保护。完整workspace测试已启动。
