# 配置与启动行为修复

## 授权与目标
用户已授权修复上一轮确认的全部问题，无需再次确认。保留当前模式的远程执行边界和既有显式 CLI 管理例外。每阶段验证后提交，不推送。

## 阶段
1. 统一初始化密钥引用与 workspaceRoot 路径展开：complete，主代理。
2. 补全辅助 Hub 连接及 Toolset 资源生命周期：complete，Hub/资源 workers。两部分共同修改 runtime state/startup，整合为一个可编译阶段提交。
3. 整合审查、真实 Supervisor 回归和完整验证：complete，主代理。

## 验证与提交规则
使用独立 HOME/XDG/TMP/D-Bus，清理子进程；新增/改写测试前说明长期行为、回归信号和既有覆盖。由主代理选择暂存范围并逐阶段提交。
