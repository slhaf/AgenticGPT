# Agentic 的 rmcp 补丁

来源：官方 crates.io 的 rmcp 3.10.0 发布包。
下载：https://static.crates.io/crates/rmcp/rmcp-3.10.0.crate
SHA-256：b5949e504d5ab9944a168259afc0c39ac0230627b6ec5367ace194c59bcbe163

相对发布包仅修改 src/model/capabilities.rs：为 ServerCapabilities 增加可选顶层 events 对象和对应 builder 字段；
调整已有 builder 泛型以保留 enable_tools/resources/prompts 等方法。缺省不输出 events，不使用 experimental。
订阅业务由 Agentic 实现，不在 SDK 补丁内实现。上游原始许可证和源码保留。
