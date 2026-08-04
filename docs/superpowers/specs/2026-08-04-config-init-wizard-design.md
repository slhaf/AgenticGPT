# AgenticGPT 配置初始化向导与可发现 CLI 设计

日期：2026-08-04  
状态：待用户复核

## 背景

当前 `agentic-gpt config init` 直接序列化一份通用 `Config::default_config()`。它能生成合法的基础 JSON，但没有根据实际运行模式展开相关配置，也不会引导用户填写 Standalone tunnel 或 Hub 连接所需信息。

同时，`agentic-gpt config --help` 只列出 `init`、`show`、`set`、`allow`、`confirm`、`deny`、`path` 与 `mcp` 等名称，命令、参数和子命令几乎没有说明。`config set <KEY> <VALUE>` 也不公开可用 key、类型和示例，用户往往必须阅读文档或源码才能发现配置能力。

## 目标

1. 让裸运行 `agentic-gpt config init` 在交互终端中启动现代、简洁的配置向导。
2. 默认选择 `standalone + normal`，同时支持 Hub、Local 与 Room profile。
3. 主流程只询问当前模式真正必要的信息；非必需项通过明确的可选入口按区块展开。
4. 为脚本和 CI 保留完整、确定、不会等待输入的非交互入口。
5. 补齐 `config set` 对安全标量与列表字段的覆盖，并让支持范围可由 CLI 自身发现。
6. 将 AgenticGPT CLI 的帮助文本、向导文本和新增配置错误统一提供简体中文与英文，并自动选择语言。
7. 保持命令名、参数名、枚举值和稳定错误码为英文，避免脚本与文档出现双重语法。

## 非目标

- 不把 CLI 做成全屏 TUI，不加入鼠标操作、页面路由或复杂布局。
- 不在首次初始化向导中配置 MCP server 集合；继续使用现有 `config mcp` 子命令。
- 不引入通用 JSONPath 修改器；`config set` 仍是受控 key registry。
- 不在本次工作中翻译 Hub 服务端或协议层全部运行时日志。
- 不改变现有配置文件的 canonical JSON 字段名。

## 术语

运行模式与能力 profile 分开建模：

- Mode：`standalone`、`hub`、`local`
- Profile：`normal`、`room`

`standalone` 是默认 mode，`normal` 是默认 profile。

当前 `Config` schema 中部分 Hub 字段是必需字段，因此所有模式仍保留 schema 所需的公共顶层字段。Mode 决定初始化时重点展示、要求填写和展开的配置块，不在本次工作中把现有 schema 改造成互斥枚举。

## 命令接口

### 全局语言

新增全局参数：

```bash
agentic-gpt --language auto|zh-CN|en ...
```

该参数应为 global argument，可放在子命令前后。默认值为 `auto`。

语言解析优先级：

1. 显式 `--language`
2. `LC_ALL`
3. `LC_MESSAGES`
4. `LANG`
5. 无法识别时使用英文

`zh`、`zh_CN`、`zh-CN` 等归一为 `zh-CN`；其他 locale 默认归为英文。向导界面语言与生成配置中的 `confirmationLanguage` 是两个独立概念，但初始化时后者默认跟随前者，用户可在可选设置中改写。

为了让 `--help` 在 clap 完整解析前就能选择语言，程序启动时先对 argv 做只识别 `--language` 的轻量预扫描，再构建和本地化 clap command tree。

### `config init`

目标接口：

```bash
agentic-gpt config init [OPTIONS]

Options:
  --mode standalone|hub|local
  --profile normal|room
  --non-interactive
  --tunnel-id <ID>
  --tunnel-api-key <file:PATH|env:NAME>
  --hub-url <URL>
  --hub-transport websocket|sse
  --agent-id <ID>
  --agent-secret <SECRET>
```

实际 help 会按当前语言提供说明、默认值、示例与安全提示。命令 token 和 value token 保持英文。

#### TTY 行为

在 stdin 与 stderr/stdout 均适合交互，且未传 `--non-interactive` 时启动向导。显式参数作为向导默认值；已提供的必要值不重复询问。

主流程：

1. 选择 mode，默认 Standalone。
2. 选择 profile，默认 Normal。
3. 询问 mode 必需信息。
4. 询问是否配置可选项；默认否。
5. 若选择是，使用多选菜单选择配置区块。
6. 显示本地化摘要并确认写入。
7. 构造、校验并一次性写入配置；中断或取消时不写入部分配置。

#### 非 TTY 行为

没有可交互 TTY 时自动使用非交互路径，绝不读取 stdin 等待选择。默认 mode/profile 仍是 `standalone + normal`。

非交互路径允许缺省模式必需值，但只能生成带安全占位符的完整模板，并在 stderr 与完成摘要中明确列出启动前必须替换的字段：

- Standalone：`tunnel_replace-me` 与默认受保护文件引用
- Hub：示例 Hub URL 与 `change-me` agent secret
- Local：不要求 tunnel 或可用 Hub 凭据

自动化环境要生成可直接运行的配置，应显式传入相应 flags。任何实际 secret 都不得出现在命令完成摘要、日志或备份提示中。

### Mode 必需流程

#### Standalone

必需配置：

- tunnel ID
- API key 引用

API key 来源选择：

1. Protected file（推荐）
2. Environment variable

Protected file 默认路径为 `~/.agentic_gpt/secrets/tunnel-api-key`。向导询问是否现在写入 secret：

- 选择写入时使用隐藏输入，创建 `0700` 目录与 `0600` 文件；配置仅保存 `file:` 引用。
- 选择稍后写入时只保存引用，并在摘要中标记待完成事项。

Environment variable 只询问变量名并保存 `env:NAME`，不读取或持久化 secret 值。

#### Hub

必需配置：

- Hub URL
- transport（默认 websocket）
- agent ID
- agent secret（隐藏输入）

Hub secret 写入 JSON 是现有 schema 行为；摘要中只显示已设置，不显示内容。

#### Local

Local 没有额外必需凭据，生成公共本地配置与所选 profile 的默认配置。现有 schema 所需 Hub 字段保留中性默认值，但不在主流程中突出显示。

## 可选设置入口

必需项完成后显示：

- 使用安全默认值
- 选择可选设置区块

选择后进入按 mode/profile 裁剪的 MultiSelect：

- Identity and display name
- Workspace and path policy
- Confirmation channels and notification language
- Runtime limits
- Sandbox
- Room settings（仅 room）
- Tunnel client overrides（仅 standalone）
- Hub reporting（仅 standalone）

每个区块只询问该区块的字段；按 Enter 接受当前默认值。首次向导不包含 MCP servers 和命令 allow/confirm/deny 规则，完成摘要提供对应后续命令。

## 交互库与样式

采用 `inquire` 实现 Select、MultiSelect、Confirm、Text、CustomType 与 Password prompt。

界面目标是现代但克制：方向键选择、当前项高亮、默认值、简短帮助、内联校验和密码隐藏；不实现全屏 TUI。遵循 `NO_COLOR`，在不支持样式的终端保持可读纯文本输出。

为避免业务逻辑和终端 UI 耦合，向导依赖一个小型 prompt abstraction。生产实现由 `inquire` 驱动；测试实现按预设答案返回，测试不需要真实 PTY。

## 双语 CLI

### 稳定语法

以下内容不翻译：

- executable、命令与子命令名称
- flag 名称
- enum value
- JSON key
- 稳定错误码

### 本地化内容

以下由统一文本目录提供中英文版本：

- 程序、命令、子命令和参数说明
- help template 的 Usage、Commands、Options、Arguments 等标题
- 默认值和示例说明
- 向导问题、选项补充说明与校验反馈
- 取消、覆盖确认、失败与完成摘要
- 本次新增或修改的配置命令错误说明

原始 OS 错误、第三方库内部错误和协议层机器错误不强行翻译；对用户展示时由 Agentic 拥有的上下文文本包裹，并保留底层原因。

实现集中为 `UiLanguage` 与编译期完整的 `CliText` 目录，不在流程代码中散落双语字符串。clap derive 继续定义稳定命令结构，运行时通过 `CommandFactory::command()` 递归附加本地化描述、参数 help 与 help template。

## `config set` 与可发现性

`config set` 保持受控 registry，不成为任意 JSON 编辑器。registry 是以下行为的单一事实来源：

- key 是否受支持
- value 类型与解析器
- accepted values / range
- nullable 规则
- 中英文说明与示例
- 实际 mutation 函数

补齐目前未覆盖、且适合安全标量或 JSON 列表设置的字段，包括：

- `displayName`
- `backupLimit`
- `sandbox.bubblewrapPath`
- `sandbox.requiredRuntimePaths`
- `limits.maxConcurrentTasks`
- `limits.maxActiveJobs`
- `limits.maxFileSearchContextLines`

并修正：

- `room.notebookRoot` 接受 `null` 清空
- list 类型使用 JSON array 输入，并返回本地化类型错误
- `limits.maxActiveJobs` 接受 `auto` 或非负整数
- 所有范围复用配置层既有约束，避免 CLI 与反序列化规则漂移

新增：

```bash
agentic-gpt config keys [--section <NAME>] [--json]
```

默认输出按 section 分组的 key、类型与简短说明；`--json` 提供稳定机器可读结果。`config set --help` 展示常用示例并引导到 `config keys`，避免把完整 registry 塞进一页 help。

结构化集合继续使用专门命令：

- `config allow|confirm|deny`
- `config path`
- `config mcp`

## `config init` 生成策略

新增纯函数式 template builder，输入为：

- mode
- profile
- UI language / confirmation language default
- 必需值与可选区块答案

输出为 `Config` 加待完成事项列表。交互 UI 不直接修改 JSON。

构造完成后：

- Local 使用 `validate_local()`
- Standalone 使用 `validate_standalone()`；占位符模板需满足结构与引用语法校验
- Hub 验证 URL、transport、非空 identity/secret 与 MCP 配置

写入继续复用现有备份机制。若目标已存在，交互模式在摘要确认中明确说明将创建备份；非交互模式保持当前可脚本化覆盖语义。

## 代码边界

建议新增或拆分：

- `cli_i18n.rs`：语言检测、文本目录、clap command tree 本地化
- `config_cli.rs`：ConfigCommand、init options、key registry 与 command dispatch
- `config_wizard.rs`：wizard flow、prompt abstraction 与 inquire backend
- `config_templates.rs`：mode/profile template builder 与校验

`main.rs` 只保留顶层 CLI 入口与 dispatch，避免继续扩大当前单文件职责。现有 `policy.rs` 与 `mcp.rs` mutation 逻辑保持独立。

## 错误与取消语义

- Ctrl-C、Esc 或 prompt 取消：不写配置和 secret 文件，返回本地化取消提示与非零状态。
- secret 文件只在最终确认之后创建；配置写入失败时不在摘要中声称成功。
- 输入校验在 prompt 内即时反馈，最终 builder 再做一次完整校验。
- 不在错误、debug 输出或完成摘要中打印 secret。
- 不识别的 `--language`、mode、profile 或 config key 由 clap/registry 返回本地化、可操作的错误。

## 测试策略

### Template 与 registry 单元测试

- mode/profile 组合生成预期相关块
- Standalone 默认与显式 tunnel 引用
- Room 默认和可选字段
- 非交互占位符及待完成事项
- 每个 registry key 的合法值、非法值、范围与 null 行为
- registry 元数据和实际 mutation 不漂移

### CLI 测试

- `config --help`、各级子命令 help 在 zh-CN/en 下具有完整说明
- 命令名和参数名在两种语言下保持一致
- `--language` 位于子命令前后均生效
- locale 自动检测优先级
- `config keys` 文本与 JSON 输出
- 非 TTY 不等待输入

### Wizard 测试

通过 scripted prompt backend 测试：

- 默认 Standalone + Normal 路径
- Hub、Local、Room 分支
- 可选区块裁剪
- 用户取消与最终拒绝写入
- secret 永不出现在摘要

仅增加少量 PTY smoke test 验证 `inquire` 集成，不用 PTY 承载业务分支覆盖。

## 验收标准

1. 中文 locale 裸跑 `config init` 得到中文 Standalone + Normal 向导；英文 locale 得到英文向导。
2. 非 TTY 裸跑不会阻塞，并生成完整、明确标记待替换项的 Standalone + Normal 模板。
3. 显式 flags 可以无交互生成可直接使用的 Standalone、Hub 或 Local 配置。
4. `config --help` 到叶子子命令不再出现空说明；中英文均完整且语法 token 一致。
5. `config keys` 能发现所有 `config set` key、类型和约束。
6. 新增 registry key 能正确写回 canonical JSON，并通过现有配置反序列化与校验。
7. 取消向导不产生部分配置或部分 secret 文件。
8. 所有 secret 在 prompt、日志、错误和摘要中保持隐藏。

## 兼容性

- 裸 `config init` 从直接写默认文件变为 TTY 中启动向导，这是有意的 UX 变化。
- 非 TTY 保持无阻塞、可脚本化。
- 现有 `config init --config PATH`、`config set`、policy/path/MCP 命令继续可用。
- canonical JSON schema 与现有加载兼容逻辑保持不变。
