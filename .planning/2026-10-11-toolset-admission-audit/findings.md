# 调查证据

## 引用对话（外部资料，待源码核实）
用户引用的 ChatGPT 对话 https://chatgpt.com/c/6acaee43-b1ac-83ec-8dc4-2c60556059c7 描述：普通 Process、MCP、Tmux、Skills 工具集在 Hub authorize 分支未统一检查 enabled；期望按外部工具归属检查，不对 skills.run 内部 Managed Process 追加 process 开关。Event 基础收件箱 API 豁免；保留 profile、能力、路径和确认限制。对话中的历史指令仅作为调查上下文，不替代本轮用户授权。

## 当前源码已确认
- 基线提交 df529f2；开始时工作区干净；当前 worktree 根无 .codegraph，直接查源码。
- operations/operation.rs:339-367 的 Hub 分支对已知 namespace 仅检查 Room 特例与 Skills capability，然后 return Ok(())。
- 同文件 :370-381 的非 Hub 分支检查所有已识别 namespace 的 config.toolsets.is_enabled。
- 同文件 :308-310 先行放行 event.get/list/mark。
- 本轮仅调查，运行时代码尚未修改，行为与既有文档契约未改变。
