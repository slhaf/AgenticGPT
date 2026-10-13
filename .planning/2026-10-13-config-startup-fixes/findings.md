# 当前事实与设计边界

- 默认 Tunnel secret setup 草稿含 ~；写文件时展开，读取 reference 时不展开。
- workspaceRoot 的 optional 草稿/手填 ~ 原样存入配置，ensure_workspace 不展开；policy.path/shell/room/tunnel override 已有消费者展开。
- 当前 Local 没有 Hub 连接，Standalone Hub 连接与 reporting 绑定；需补可选辅助连接，不允许辅助连接执行远程命令。
- Tmux 自动创建、Browser runtime resolver 与 Skills 恢复未检查 Toolset；共享历史/Event 协调仍需保留。
- 既有 .archify/ 是用户工作，不触碰。恢复旧 active_plan 指针，当前任务使用独立目录。

## 回归保留价值
- 生成路径回归：保护 setup draft -> 事务secret写入 -> JSON reference -> load/secret consumer/workspace副作用，发现引用与实际target失配及cwd下字面~目录；既有事务测试仅覆盖绝对路径，无相同模式覆盖。
- 既有 supervisor smoke 改用 home-relative key/workspace，在真实worker完成原有MCP操作时顺带保护旧配置~消费；不重复新增测试。该验证穿过独立supervisor resolver，与setup事务测试的消费入口不同。fixture关闭不使用的tmux并显式隔离D-Bus。
- Browser/Skills worker记录：现有测试无禁用时source/provision零副作用及持久queue defer/re-enable的覆盖，新增真实磁盘/状态链路回归；不开启低档机械断言测试。

- 资源审查发现并修复新安装发布Queued与恢复扫描间的重复spawn竞态；两入口共享claim_worker且在发布前认领，failure释放。复核无剩余并发缺陷。
- Browser runtime共享状态采用OnceCell；disabled startup不预置None，首次已准入Browser调用按live配置初始化，不持公共锁跨网络；浏览器消费者/审计读取已缓存runtime。
- Hub公开配置新增hub.enabled（默认false）；Local/Standalone辅助模式不接受执行消息，旧Standalone reporting开关兼容启用连接；reporting发送开关独立，确认-only仍有Hello/心跳。
- 新Hub传输回归保护受限辅助连接的真实确认收发、不报告运行事件、不执行收到的有效ProcessExec envelope。现有parity只验证ReportingOnly路由不能成为Room目标，无同一失败模式覆盖。
