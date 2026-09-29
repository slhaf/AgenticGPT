# 进度

- 自主目标已确认；原工作树干净。已读取中文标准，清点现行、归档与版本化文档，确定六个不交叠的写作切片和统一归档路径。
- 六个写作切片已完成：根 README/Console/测试贡献，配置/开发，Standalone/Browser，接口/工具矩阵/运维/Process 切换，架构现状/规则/目标，决策/路线图/归档。配置、工具数量、发布标签、进程等待、argv 密钥、CLI tmux、WP-T 和文件路径等已按源码核对修订。
- 诊断和两份旧设计移至 `docs/archive/`，删除已废弃的 2026-08-04 实施计划。历史 `.planning/` 审查记录仅含原路径的非链接性事实陈述，无失效可点击链接，予以保留。
- 集成发现浏览器部署外部核实措辞缺少本轮复核证据，已将双语页明确标为既有部署记录和本轮未复核；修正 README 重复 NOTICE 链接、中文版凭据风险措辞和架构导航旧路径。额外建立历史归档索引，翻译仍残留的英文自然语言标题。
- 验证：177 份已跟踪/新增 Markdown 中的 176 条仓内行内链接和 18 个目标锚点均存在；引用式及 HTML 本地链接为 0 条。`git diff --check` 通过；`cargo build -p agentic-gpt -p agentic-gpt-hub` 通过；实际执行 `target/debug/agentic-gpt config init --language zh-CN --help` 与 `target/debug/agentic-gpt-hub agent add --help`，观察到中文配置入口和 secret argv 警示/必填参数。系统 `python3 scripts/check_contract_parity.py` 因缺少 `referencing` 未进入契约检查；改用已具备依赖的 `target/contract-venv/bin/python scripts/check_contract_parity.py` 通过，覆盖 Agent/Hub 启动、MCP、HTTP、Room、Process 和跨端契约；生产隧道、外部 OAuth 和真实云部署不在本轮证明范围。
