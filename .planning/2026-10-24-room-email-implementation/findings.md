# 已确认的实现与验证边界

- 使用用户的 `/home/dev/.local/bin/remote-build` 远程编译与测试；不使用同名仓库脚本。配置、实现、文档按三个阶段提交。
- 图片采用 litehtml 0.2.6 的 vendored/email/pixbuf；独立有资源限制的子进程，不使用 Browser/Chrome。保留库完整默认HTML样式，确保div/p/table及分页高度正确。
- `view=text/image/imageWithRemote`，默认text；只有最后一种允许受限公网图片下载。准备HTML缓存供后续分片复用，禁止每页重复下载；内网地址、文件URL、脚本及外部CSS不开放。
- Local和Hub运行时本来均经公共 `operation_result::attach_event_panel` 附加events，未另造邮件封套；补齐OpenAPI响应中的同一字段。实际注入事件验证六API、图片和错误响应中的保留。
- 新邮件提醒写入原EventStore。邮件数据库保存待投递记录，EventStore持久投递回执避免重启/事件历史清理后重复提醒；事件满载不阻塞邮件索引。
- 独立进程锁保护同一邮件数据库；关闭邮件不夺取其他实例所有权，网络任务继续持有锁直至实际结束。首次分界持久化、pending恢复unknown、过期内容保留ID、明确恢复UID关联均由真实存储测试覆盖。
- 附件导出复用文件解码和写入能力；workspace目录句柄固定位置、禁止符号链接逃逸、原子不覆盖及600权限创建均在实际效果边界执行。已存在目标根据真实OS错误返回file_already_exists。
- 测试保留价值：真实TLS收发/确认、恢复与提醒投递保护数据完整性，文件/renderer/Local MCP保护消费者实际可见行为；未新增默认值或工具清单等固定内容测试。按用户要求删除原固定工具集合测试。
- 远程helper原有fd9被旧tmux继承导致锁等待；只处理本次等待者，保留原tmux，并让编译/测试子进程关闭fd9。隔离目录改为短/tmp/aget防止Unix socket过长，清理只读测试目录；增加lint/parity模式。
- 最终证据：check/build/lint/test成功，工作区542通过、1个原有手动用例跳过；现有真实HTTP/MCP parity通过。未使用真实邮箱服务商凭据，未验证生产隧道传输。
