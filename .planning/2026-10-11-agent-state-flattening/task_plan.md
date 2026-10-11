# 本地状态统一与进程模型拆分

## 目标
本地权威状态直接位于 HOME/.agentic_gpt/state，不按 agentId 定义归属；Agent 本地进程模型与共享协议分开，Hub 身份、wire 与远程回执/事件行为保持。

## 已确认约束
- Agentic 启动仅初始化新布局，不探测/迁移旧路径，不回退 workspace/state。
- 独立迁移脚本运行时判断新版状态；没有新版数据才寻找当前 config.agentId 的旧目录，转换关联数据。保留旧源以便回滚，不合并其他身份，不操作本机真实状态。
- 本地进程模型去 agentId；共享协议模型保留，通信边界补当前连接身份。远程 origin/receipts 不去归属。
- 不引入生产 agentId、多 workspace 隔离或多实例协调系统；测试使用每 fixture 临时 HOME。
- smoke/新增测试的 runner 与所有子进程隔离 HOME/XDG，先终止等待进程再清理；不修改真实 ~/.agentic_gpt。
- 每阶段提交；任务完成前执行适用回归及 workspace / live parity 检查。

## 阶段
### 阶段 1：实施规划
- 记录最终范围、接受条件与模块分工。
- Status: complete
### 阶段 2：测试 HOME 隔离
- 修复 local_control、standalone_http_mcp、standalone_supervisor 与 builder 测试；更新 parity 状态路径。
- Status: complete
### 阶段 3：本地状态与模型去归属
- 简化状态根、事件 schema，拆开本地 process 模型与 wire，调整持久化与边界映射。
- Status: complete
### 阶段 4：独立迁移脚本与文档
- 实现仅选当前旧目录的显式迁移、SQLite/JSON转换、目标存在不覆盖、失败清理、旧源保留、关联数据完整性；同步当前文档。
- Status: complete
### 阶段 5：整合验证
- 实际迁移→新 Agent 读取、改变 agentId 状态续用、临时 HOME 互相独立、远程协议不变、重启恢复与清理。
- fmt/check/clippy/test/build 与 live contract parity；记录实际结果和限制。
- Status: complete

### 阶段 6：后续实际目录 smoke 核验
- 新临时 HOME 下冷启动、写进程与 Skill 状态，检查实际目录树、SQLite schema/完整性及权限，终止等待后清理。
- Status: complete

### 阶段 7：删除本轮新增测试
- 按用户要求删除迁移、状态目录及 SQLite 新增回归用例，移除既有测试中新增的状态验证断言。
- 保留既有用例的 HOME 隔离和接口适配；运行格式与全目标编译检查，独立提交。
- Status: complete

## 验收
- 新 runtime 不再创建 state/agent/<key>、无 owner_fingerprint 或 local process agentId。
- Hub payload、local API 的 wire 身份和身份校验保持原样；内部模型及落盘记录不携带本地归属身份。
- 迁移脚本幂等：新版数据存在即跳过；普通错误和可处理信号撤回已发布对象，源数据不变。SIGKILL/断电的多对象发布限制见迁移指南。
- 运行测试后真实 HOME 无新增，所有临时服务/资源及时清理。
