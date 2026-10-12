# confirmation 与命令/路径策略重构

## 目标
实现已确认的 acquireConfirmation、单源通配符策略、文件批准、Process临时范围、公开契约和手动迁移；MCP批准不变，不保留旧格式兼容。

## 当前阶段
阶段四：最终隔离验证和交付（进行中）。

## 代码阶段（每阶段独立提交）
1. 策略配置/匹配、初始化、确认请求类型与判定基础（完成）。
2. Process/tmux/file实际执行授权及sandbox范围、接口映射和执行回归（完成）。
3. 文档、公开schema、迁移脚本与消费者/既有测试同步（完成）。
4. 隔离验证、独立审查、修复与交付（进行中；Rust全套通过，live parity待结束）。

## 已确定语义
- acquire默认为false，paste例外true；method仅ntfy/freedesktop，省略配置fallback。
- 主动批准覆盖本请求命令及路径策略；拒绝/取消终止；超时/不可用回规则，Confirm复用失败不再询问；paste启用主动确认时任何未批准终止。
- Process可在acquireConfirmation提供read/write具体文件/目录，非空要求acquire=true；批准临时开放且不改全局，file请求自行确定范围且不接受read/write。
- command policy defaultAction=confirm，allow/confirm/deny字符串patterns，deny>confirm>allow。单独*全命令含不完整脚本；token *字符、尾部**剩余参数。
- pathPolicy.read/write分区各defaultAction=deny，路径*不跨/，**跨目录，单独*全路径；初始化写明规则，运行无builtin或隐式workspace grant。
- file.search批准指定目录递归范围+过滤条件，正常访问逐实际路径检查；edit全patch批准，身份/revision/资源限制保留。
- sandbox基础具体挂载独立于pathPolicy glob；临时read/write须正确覆盖实际已存在目录/文件；不静默扩大到父目录。
- 全面升级旧字段报错，独立手动迁移脚本预览/备份/差异，不自动执行真实配置迁移。

## 验证与测试保留价值
新增/改写前先评估保护的长期行为、回归信号及与具名既有测试的重复；只主动增加真实跨组件授权/副作用回归。测试外层和子进程使用隔离HOME/XDG/TMP/D-Bus，结束清理。
