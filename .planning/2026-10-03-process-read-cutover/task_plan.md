# Process 统一读取重构

## 目标与边界
彻底替换相关 process.status/output/result 接口为 process.read，统一首次响应与后续观察，迁移所有真实消费者与契约；其余功能不动。底层存储不必变则不改。默认等待 5 秒，上限 30 秒；默认序列化响应预算 8192 bytes，可配置并接入 config TUI。

## 阶段
1. 契约与边界（complete）：已在 contract.md 固定 DTO、预算和等待规则；实现期间的局部澄清由集成所有者记录。
2. 跨接口实现（complete）：已完成实现、相关回归/契约脚本、当前文档及双语事实同步；完整验收仍由阶段3决定，不能把实现落地视为验收通过。
3. 验收交付（complete）：本次第2/5轮（累计12）完整Rust验证、构建、真实Agent/Hub全量parity全部通过；aggregate裁剪分类有旧二进制失败/新二进制通过证据，永久回归及live gate已保留；游标重放/缺口与TUI生效实际验证完成。审查发现已修复，剩余验证边界及历史诊断不确定性如实记录于progress.md。

## 验收
- fmt check、workspace check、严格 Clippy、workspace test、构建 Agent/Hub、live contract parity。
- 隔离 Agent/Hub 验证短/长任务、纯状态与增量等待、游标重放/缺口、MCP 预算/不可恢复、取消证据。
- config TUI 实际编辑、校验、保存、生效。若修改 Console，运行受影响平台验证。
- 接口及调用方无旧路径别名；相关双语文档、配置与工具说明同步。

## 迭代与暂停
用户再次授权五轮完整验收（累计11–15），在第2轮达到成功标准，不再运行剩余三轮。可排查失败属于待修复事项，不是技术阻塞；定点定位/修复不单独计完整验收轮。此前失败证据保留，未缩小目标或以忽略失败换取通过。

## 错误记录
- bash 无法直接打开 skill:// 脚本；通过 glob 找到安装路径后成功初始化。
- LSP status：项目无配置语言服务器，使用 CodeGraph 与文本/结构工具。
