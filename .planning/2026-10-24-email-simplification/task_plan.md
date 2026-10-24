# 邮件实现收敛
目标：保持已确认邮件功能、unknown不自动重发与events响应结构，去掉明确重复，修复text重复解析，图片优先复用库资源发现并收敛缓存/锁。
阶段1 公共请求与MCP入口去重：complete
阶段2 按需读取、图片资源加载/缓存收敛、文档及完整验收：in_progress
每阶段独立Conventional Commit；主代理审最终diff并通过用户remote-build编译、测试、真实二进制调用与parity。
只添加或调整有长期保留价值的④⑤链路测试，保持临时HOME/子进程清理。
