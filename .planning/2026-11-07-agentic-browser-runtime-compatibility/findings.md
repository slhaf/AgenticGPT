# 调查发现

- 项目 managed runtime 从官方签名仓库选择版本，只提取 cua_node 和 chrome；新版缺少 Codex 时直接读取配置会报错。
- 本次部署用系统服务环境变量提供同包 codex 程序，不能替代项目分发和启动修复。
- 需核对当前有效缓存、自动安装和包签名机制，避免项目升级后仍使用不完整的旧缓存。

- 用户明确要求最终行为不能要求下载 Codex。此前将完整 Codex 程序作为依赖的方案不再采用；需要沿用或实现原有无 Codex 的接法。
- 直接测试新版复用旧版 Full CDP 读取失败的降级，再忽略新增请求头读取：Chrome 可识别且可新建标签，但跳转和关闭仍因 requirements 读取失败。
- 新版 browser-service 临时修改为仅在 local-testing 模式下配置读取失败时返回空配置，并启用已有的 BROWSER_USE_DISABLE_AMBIENT_NETWORK，真实 Room 浏览器各项操作成功，stderr 无错误。其他模式保留原始报错行为。
- 稳定源当前提供 26.1103.32239，需要确认项目下载到的版本也可应用该修改。
