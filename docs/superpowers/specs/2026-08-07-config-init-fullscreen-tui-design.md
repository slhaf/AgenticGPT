# AgenticGPT `config init` Fullscreen TUI 设计

日期：2026-08-07  
状态：已确认，待实现计划

## 1. 背景与范围

2026-08-04 的 `config init` 设计已经完成了模式化初始化、非交互参数、双语 CLI、template builder 与基于 `inquire` 的交互式向导。实际运行后暴露出交互层问题：

- `inquire` 在 Esc / CapsLock→Esc 场景会打印 `<canceled>`，即使业务层选择忽略取消也会留下视觉垃圾。
- 当前界面是连续 prompt 历史，步骤拥挤、层级弱、输入字段缺少清晰的编辑状态和输入前缀。
- 可选配置使用“是否配置 → MultiSelect → 顺序填写”的线性流程，不适合反复进入多个 section、修改后返回、再继续配置。
- Review 只是摘要 + Yes/No，发现问题后不能直接跳回对应设置。
- `PromptBackend::ask()` 的顺序模型天然不适合页面栈、表单焦点、Review 回跳和 staged editing。

本设计仅重做 `config init` 的交互式体验，并建立一层最小可复用 Fullscreen TUI 基础设施。

本设计覆盖并替代 `2026-08-04-config-init-wizard-design.md` 中以下部分：

- `config init` 的 TTY 交互流程；
- `inquire` 交互库与 prompt abstraction；
- Esc / 取消语义；
- 非 TTY 裸跑 `config init` 的行为；
- 交互模式下显式 init flags 的处理方式。

2026-08-04 设计中的以下内容继续有效，除非本设计明确修改：

- mode/profile/template builder；
- `--non-interactive` 参数式初始化；
- 双语 CLI 与稳定英文语法 token；
- `config set` / `config keys`；
- canonical JSON schema；
- secret 安全约束与备份机制。

## 2. 目标

1. 将交互式 `config init` 改为现代、克制、全屏的 Setup Wizard。
2. 使用 `ratatui + crossterm`，从机制上消除 `<canceled>` 等连续 prompt 历史问题。
3. 将 setup 业务 session 与 TUI 页面/输入状态分别显式建模，而不是继续依赖顺序 `ask()`。
4. 所有配置先写入内存 staged state，只有最终确认时才一次性落盘。
5. 可选配置改为可反复进入的“配置中心”，每个 section 独立子页面。
6. Review 页面可直接跳回对应设置修改。
7. 将 setup 的 draft、校验、Review 与最终 outcome 构建从 Ratatui 前端中分离，形成 frontend-neutral 的 setup application/domain 层。
8. 建立一层不认识 config 业务的最小公共 TUI runtime，未来可被 Jobs、Python REPL、Terminal 等独立功能复用。
9. 保持现有 `--non-interactive` 路径为明确、稳定的自动化接口。
10. 在当前 keyd 将 CapsLock 映射为 Esc 的环境下，单次 Esc 不得终止整个向导。

## 3. 非目标

本轮不实现：

- Inline TUI；
- Agentic 主控制台 / Dashboard；
- Job / Process Browser；
- Python REPL Session；
- Terminal / PTY Session API；
- 鼠标操作；
- pane / tab / split 等 Zellij 式终端复用能力；
- MCP server 初始化；
- policy rule 初始化；
- 对 `Config` schema 做结构性重构；
- 扩展当前 `console/` KMP 前端、实现 KMP setup 页面或现在就设计 Rust↔Kotlin bridge/API。

公共 TUI 层和 frontend-neutral setup 层都只实现 `config init` 当前真正需要的能力，不为未来功能提前增加抽象。

## 4. 总体架构

采用 `ratatui + crossterm` Fullscreen App + 独立 Wizard 状态机。

概念边界：

```text
crates/agentic-gpt/src/tui/
  runtime        终端 enter/restore、event loop、resize
  theme          统一样式 token
  widgets        少量通用输入/选择/菜单/footer 控件

crates/agentic-gpt/src/config_setup/
  model          SetupSession / SetupDraft / mode-specific drafts
  validation     frontend-neutral 的字段、section、整体校验
  review         frontend-neutral 的 ReviewModel
  outcome        build_config / SecretWritePlan / commit handoff

crates/agentic-gpt/src/config_tui/
  app            TUI event handling 与 setup use-case 调用
  navigation     page stack、Review return target
  pages          Setup 页面渲染
  input          focus、editing、scroll、modal 等纯前端状态

config_templates / config_cli / config domain
  继续负责真实配置构建、canonical validation、非交互模式与落盘
```

文件名可在实现计划中按现有模块风格微调，但职责边界保持不变。

### 4.1 公共 `tui/` 的职责

`tui/` 不认识：

- `RuntimeMode`；
- `Config`；
- tunnel/hub/room；
- Job、Python、Terminal。

它只负责：

- 进入/退出 alternate screen；
- raw mode；
- 光标显隐；
- resize；
- 按键事件；
- frame draw loop；
- 主题 token；
- 输入、单选、菜单、footer 等可复用控件。

### 4.2 `config_setup/` 的职责

`config_setup/` 是 frontend-neutral 的 setup application/domain 层。它负责：

- selected mode/profile；
- mode-specific staged draft；
- optional section staged draft/status；
- secret write plan draft；
- 字段/section/整体校验及结构化错误；
- Review 数据模型；
- 将 staged state 转换为 `WizardOutcome` / commit handoff。

这一层不得依赖 Ratatui、Crossterm、terminal key event、页面编号、光标或终端尺寸。它继续复用既有 `config_templates::build_config` 与 canonical config validation，不复制 schema 规则。

### 4.3 `config_tui/` 的职责

`config_tui/` 是 setup 的终端前端适配层。它负责：

- 当前页面；
- 页面栈和 Review 回跳来源；
- 当前 focus；
- 当前是否处于文本编辑状态；
- 文本编辑缓冲、scroll、modal 等 UI-only state；
- 将终端事件转换为对 `config_setup` 的业务动作；
- 将 `config_setup` 返回的 draft/status/error/review model 渲染为 TUI。

`config_tui/` 不拥有 mode 切换语义、optional applicability、配置校验或 Review 业务构建规则，也不直接拼 JSON。

### 4.4 与 KMP/其他前端的关系

仓库已有 `console/` KMP/Compose Multiplatform 前端，但当前功能仍较薄，本轮不修改它。架构上把它视为与 TUI 同层级的另一个 frontend，而不是 TUI 的上层或下层。

本轮只保证 setup 业务语义不会被写死在 Ratatui 层。未来 KMP 真正实现 setup 时，应通过届时确定的稳定控制 API/协议复用同一 Rust setup use-case 语义，而不是在 Kotlin 中重新复制 mode applicability、validation、Review 和 commit 规则。本设计不提前决定或实现这条跨语言 transport。

## 5. CLI 行为

### 5.1 交互式入口

```bash
agentic-gpt config init
```

当 stdin、stdout、stderr 都是可交互 TTY，且未传 `--non-interactive` 时，启动 Fullscreen TUI。

当前已有的三端 TTY 判定继续作为第一版能力门槛；本轮不实现“自动判断 alternate screen 支持后降级 Inline TUI”。

### 5.2 非交互入口

```bash
agentic-gpt config init --non-interactive [OPTIONS]
```

始终走现有非交互路径，不初始化 Ratatui，不读取交互输入。

### 5.3 非 TTY 且未显式 `--non-interactive`

普通 `config init` 在 pipe / CI / stdin 重定向等非交互环境中：

- 不自动生成配置；
- 不回退到 `inquire`；
- 不等待 stdin；
- 返回可操作错误，提示显式使用 `config init --non-interactive ...`。

这是相对 2026-08-04 设计的有意行为变化。交互和自动化必须由用户意图明确区分。

### 5.4 交互模式中的 flags

交互模式下：

```text
--mode
--profile
--tunnel-id
--tunnel-api-key
--hub-url
--hub-transport
--agent-id
--agent-secret
```

均视为 Wizard 的 **seed / 预填值**，不是锁定值，也不导致页面永久跳过。

用户可在 TUI 中查看和修改这些值。

只有 `--non-interactive` 才把这些 flags 视为直接执行参数。

交互模式不因“当前 mode 与另一个 mode 的 seed 同时存在”在启动阶段报 applicability error。例如 `--mode hub --tunnel-id x` 可以进入 Hub 页面；`tunnel-id` 作为 Standalone draft 保留，若用户切换到 Standalone 可继续使用。

非交互模式继续执行严格的 mode applicability 校验。

## 6. Wizard 页面模型

Wizard 使用动态页面流：

```text
基础设置
   ↓
连接设置       Local 时跳过
   ↓
可选配置中心
   ↓
检查与写入
   ↓
完成
```

顶部进度显示当前实际页面数，不为不适用步骤制造空页。

示例：

```text
AgenticGPT Setup                                      2 / 4
```

主进度只覆盖“基础 / 连接 / 可选 / Review”四类配置步骤；Local 跳过连接页时总数随实际流程缩减。完成页不再显示 `n / n` 进度，只显示完成状态。

## 7. 视觉布局

### 7.1 总体风格

采用：

- 单列；
- 限宽；
- 上半部锚定；
- 留白优先；
- 单一 accent + dim secondary text；
- 少量水平分隔线；
- 不做满屏 box；
- 不严格垂直居中。

主内容宽度目标约 70–90 columns，并随终端宽度缩放。

### 7.2 基础结构

```text
 AgenticGPT Setup                                      2 / 4
────────────────────────────────────────────────────────────

     连接设置
     配置 Standalone 与 Tunnel 的连接信息

     Tunnel ID
     用于标识当前 Tunnel 实例
     › tunnel_1

     Secret 来源
     ● 受保护文件
     ○ 环境变量

     Secret 文件
     › ~/.agentic_gpt/secrets/tunnel-api-key


                                      [ 下一步 ]

────────────────────────────────────────────────────────────
 Tab/↑↓ 切换   Enter 操作   Esc 返回   Ctrl+C 退出
```

### 7.3 小终端

第一版不追求极端窄屏的完美体验，但必须：

- 不 panic；
- 不越界；
- 文本可截断或换行；
- 主操作仍可访问；
- resize 后保留 `SetupSession` staged state 与当前 TUI 导航/编辑状态。

## 8. 页面内容

### 8.1 基础设置

同页包含：

- Runtime Mode；
- Profile。

不是“一字段一页”。

选择项带简短说明，提供足够视觉重量和语义信息。例如：

```text
● Standalone
  本机运行 Agent，并通过 Tunnel 暴露能力

○ Hub
  连接 AgenticGPT Hub

○ Local
  仅提供本地控制接口
```

Mode 改变后，后续页面和可选 section 可用性即时重算。

### 8.2 连接设置

#### Standalone

同一语义页中包含：

- Tunnel ID；
- Secret 来源；
- Secret 文件路径或环境变量名；
- file source 时是否现在 provision secret；
- 需要立即 provision 时的 secret 输入。

依赖字段按当前选择动态显示。

#### Hub

包含：

- Hub URL；
- Transport；
- Agent ID；
- Agent Secret。

#### Local

没有连接页，直接跳到可选配置中心。

### 8.3 Mode-specific draft

Wizard 在内存中保留各 mode 的 staged draft，但只有当前 mode 对应字段参与最终 build。

例如用户：

1. 在 Standalone 输入 tunnel 数据；
2. 切换 Hub；
3. 填 Hub 数据；
4. 再切回 Standalone；

之前的 Standalone staged 值仍可恢复，避免切换 mode 时无意义丢输入。

未激活 mode 的 secret 只存在当前 Wizard 进程内存，不写磁盘、不进入 summary、不进入日志；Wizard 退出后丢弃。

## 9. 可选配置中心

取消旧流程：

```text
是否配置可选项？
→ MultiSelect
→ 逐项顺序 prompt
```

改为配置中心：

```text
可选配置

❯ 身份与显示             默认
  工作区与路径策略       已配置
  确认方式与语言         默认
  运行时限制             默认
  沙箱                   默认
  Room                   默认
  隧道客户端覆盖设置     不适用
  Hub 报告               不适用

  完成并继续
```

### 9.1 section 可用性

继续使用 mode/profile 裁剪：

公共：

- Identity；
- Workspace；
- Confirmation；
- Limits；
- Sandbox。

`profile=room`：

- Room。

`mode=standalone`：

- Tunnel client overrides；
- Hub reporting。

不适用项保留在列表中，以 dim 的“不适用”状态显示且不可聚焦，使用户能理解当前 mode/profile 下为什么没有该入口。

### 9.2 section 交互

- Enter 进入聚焦 section；
- section 是独立子页面；
- “保存并返回”只保存到 staged state；
- 返回配置中心后可进入任意其他 section；
- 可重复进入同一个 section 修改；
- “完成并继续”进入 Review。

### 9.3 section 状态

至少支持：

- `默认`：未显式修改，最终使用 builder/default 行为；
- `已配置`：当前 Wizard 有显式 staged 值；
- `不适用`：当前 mode/profile 不允许。

不需要引入更多状态，除非实现时确有语义需要。

## 10. 输入、焦点与编辑状态

### 10.1 两层输入状态

文本字段有：

1. focused；
2. editing。

focused 时字段可见强调，但不会直接吞普通文字输入。

示例：

```text
Tunnel ID
用于标识当前 Tunnel 实例
› tunnel_1
```

按 Enter 后进入 editing，并显示真实光标：

```text
Tunnel ID
用于标识当前 Tunnel 实例
› tunnel_1█
```

正式 UI 不显示 `focused` / `editing` 字样。

### 10.2 键盘语义

导航状态：

- `Tab` / `Shift+Tab`：前后字段；
- `↑↓`：列表/选择控件移动；在适合的表单场景也可用于前后 focus；
- `Enter`：进入编辑、选择、进入子页或触发按钮；
- `Esc`：返回上一层；
- `Ctrl+C`：全局取消 Wizard。

编辑状态：

- 普通字符：编辑；
- `Enter`：确认当前字段编辑并退出 editing；
- `Esc`：只退出 editing，不离开页面；
- `Ctrl+C`：仍为全局取消。

Wizard 根页面：

- `Esc` 不取消整个 Wizard；无上一层时 no-op；
- 退出整个初始化只有 `Ctrl+C`。

这一语义是明确约束，用于避免 keyd 的 CapsLock→Esc 映射再次造成意外退出。

### 10.3 Footer

footer 随状态变化，不显示无效快捷键。

例如 focused：

```text
Enter 编辑   Tab/↑↓ 切换字段   Esc 返回   Ctrl+C 退出
```

editing：

```text
Enter 确认   Esc 结束编辑   Ctrl+C 退出
```

## 11. Setup 校验与页面错误呈现

字段、section 和整体校验规则属于 `config_setup`；TUI 只决定何时触发校验、如何聚焦错误字段以及如何把结构化错误以内联形式展示。普通配置错误使用 inline validation，不弹 modal。

示例：

```text
Tunnel ID
›

  必填项不能为空
```

触发时机：

- 点击“下一步”或“保存并返回”时完整校验当前页面；
- 页面已产生字段错误后，用户完成该字段的一次编辑时重新校验该字段，合法后立即清除对应错误。

失败行为：

- 留在当前页面；
- 聚焦第一个错误字段；
- 就地显示错误。

覆盖：

- required text；
- URL/transport；
- number/range；
- path list；
- secret 非空；
- enum；
- section-specific syntax。

最终进入 Review 前，以及真正 commit 前，必须继续调用真实 builder / config validation 再校验一次，避免 UI 校验成为另一套事实来源。

## 12. Staged state 与原子写入

业务 staged state 由 `config_setup::SetupSession` 持有；TUI 只持有未确认的文本编辑缓冲等 UI-only 临时状态。Wizard 全程只修改这些内存 state。

在最终确认前，不允许：

- 写 config 文件；
- 创建 secret 文件；
- 创建 config backup；
- 修改已有配置。

section 中的“保存”只代表保存到 `SetupSession` staged state。

Secret 输入保存在 secret-aware wrapper 中，不进入 Debug、Display、render snapshot 或普通日志。

最终步骤：

```text
staged state
   ↓
build_config / 完整 validation
   ↓
生成 WizardOutcome + SecretWritePlan
   ↓
用户确认并写入
   ↓
commit_wizard_outcome / 既有安全写入路径
```

保持现有“secret 只在最终确认后 provision”的语义。

## 13. Review：检查与写入

`config_setup` 从当前 staged state 生成 frontend-neutral `ReviewModel`；TUI 将其渲染成可导航 Review 页面，而不是纯文本 summary + Confirm。Review 的业务字段、脱敏值、pending actions 与 section 状态不由 Ratatui 页面自行重新计算。

示例：

```text
检查与写入

❯ 基础设置
    模式          Standalone
    Profile       normal

  连接设置
    Tunnel ID     tunnel_1
    Secret        file:~/.agentic_gpt/...
    Secret 写入   是 · ••••••••

  可选配置
    身份与显示    默认
    工作区        已配置
    沙箱          默认

  配置文件
    ~/.agentic_gpt/config.json

                         [ 确认并写入 ]
```

### 13.1 Review 回跳

聚焦某个 group 后按 Enter：

- 基础设置 → 基础页；
- 连接设置 → 当前 mode 的连接页；
- 可选 section → 对应 section / 配置中心；
- 修改完成后回到 Review，而不是强制沿主流程重新走一遍。

实现应显式记录“从 Review 进入编辑”的 return target，而不是依赖固定 page order。

### 13.2 Review 内容

必须展示：

- mode/profile；
- 当前 mode 的关键连接信息；
- optional section 状态/关键摘要；
- config path；
- 是否存在 backup；
- pending actions；
- secret 是否会写入，但绝不显示 secret 内容。

不适用 mode 的 staged draft 不进入 Review。

## 14. 完成页

commit 成功后显示轻量完成页：

```text
✓ AgenticGPT 已完成初始化

配置
~/.agentic_gpt/config.json

下一步
<已有且真实的后续 CLI 提示>

                         [ 完成 ]
```

本轮不加入不存在的 Dashboard/主 TUI 入口。

退出 alternate screen 后，保留现有普通 CLI 完成摘要/配置路径输出，使结果在终端 scrollback 中仍有持久记录。

## 15. 错误与取消

### 15.1 表单错误

使用 inline validation，不弹窗。

### 15.2 系统/提交错误

以下系统级错误使用 modal / blocking error page：

- config 写入失败；
- secret provision 失败；
- backup 失败；
- terminal setup/restore 异常；
- 其他无法归属到某个表单字段的系统错误。

错误内容不得泄漏 secret。

### 15.3 Ctrl+C

`Ctrl+C` 是唯一全局取消快捷键。

取消时：

- 不写 config；
- 不写 secret；
- 恢复终端；
- 返回既有本地化取消错误/退出语义。

### 15.4 Panic / error terminal restore

TUI runtime 使用 RAII guard 管理：

- raw mode；
- alternate screen；
- cursor visibility。

正常返回和可传播 error path 必须恢复终端。

TUI runtime 必须覆盖 panic cleanup：panic 时尝试恢复 raw mode、alternate screen 与 cursor。若通过 panic hook 实现，必须链式调用并保留已有 hook，不能静默覆盖。

## 16. `inquire` 与旧 PromptBackend 的迁移

新 Fullscreen Wizard 不以：

```text
PromptRequest → backend.ask() → PromptAnswer
```

作为核心执行模型。

原因：该模型假设严格顺序问答，与以下需求冲突：

- 表单多字段 focus；
- optional config center；
- section 反复进入；
- Review 回跳；
- mode 动态切换；
- editing / navigation 双状态。

迁移原则：

- 业务纯函数继续复用；
- sequential prompt flow 可逐步删除；
- `InquirePromptBackend` 完成迁移后不再用于 `config init`；
- 若仓库其他位置没有 `inquire` 使用，最终移除依赖；
- 当前针对 Esc 的临时 `inquire` retry 修复只作为过渡行为，不成为最终架构的一部分。

## 17. 状态与数据流

业务状态和前端状态明确拆开：

```text
SetupSession                         TuiState
  selected_mode/profile               current_page / page_stack
  mode_drafts                          return_target
  optional_section_drafts              focus
  optional_section_status              edit_state / edit_buffer
  structured_validation_errors         scroll
  secret_write_plan draft              modal
  config_path
```

`SetupSession` 不包含任何 Ratatui/Crossterm 类型；`TuiState` 不重新实现 config applicability、validation、Review 或 outcome 构建规则。

事件循环：

```text
Terminal Event
    ↓
map to TuiAction
    ↓
update UI-only state
或调用 SetupSession use-case
    ↓
SetupSession returns state / validation / ReviewModel
    ↓
render(TuiState, SetupSession view)
```

文本输入 editing 期间可以先修改 TUI 的 edit buffer；用户 Enter 确认编辑时再把值提交给 `SetupSession`。这样 Esc 结束编辑可以明确恢复/保留当前已确认业务值，而不会把每个按键都变成 domain mutation。

渲染函数不得执行文件写入或业务副作用。最终 commit 是独立显式 use-case。

## 18. 主题与通用组件

第一版公共组件保持最小：

- text input；
- secret input；
- single choice / radio；
- menu list；
- action button；
- inline field error；
- header / progress；
- contextual footer；
- modal/system error surface。

主题至少定义：

- accent/focus；
- normal foreground；
- dim/help；
- success；
- warning；
- error；
- disabled。

不允许业务页面直接散落大量具体颜色值。

遵循 `NO_COLOR`：设置后不使用色相区分状态，但 Fullscreen TUI 仍正常工作，并依赖 `›` / `●` / `○` 等符号、文字修饰与结构保持焦点和状态可识别；颜色不得成为唯一状态信号。

## 19. 测试策略

### 19.1 `config_setup` 业务单元测试

不依赖 Ratatui/PTY，覆盖：

- 默认 Standalone + Normal；
- mode/profile applicability；
- mode 切换与 mode-specific draft 保留；
- optional section staged draft/status；
- interactive flags 是 seed，不是锁定值；
- 不适用 seed 不参与当前 mode build；
- required / URL / numbers / path lists / secret / section-specific constraints；
- frontend-neutral `ReviewModel` 的内容、脱敏与 pending actions；
- builder 最终校验仍可捕获跨字段错误；
- staged state 在最终 commit 前无文件副作用；
- secret 不进入 debug/renderable review model。

### 19.2 `config_tui` 状态/导航测试

覆盖：

- Hub/Local/Room 页面裁剪呈现；
- optional section 进入 → 保存 → 返回 → 再进入；
- Review 回跳及回到 Review；
- edit buffer 与已确认 setup 值分离；
- `Esc` 在 editing 只退出编辑；
- `Esc` 在子页返回；
- 根页 `Esc` no-op；
- `Ctrl+C` 任意页面取消；
- setup 返回的结构化 validation error 映射到正确字段。

### 19.3 Ratatui `TestBackend`

关键页面做 buffer/layout 测试：

- Basic；
- Standalone connection；
- Hub connection；
- Optional center；
- 一个复杂 optional section；
- Review；
- Completion；
- system error modal。

测试关注结构、关键文本和 focus 标识，不对颜色做脆弱像素式断言。

### 19.4 CLI / TTY 行为测试

覆盖：

- `--non-interactive` 不启动 TUI；
- 非 TTY 裸 `config init` 返回 actionable error，不自动写配置；
- 交互 TTY 启动 Fullscreen Wizard；
- Ctrl+C 退出后无 config/secret 副作用；
- 参数 seed 可在 Wizard 中修改。

### 19.5 真实终端 smoke

使用真实 PTY/tmux 做少量 smoke：

- alternate screen 进入/退出；
- raw mode 恢复；
- resize；
- Esc/CapsLock 对当前页行为；
- Ctrl+C；
- secret 输入不回显；
- commit 后 config/secret 正确；
- error path 后终端仍可正常输入。

视觉样式最终仍需人工运行检查，不为“好不好看”制造无意义自动测试。

## 20. 安全与隐私

- Secret 不进入 Review 明文；
- Secret 不进入普通日志、debug、panic message、snapshot；
- secret text widget 不回显真实字符；
- 不适用 mode 的 secret draft 也只保留在当前进程内存；
- Ctrl+C / error / wizard drop 后尽快 drop secret-bearing state；
- secret 文件权限继续复用现有安全写入语义；
- `--agent-secret` 仍保留现有 CLI 对 shell history / process inspection 的安全提示。

## 21. 验收标准

1. `agentic-gpt config init` 在交互 TTY 中进入 Fullscreen TUI，不出现连续 `inquire` prompt 历史和 `<canceled>` 文本。
2. `Esc` 不会退出整个 Wizard；`Ctrl+C` 是唯一全局取消键。
3. Mode/Profile 与 mode-specific connection 使用语义分组页面，不是一字段一页。
4. 输入字段有明确 focus/edit 状态与 `›` 输入 affordance。
5. 可选配置是导航中心，可反复进入多个 section，保存后返回。
6. 所有页面修改仅写 staged state；最终确认前不创建 config、backup 或 secret 文件。
7. Review 可直接跳回对应设置修改，再返回 Review。
8. Secret 始终脱敏且不进入日志/snapshot。
9. Local 模式不显示空 connection 页面。
10. interactive flags 作为预填值，用户可修改；`--non-interactive` 继续严格参数执行。
11. 非 TTY 裸 `config init` 不自动生成配置，而是提示使用 `--non-interactive`。
12. resize、Ctrl+C、普通 error 后终端状态可恢复。
13. `config_setup` 不依赖 Ratatui/Crossterm 或 TUI 导航类型，draft、applicability、validation、Review 与 outcome 规则可脱离终端前端测试和调用。
14. `config_tui` 只负责终端交互/导航/呈现，不复制 setup 业务规则；TUI 与现有 KMP `console/` 在架构上是同层级 frontend。
15. 本轮不实现 KMP setup/bridge、Inline TUI、Jobs、Python REPL 或 Terminal Session。

## 22. 后续但不属于本轮

未来真实需求出现后可独立设计：

- Inline TUI viewport；
- Agentic 主 TUI / Dashboard；
- Job/Process history browser；
- Python stateful REPL；
- PTY Terminal Session；
- 这些功能可以复用本轮 `tui/` runtime/components，但不应反向扩大本轮范围。
