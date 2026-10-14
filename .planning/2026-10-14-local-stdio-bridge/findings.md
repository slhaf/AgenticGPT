# 发现

- 工作区初始干净；Rust 基线 1.99.0。
- local_control::connect 会完成 rmcp 握手，需提取 raw Unix 连接（超时与 server UID 校验）供两类消费者使用。
- LocalCommand 增加 Stdio，CLI handler 必须提前返回，绕过 pretty JSON stdout。
- local listener 每连接一个 AgentMcpServer，共享 AppState，最大 16 个同 UID 连接。
- cli_i18n COMMAND_CATALOG 必须增加 local.stdio 双语说明。
- tokio stdin blocking runtime drop 风险正在作独立设计核对。

- 采用进程生命周期专用 stdio 线程直接 io::copy，socket EOF 时完成通道使 CLI 返回，阻塞 stdin 不会成为 Tokio blocking task；使用 AsFd dup -> File，未修改标准流 fd 标志且不持文本缓冲。连接关闭显式 shutdown(Both)。
- public handler 提前返回，bridge只转发原字节；既有 client仍 connect_stream 后自行 rmcp initialize。
- 文档工作者已完成四文件，暂保留到阶段2提交。

- 真正交互测试发现 std::io::copy 的 Linux socket→pipe splice 路径造成stdout读阻塞；大帧fakepeer发送后关闭不会暴露该问题。手工小帧fakepeer复现：输入151bytes准确到达，stdout读与服务端socket读同时阻塞直到bridge kill。显式8KiB read/write_all替代后真实initialize与disconnect通过。故保留真实握手测试，不能仅依靠fakebyte测试。

- CI模拟：migration测试glob指向已缺失文件，初始baseline 300559a也无此文件；本机Python3.14.8返回5(NO TESTS RAN)，未修改无关CI。后续所有workflow检查含live parity通过，故不能宣称完整workflow全绿。
- 无公开HTTP/OpenAPI/MCP工具schema改变；仅公开local CLI新增bridge。CLI帮助双语、README双语、运行指南和接口说明已同步。
