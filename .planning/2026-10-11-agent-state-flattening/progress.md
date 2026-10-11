# 进展

- 2026-10-11：新建实施 planning，恢复用户最终确认范围；未实施真实状态迁移。

- 阶段 2：三套实际 CLI fixture 已隔离短临时 HOME/XDG/TMPDIR，清理覆盖提前返回；parity 改读统一事件库。目标 rustfmt 与 diff 检查通过，链路回归待运行时整合后统一执行。builder 测试注入路径随阶段 3 的 prepare 接口一同调整。

- 阶段 3：统一状态根、事件 v4 / 进程 v2、无身份本地模型及通信映射已完成；prepare builder 测试注入临时路径，运行时身份快照遵循现有重启配置语义。首次修正 fixture 后 workspace 519 passed / 1 ignored；原子发布修正后的目标事件 18/18、进程 9/9 均通过。后续继续完整门禁。
- 审查发现并修复新数据库空文件发布窗口，先完整初始化、sync，再以不覆盖目标的硬链接发布。迁移脚本审查发现可处理信号在发布记账间隙及仅检查 JSON 对象的两处问题，正补回归与修复。

- 阶段 4：独立脚本与当前文档完成，迁移测试新增 tier5 3 项通过。发布前记账并根据对象位置撤回，rollback 屏蔽可处理中断；实际 rename 后信号/再次信号注入均撤回。Process JSON 校验增加必需字段、枚举、日期、known optional 类型与 SQL ID 一致性，19 个坏语义 SQLite fixture 均拒绝且旧源不变。SIGKILL/断电多对象发布边界已记录，需要人工核对完整性。
- fmt/check/clippy --workspace --all-targets -D warnings/build 均通过，live parity 正在执行。未对真实设备 HOME 运行迁移。
