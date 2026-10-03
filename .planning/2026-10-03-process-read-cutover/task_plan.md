# Process 统一读取重构

## 目标与边界
彻底替换相关 process.status/output/result 接口为 process.read，统一首次响应与后续观察，迁移所有真实消费者与契约；其余功能不动。底层存储不必变则不改。默认等待 5 秒，上限 30 秒；默认序列化响应预算 8192 bytes，可配置并接入 config TUI。

## 阶段
1. 契约与边界（complete）：已在 contract.md 固定 DTO、预算和等待规则；实现期间的局部澄清由集成所有者记录。
2. 跨接口实现（complete）：已完成实现、相关回归/契约脚本、当前文档及双语事实同步；完整验收仍由阶段3决定，不能把实现落地视为验收通过。
3. 验收交付（blocked）：续轮5/5（累计10轮）已用尽；最终fmt/check/Clippy/build通过，workspace test有1项supervisor失败，live parity停在Hub Full descriptor检查。TUI编辑/保存/预算生效已实际验证；详细通过范围及未完成项见progress.md末尾。

## 验收
- fmt check、workspace check、严格 Clippy、workspace test、构建 Agent/Hub、live contract parity。
- 隔离 Agent/Hub 验证短/长任务、纯状态与增量等待、游标重放/缺口、MCP 预算/不可恢复、取消证据。
- config TUI 实际编辑、校验、保存、生效。若修改 Console，运行受影响平台验证。
- 接口及调用方无旧路径别名；相关双语文档、配置与工具说明同步。

## 迭代与暂停
用户已明确授权追加五轮完整验收—修复—再验收，记为续轮1–5（累计6–10）；现已5/5全部执行。原五轮及新增五轮失败证据保留。依约停止，不开第六轮，不宣称完成；需新的用户授权后才能继续修复与验收。

## 错误记录
- bash 无法直接打开 skill:// 脚本；通过 glob 找到安装路径后成功初始化。
- LSP status：项目无配置语言服务器，使用 CodeGraph 与文本/结构工具。
