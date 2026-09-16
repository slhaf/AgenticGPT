# 本轮核验记录

## 已执行

1. `git status --short && git diff --stat`：任务开始时工作区干净。
2. `cargo metadata --no-deps --format-version 1 --offline`：成功，确认五 crate 及本地依赖；未编译。
3. 对 `git ls-files` 中 `.rs/.kt/.kts/.js/.ts/.py/.sh` 统计：148 文件、91041 物理行，包含测试和构建/脚本；Rust crate 合计 84697，Console 合计 2868。不是生产 LOC。
4. 通过 Python `yaml.safe_load` 读取 `openapi/hub.yaml`：成功。确认 JobInfo.required 包含 startedAt；JobListResponse 仅允许 jobs、items 引用 JobInfo；NotebookSelectExactRequest 要求 year/month/day、additionalProperties=false。
5. 对照源码 `protocol/lib.rs::JobInfo` 的 started_at Option + skip_serializing_if、`JobListResponse` 的 JobListItem/next_cursor、`JobCancelResponse`，确认静态合同不一致。未连接外部 Actions importer，不声称复现其具体错误消息。
6. 直接核验 `local_service.rs:132-141,275-281` 的旧 Room 拒绝，以及 Hub 调查中的 Full tools/list 注册与 dispatch 链。结论是源码可达链不一致，未启动真实 Hub↔Agent 场景。
7. 直接核验 `agents.rs:327-340`：store_result 的成功返回值不参与 pending.remove 判断。记录为响应所有权完整性风险，未做利用或竞态复现。
8. 直接核验 browser host `handle_client`/`prepare_socket`：socket 0660，接收路径未见 peer UID 检查；实际可访问性还受父目录、组、umask 与容器挂载影响，不宣称任意本地用户一定可访问。

## 尚未执行且不作通过声明

未运行 cargo/Gradle 构建或测试、真实 Android/浏览器/tunnel/Hub/Agent 部署、WS 竞态、OAuth 重启、ARM 发布或网络服务。本轮无生产行为修改，验证目标是架构证据和文档一致性，不是证明整个仓库健康。

## 后续本轮交付检查

正式文档完成后执行本地路径/链接核验、需求覆盖审阅和阶段提交；结果追加于此。
