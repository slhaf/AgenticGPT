# Agent 私有状态扁平化迁移

本文适用于私有状态改为 `$HOME/.agentic_gpt/state/`、事件数据库 schema v4、进程数据库 schema v2 的版本。新版 Agent 只使用新布局，不自动读取或转换旧目录。需要旧记录时，先运行仓库中的[统一迁移脚本](../../scripts/migrate-agentic.py)。

## 迁移前检查

1. 停止使用本次 `HOME` 的所有 Agent 和 supervisor；迁移脚本不会停止服务。
2. 确认旧配置中的 `agentId` 对应要迁移的记录。脚本自动读取当前用户的 `~/.agentic_gpt/config.json`，无需传路径。
3. 确认 `$HOME/.agentic_gpt/state/` 尚无新版状态。只要存在 `events.sqlite3`、`process.sqlite3`、其 SQLite sidecar、`active-skills.json` 或 `skill-installs` 中任意一个对象，脚本就跳过，不覆盖或合并。
4. 使用 Linux 上的 Python 3；迁移进程数据库要求 Python 链接的 SQLite 版本不低于 3.35。脚本只使用 Python 标准库。

脚本根据配置的当前 `agentId` 选择旧目录 `state/agent/<key>/`。不超过 128 字节且只含 ASCII 字母、数字、点、下划线、连字符的 ID 直接用作 key；其他 ID 使用 `id-` 加 UTF-8 内容的 SHA-256。`.` 和 `..` 不能代表独立旧目录，脚本会拒绝。

脚本只迁移这个目录，不搜索或合并其他 agent key，也不迁移 workspace 下更早的状态目录。

## 执行步骤

统一脚本同时迁移命令／路径策略配置和本地私有状态，只接受 `--write`。先预演：

```bash
python3 scripts/migrate-agentic.py
```

预演会报告配置差异，在 `.agentic_gpt` 内临时复制、转换和验证所选旧状态，完成后清理临时副本，不替换配置或发布新状态。

它会创建迁移锁、配置 run 锁及配置写锁，并设置锁文件和 `.agentic_gpt` 目录的私有权限。

核对预演中的配置差异、假设和冲突，并确认状态验证成功后执行：

```bash
python3 scripts/migrate-agentic.py --write
```

脚本输出 JSON 报告，配置差异在 `changes`，状态迁移结果在 `stateMigration`。写入时先备份原配置，并保留旧状态目录用于恢复。新位置已有状态、当前身份没有旧目录或没有可迁移对象时，状态项报告跳过，配置迁移仍执行。

成功和跳过退出码均为 `0`，错误为非零。历史 jobs 数据与未知文件显示在 `notImported`，保留原处。配置转换规则见[确认与策略格式迁移](../operations.md#确认与策略格式迁移)。

## 保留内容与转换边界

| 旧对象 | 新对象和处理 |
| --- | --- |
| `events.sqlite3` | 复制为 `state/events.sqlite3`；schema v1 补齐三列远程 origin，v1/v2/v3 均移除 `event_store_metadata` 并写入 v4；事件、来源记录和索引保留。 |
| `process.sqlite3` | 复制为 `state/process.sqlite3`；schema v1 移除 `processes.agent_id` 并写入 v2；`info_json` 顶层和 `detail_json.process` 内的本地 `agentId` 移除；其他 JSON、输出、完成事件队列及索引保留。 |
| `active-skills.json` | 验证为有效 JSON 后复制到新布局。 |
| `skill-installs/` | 完整复制记录、journals、staging 及其他普通文件和目录，保留关联内容。 |
| `jobs.sqlite3` 及其 sidecar | 历史 jobs 资源不导入，也不改名为进程数据库；报告并保留原处。 |
| 其他旧对象 | 报告并保留原处，不导入。 |

迁移不会递归删除业务 JSON 中的 `agentId`，也不会修改 `$HOME/.agentic_gpt/transport-runs.jsonl` 等远程运行回执资源。

旧数据库及其 sidecar 先复制到临时目录，SQLite 只打开临时副本来制作快照，不对原库执行 checkpoint。原目录保留，迁移不删除旧记录。

脚本拒绝源、目标、配置及其路径中的符号链接，不跟随技能树中的符号链接；需要复制的对象只允许普通文件和目录。新目录权限为 `0700`，新文件为 `0600`。

数据库先通过 SQLite 完整性和外键检查，转换后再次检查。未知数据库版本、损坏的数据库或无效的本地进程 JSON 会导致失败，不发布目标。

## 并发、失败与回滚

脚本持有 `.agent-state-migration.lock`、标准配置的 `.config.lock` 和 `.run.lock`，阻止同配置 Agent 在迁移期间启动。活动的旧 Unix MCP socket 也会使迁移失败；这些检查不能代替迁移前停止所有 Agent 的要求。发布前再次检测新状态，避免覆盖迁移期间出现的新对象。

复制与转换都在目标文件系统内的临时目录进行，配置及状态全部验证完成后才逐个发布状态对象并原子替换配置。发布发生异常、可处理的中断或 SIGTERM 时，脚本逆序撤回本次已发布对象并清理临时目录；配置替换后的同步异常会尝试恢复原配置，配置备份仍保留。

系统断电或无法捕获的强制终止仍可能中断多对象发布；再次启动 Agent 前应检查新目录是否完整，不能把状态跳过报告当作完整迁移证明。

迁移成功后如需回滚，先停止新版 Agent，将本次发布的新对象移到另一个备份目录，恢复本次配置备份，再使用旧版 Agent 及保留的 `state/agent/<key>/`。新版运行后产生的新记录不会自动合并回旧库；不要删除这些新记录，也不要把多个旧 key 合并为一份状态。

## 迁移脚本验证

CI 在隔离 HOME 中运行 `python3 -m unittest discover -s scripts/tests -p test_migrate_agentic.py -v`。集成测试使用临时配置、实际文件及 SQLite 数据库，覆盖统一入口的预览、备份、状态转换、重复运行、运行锁、无效旧数据，以及状态发布／配置替换失败和中断后的撤回。测试不会迁移真实用户配置。
