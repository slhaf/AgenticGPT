# Room Email 实现
## 目标
实现已确认的六个room.email API、后台IMAP同步、SMTP发送、私有持久存储及HTTP/MCP一致投影。保留账户单例和严格确认；所有编译测试通过用户的 /home/dev/.local/bin/remote-build。
## 已确认
- 首次已有INBOX导入不提醒；持久保存首次分界，中断后继续；正常重启对停机新增邮件提醒。
- INBOX成员及flags与服务器同步；已保存邮件可读可搜；sent/unknown记录可查。
- state/email.sqlite3保存索引进度发送状态，state/email/保存eml，cache/email/保存派生缓存；workspace附件导出由用户管理。
- 默认保存90天，容量2GiB，同步60秒，均可配置覆盖；账户连接配置重启，其他立即生效。
- outbox保证event满载不阻塞收信；未知发送不自动重试；身份恢复不静默重建基线或重复提醒。
## 阶段
1. 配置、TUI与实时生效边界：complete
2. 同步、存储、确认发送、附件、图片及全部接口：in_progress
3. 远程构建、修复、文档与实际链路验证：pending
每阶段完成后使用Conventional Commit提交；不推送。只添加有长期价值的④⑤测试；①②③不主动编写。
## 分工
主代理持有main/state/startup启动段、EventStore幂等注入、email_files/email_render、文档、验证与提交。配置代理持有config/TUI及reload函数；core持有room/email/**及Cargo依赖；interface持有protocol及其入口映射与OpenAPI。
