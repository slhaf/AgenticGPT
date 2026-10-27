# Room 子能力工具集与配置目录

## 已确认目标
Room 保留 API 归属与活动 Room 路由，子能力独立启停。子工具集是唯一启停依据，不保留任何针对旧行为的正向或负向逻辑。工具发现、调用准入、后台资源、TUI 使用同一 toolsets 真值。关闭仍可预配置。取消 Other configuration，按真实领域组织。

## 阶段与提交
1. [complete] 工具集/配置合同、发现准入、资源生命周期及其TUI消费者一起切换；Laptop完整测试和严格检查后提交。
2. [complete] 文档与当前格式说明、跨入口合同及真实终端冒烟验证；提交收尾。

核心类型与TUI消费者存在编译依赖，合并为同一实施阶段，保证阶段提交可构建。

## 消费者/语义
- 子工具集：room.bootstrap / diary / notebook / state / maintenance / email / secret；无父级授权开关。
- Room预设保留核心五项，Email/Secret按需开启，避免默认无凭据也激活服务；Normal默认不启用Room子项。
- Bootstrap读取workspace/bootstrap，无需Room仓库；Diary/Notebook/State/Maintenance需要仓库，Email/Secret为独立私有服务。
- 子工具集控制工具发现、调用准入和服务运行，独立账户/连接设置保留。活跃Room选择与API名称不变。
- 服务关闭：阻止新请求，按既有owner规则排空在途操作并释放资源；再开启可恢复，无旧副作用重放。

## 测试保留价值
迁移既有契约/权限/生命周期测试，不机械锁定新枚举列表。新增仅IV/V：独立工具发现与调用拒绝；关闭服务及重启后资源/所有者恢复；关闭时预配置与启用完整性、TUI/Review单一真值。这些保护长期的能力隔离、私密资源与配置数据完整性。

## 执行边界
按文件分工，并行agent不提交。所有Rust构建/测试/二进制smoke通过用户已授权的Laptop流程，使用独立临时HOME/XDG/TMP/D-Bus并等待子进程后清理。不修改remote-build。

## 用户进一步约束
重构不保留旧行为的正向或负向逻辑，包括专项识别、拒绝、错误码及相关测试。只保留新类型和当前业务规则的正常校验。此要求已记录为所有项目长期偏好。
