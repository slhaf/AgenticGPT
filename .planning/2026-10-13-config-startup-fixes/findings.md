# 当前事实与设计边界

- 默认 Tunnel secret setup 草稿含 ~；写文件时展开，读取 reference 时不展开。
- workspaceRoot 的 optional 草稿/手填 ~ 原样存入配置，ensure_workspace 不展开；policy.path/shell/room/tunnel override 已有消费者展开。
- 当前 Local 没有 Hub 连接，Standalone Hub 连接与 reporting 绑定；需补可选辅助连接，不允许辅助连接执行远程命令。
- Tmux 自动创建、Browser runtime resolver 与 Skills 恢复未检查 Toolset；共享历史/Event 协调仍需保留。
- 既有 .archify/ 是用户工作，不触碰。恢复旧 active_plan 指针，当前任务使用独立目录。
