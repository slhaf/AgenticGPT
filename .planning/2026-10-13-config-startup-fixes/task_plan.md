# 配置与启动行为修复

## 授权与目标
用户已授权修复上一轮确认的全部问题，无需再次确认。保留当前模式的远程执行边界和既有显式 CLI 管理例外。每阶段验证后提交，不推送。

## 阶段
1. 统一初始化密钥引用与 workspaceRoot 路径展开：complete，主代理。
2. 补全 Standalone/Local 的可选 Hub 配置、确认连接和校验：in_progress，Hub worker（先设计，阶段1提交后实施）。
3. 按 Toolset 约束 Tmux/Browser 自动初始化、Skills 重启恢复：in_progress，资源 worker；启动层由主代理/Hub worker协调。
4. 整合、审查、文档同步和完整验证：pending。

## 验证与提交规则
使用独立 HOME/XDG/TMP/D-Bus，清理子进程；新增/改写测试前说明长期行为、回归信号和既有覆盖。由主代理选择暂存范围并逐阶段提交。
