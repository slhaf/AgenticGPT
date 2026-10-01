# 实现发现与合同

## 初始事实
- 初始git status --short和diff --stat均无输出。
- 存在.codegraph；已优先CodeGraph定位Config、AppState/build_app_state、finalize_process、skill install save_cache、配置TUI组件。
- 已读开发指南和文档标准，所有者沿既有Agent→protocol、Hub→protocol方向。
- process现有history与hook不原子；skill install独立JSON保存。原响应仲裁与崩溃恢复需新增持久协调，不把stdio日志tracker等同持久通知。
- TUI范围经用户二次澄清，仅配置通知等级覆写。

## 未决实现细节
- 核心store/响应判定/恢复接口与跨入口对齐方式，须复用真实调用路径。
- 配置TUI当前enum选择/分组/写入锁与config key registry，待核对。
- Hub原业务response/error/cache路径需共同处理事件骨架，不越过目标Agent授权。

## 阶段一映射结果
- LSP status：No language servers configured；使用CodeGraph调用路径与精确源码，不能运行不存在的LSP引用查询。
- 配置TUI只有init/import向导，无独立编辑器。可沿OptionalDrafts/Review Choice/既有choice组件增加覆写，最终复用配置写锁和备份。无需运行时事件UI。
- Unix现有local call --arguments-file -已有受限stdin→Unix MCP方式，可复用基础读取/传输，不引入新协议。
- Hub原响应data是业务Value，可保留形状加events；Room目标需捕获原lease，不跨Agent合并。
- Console无现有事件SDK/解码消费者，不因此新增UI。
- 已定义内部store/DTO/响应仲裁的候选共享接口 local://event-implementation-interfaces.md；未实现，等待下述合同边界确认。

## 需用户确认的公开合同歧义（按目标停止条件暂停实现）
- CodeGraph直接核对hub_info（Hub MCP :376、HTTP :187）与agent.list无单一目标Agent。
- process_list MCP :587-592，以及get_process_status HTTP :368-388，会在Agent离线/超时后返回Hub缓存业务结果；当前Hub无权威事件副本。
- “都覆盖”尚未定义无目标/多目标及离线时的events呈现；不能返回假零计数，也不能未经确认增加陈旧快照字段/新的全局收件箱。
- 询问无目标返回是否省略events，以及离线缓存结果是否省略events或改用明确标注陈旧的快照。

## 工具错误
- `sh skill://.../resolve-plan-dir.sh`未自动解析URI，exit127；已报告工具问题。随后coreutils realpath得到安装实际路径，使用该路径+PLAN_ID/PWF_PLAN_ROOT成功解析到本任务目录。
- 未运行构建、测试、格式化；验收修复轮次仍0/3。

## 合同歧义已清除
- 用户明确选择：Hub无目标Agent不附events；Agent离线/超时缓存或错误返回不附events。
- 因而不需要Hub镜像事件store或陈旧面板格式，不改现有离线业务语义。
- 共享接口与文件所有权已映射为核心DTO/store、配置TUI、Agent入口、Producer和Hub入口五个真正独立编辑切片。
