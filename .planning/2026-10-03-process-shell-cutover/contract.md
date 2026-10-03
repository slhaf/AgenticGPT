# 已确认合同

## 输入与运行
- 模型process.exec使用command必填/cwd可选；batch保留elements，各项command/cwd。保留其他控制字段及进程身份/状态/output/result协议，不保留旧输入别名。内部固定程序与Skill等仍可argv。
- 普通系统Bash，非登录非交互，pipefail不set-e，每次独立，stdin/PTY/持久会话不新增。Bash执行原文，cwd独立设置在init后，waitSeconds仅等待。

## shell.initFile
- 省略：默认~/.agentic_gpt/.bashrc；open ENOENT统一跳过，包括悬空symlink。
- 显式String：可打开读取才执行；任何错误含ENOENT返回Process失败，不继续command。
- null：禁用。默认其他访问错误或source非零同样失败。
- 允许跟随symlink；禁止新增symlink专门检测/拒绝。不得因链接自动放宽既有权限/挂载。
- Agentic内部显式加载；不作为模型参数；不自动生成文件或加载~/.bashrc；控制BASH_ENV等继承隐式初始化。init输出计入任务，加载后恢复请求cwd。沙箱按现有配置启用/禁用，不能静默降低或扩大挂载。
- init可信本地配置，不查其内容、不追踪PATH/函数/别名/hooks，不做执行对象绑定/同进程授权握手；接受同名函数替代允许程序。

## 白名单
现成parser（优先tree-sitter-bash）及受限literal argv提取，适配现有program/argsPrefix matcher。支持&&/||/;/|链，所有可识别调用先聚合：拒绝优先不执行任何前半段；需确认/未获允许/不支持/不完整解析进入既有整体确认；全部满足才执行原脚本。不额外允许Bash，不建立第二套策略/沙箱。确认不可用不放行。未知复杂语法不宣称逐命令deny硬保证。

## 生命周期
独立进程组及同组常规后代取消，处理确认等待/组终止升级/output EOF/终态证据；不保证主动setsid脱离，无cgroup。批次全元素预检/集中确认后才启动，不承诺已开始效果回滚。保留history/list/read/取消、bounded rings/gap/Base64/预算/MCP完整结果。

## 验证与交付
最多20轮全链：fmt/check/strict clippy/workspace test/build Agent+Hub/live parity。真实隔离Agent/Hub额外场景断言Shell结果/pipefail/链策略/init各种状态及symlink/cwd/init修改信任/进程组取消/捕获收敛/全入口。HOME/XDG/config/workspace/runtime/DB/TMUX私有临时根，不写真实用户数据。子代理不跑mid-flight检查，父代理统一集成后验收。必要docs/schema/tools消费者全同步；每阶段本地聚焦提交，不push/tag/release；超20轮、未约定公共行为/权限变化或真实数据操作暂停确认，不放宽标准。
