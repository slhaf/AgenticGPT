# 已确认依据

- 搜索入口对相同参数反序列化、校验两次；单项搜索经批次 JSON 包装再拆解。
- 搜索内核把扫描计数转成 JSON，批次又解析并用零兜底；授权只需要路径，却构造完整读取请求。
- 搜索达到 maxResults 后仍构造必定丢弃的候选上下文。
- 编辑生成的完整 diff 与截断字段无生产消费者；审计仍需要 changedLines 与 revision。
- 编辑的 lock_paths 与 normalized_paths 完全相同；成功后重复写入每项状态。
- 编辑结果先变成 JSON，再由响应和审计层解析、默认补值，适合改为具体内部数据类型。
- commit_replace／commit_move 在 rename 前取走 temp；rename 失败后 cleanup_temps 无法发现残留文件。

## 测试保留价值

临时文件失败清理测试保护真实失败后的资源回收及数据完整性。已有移动删除失败和部分提交测试没有触发 rename 本身失败，不能发现提前丢失 temp 所有权。清理阶段复用现有测试。
