# 确认文案字符校验调整

## 阶段 1：实现、验证并提交
Status: complete

移除 reason 非空白校验与 schema pattern，保留必填字符串及 1–100 Unicode 字符长度限制。同步当前文档，保留历史发布说明。运行远程既有测试与格式检查，不新增锁定简单校验实现的测试。完成后以一个 Conventional Commit 提交。

验证：远程 workspace 测试 567 通过、0 失败、1 忽略；remote-build lint（格式与严格 Clippy）通过；git diff --check 通过。未部署，未复测截图客户端。
