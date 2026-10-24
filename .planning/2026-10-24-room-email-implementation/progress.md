# 进度
2026-10-24 已开始实施，三个独立代理负责配置、核心邮件服务和跨接口合同。主线程实现文件共享能力及受控HTML渲染，统一远程验证。
用户要求删除固定工具清单测试，已由接口代理执行。配置/TUI与六接口已落盘；核心与恢复测试正在完成。账户重启/其他实时更新与目录方案已落实。渲染选择是当前待用户决定项，故尚不进行此部分实现或宣称完成。
用户已确认view=text/image/imageWithRemote(default text)，不增加单模式专用字段。继续采用litehtml-rs独立CPU渲染，只有imageWithRemote允许安全下载远程图片。配置/接口、核心修复、渲染三个代理恢复并行；主代理实现远程图片下载、worker CLI、远程验证和提交。

核心与接口已实现；配置阶段提交 dbae739。远程check、build及严格lint通过；工作区邮件核心10项真实存储/TLS/确认测试及文件导出测试通过。图片worker保留litehtml完整HTML默认样式，修复分页高度；公开API测试正在实际执行验证。
用户再次明确邮件响应沿用event公共结构。运行时本就复用operation_result::attach_event_panel（Local/Hub统一返回路径），未新建邮件封套；补齐OpenAPI邮件响应遗漏的events声明，并用真实注入事件保护图片投影及错误响应。
remote-build修复fd9锁被子进程继承，测试HOME改短/tmp/aget以避免Unix socket路径长度限制；添加lint/parity模式，所有编译和执行测试保持远端。

实现阶段完成：远程实际Local MCP邮件组合测试通过，覆盖三种read模式、正文/搜索/图片cursor、远程缓存、本地地址阻断、附件原始写入与拒绝覆盖、六API成功/错误中的真实events保留。独立renderer进程测试通过。现有HTTP/MCP contract parity门禁PASS；无真实邮箱服务商凭据参与。工作区其余测试及最终文档检查收尾中。
