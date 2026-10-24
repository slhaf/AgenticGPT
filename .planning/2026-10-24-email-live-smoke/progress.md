隔离根目录：/tmp/emlive.odj1s0gc
计划只保存路径和脱敏结果，不保存凭据。
当前源码HEAD已更新为88870b3，保留用户更新。首次remote-build release失败：cross交叉C++编译器缺少C++17的variant头；由独立构建代理处理，不改变邮件语义。计划脚本无执行权限已改用sh调用。
SMTP登录已成功，TLSv1.3。IMAP已登录，但预检脚本处理capabilities字符串类型出错，已修正脚本，仅重跑IMAP预检。
首次测试邮件已通过真实SMTP发送且在真实INBOX找到。为隔离HOME准备Noto中英文字体，只放本次临时目录，不改变系统字体。完整API smoke脚本已就绪，等待当前HEAD ARM64构建。
慢下载已取证（apt索引部分文件持续增长），代理7897实测未提速。改用Laptop现成C++17交叉工具链，经一次性REMOTE_BUILD_CONFIG向remote-build远端release注入cargo等价调用；正式远程构建已启动。交付后按ELF实际符号需求及本机执行验证，不由sysroot版本猜测运行结果。默认remote-build脚本和仓库业务代码未改。
remote-build release已完成：采用Laptop现成GCC16，以临时配置运行，release优化构建2分17秒，ARM64二进制取回当前设备。完整隔离API smoke已启动，输出/tmp/email-live-smoke.log。
完整真实API验证通过：首次导入无提醒；Unicode正文3页连续；本地/正文搜索；离线图片分页、远程PNG下载和缓存续页；读取不改已读；附件导出字节一致且不覆盖；4个状态操作在服务器复核；API自发自收只提醒一次；停机来信在重启后补收且旧提醒不重复；MOVE不支持时返回明确业务错误。
主代理实际查看PNG发现文字缺失，核对当前fontdb源码后给隔离HOME补fonts.conf，实际复验中文、英文、表格、内嵌和远程图片均正常。Context7未收录fontdb，没有使用不相关库文档替代。
额外的邮件数量检查用IMAP HEADER Subject子串搜索未匹配；改用本次本地记录的3个UID逐一从真实服务器确认，无重复发送。此检查方法不属于邮件模块的本地search实现。
最终12项检查通过。已停止并等待所有本次Agent/CLI进程，按隔离HOME检索确认无残留；删除整个临时HOME、凭据、本地原始邮件/数据库和下载附件。仅保留本次自建测试内容的PNG和脱敏结果于/tmp/email-live-smoke-20261024；服务器留下3封唯一标记的测试邮件，原有邮件未操作。仓库业务代码和remote-build脚本均未修改。
