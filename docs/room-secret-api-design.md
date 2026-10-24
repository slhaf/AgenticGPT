# Room Secret API 设计

状态：产品语义已确认，首阶段代码与接口已实现；真实 Vaultwarden 互通验证待完成。最后整理：2026-10-24。

本文记录原始设计及可行性审查中确认的边界，供实现和评审追溯。当前配置、调用步骤及实现限制见[Room 密钥使用指南](room-secret.zh-CN.md)；真实 provider 的兼容性不能仅由编译和替代 CLI 测试推定。

## 1. 范围与归属

- 密钥能力属于 Room 领域，统一使用 `room.secret.*`。
- 同一 Agentic 实例的获准调用方共享全部密钥，第一阶段不按调用 Agent 分库或增加 owner 字段。
- 使用独立的 Vaultwarden 账号连接指定的自托管实例，不接入使用者的个人密码库。
- 每个 Secret 对应一个 `(site, account)` 账号组，包含可选的 `password` 和可选的 `totp`；创建时至少提供一个。
- 第一阶段提供 `room.secret.list`、`room.secret.get`、`room.secret.upsert`、`room.secret.delete`。
- 所有接口面均需接入：Agent 本地 MCP（适用的 stdio／Unix／HTTP MCP 路径）、Hub wire、Hub HTTP／OpenAPI、Apps MCP；各入口保留自身认证、准入、错误和响应封套。
- Secret 由实际执行请求的 Room／Agent 实例持有。Hub 的 Room 请求沿用当前活动 Room 路由，不在 Hub 另建一份密码库或同步任务。

第一阶段不提供：多个 Vaultwarden 账号、多个 provider、密码生成器、历史版本查询、回收站管理／恢复接口、其他 Bitwarden item 类型、附件、任意进程环境变量注入、浏览器无明文填充、批量导入导出或批量修改。

不提供回收站管理接口，不影响 `delete` 使用 Vaultwarden 自身的回收站。

## 2. 数据模型与唯一身份

以下为结构示例，字段值仅为占位符：

```json
{
  "site": "github.com",
  "account": "agent@example.invalid",
  "password": "<PASSWORD_PLACEHOLDER>",
  "totp": "<BASE32_OR_OTPAUTH_TOTP_PLACEHOLDER>"
}
```

| 字段 | 语义 |
| --- | --- |
| `site` | 站点或服务的稳定名称。 |
| `account` | 站点内账号标识，例如用户名、邮箱或手机号。 |
| `password` | 可选密码；不自动裁剪或转换其内容。 |
| `totp` | 可选 TOTP seed，接受受支持的 Base32 secret 或 `otpauth://totp/...` URI。只用于保存和生成验证码，不通过 `get` 返回。 |

`site`、`account` 比较前裁剪首尾空白，裁剪后不得为空。`site` 不区分大小写，`account` 区分大小写；同一站点的 `Alice` 和 `alice` 可以是两个账号。保存和返回保留显示形式。不要自动合并 `GitHub`、`github.com`、`https://github.com` 等不同字符串。

`(site, account)` 是公开唯一身份。底层 item ID 仅用于内部定位、并发处理及权重关联，不出现在 Secret API 请求或响应中。

如果发现多个相同身份的条目，`get/upsert/delete` 返回 `ambiguous_secret`，不得自行选择或批量修改；调用方需通过 Vaultwarden 客户端处理重复项。`list` 不以排序结果替代唯一性判断。

## 3. Vaultwarden 与 bw 的职责

第一阶段使用官方 Password Manager CLI `bw`，不使用 Bitwarden Secrets Manager SDK。Vaultwarden 不提供 Secrets Manager API。

每个账号组映射为原生 Login item：

| Room 字段 | Bitwarden 字段 |
| --- | --- |
| `site` | `name`；识别为合适 URI 时可以同时写入 `login.uris`。 |
| `account` | `login.username` |
| `password` | `login.password` |
| `totp` | `login.totp` |

调用链为 `room.secret.* → Agentic Rust adapter → bw CLI → Vaultwarden`。

Agentic 负责输入校验、按公开身份定位、确认、同步任务协调、结果裁剪、权重和错误归一化。不能用 `bw` 的模糊搜索替代 `(site, account)` 的精确判断。调用方不接触 CLI 命令、session key 或原始 item JSON。

本地密码库的加密存储、解锁、同步和离线读取沿用 `bw` 的原生行为。Agentic 不另建密钥快照、明文密钥 JSON 缓存或第二套加密机制，也不承诺 `bw` 本地库具有额外的快照原子性。

`bw list items` 内部可能解密并输出完整条目。因此确认边界是“未经批准不得向调用方披露密钥”，不是“后端在确认前完全不接触密钥”。`list` 始终只向调用方返回账号元数据。

## 4. 同步任务与查询等待预算

### 4.1 触发、复用和生命周期

- 每次 Secret 操作前触发同步检查；已有同步任务时复用它，不再启动并行同步，也不更新原任务的触发时间。
- 同一实例由 Secret service 持有唯一在途同步任务、受控 `bw` 子进程和同步状态。
- 同步任务使用与 `process` 相同的 Tokio runtime，沿用现有后台服务的任务持有、停止和等待方式；不接入公开 `ManagedProcess` 链路。
- 不要求开启 process toolset，不占用公开进程容量，不进入 `process.list/read`、进程历史、普通命令策略或原始输出 reporting。
- 查询结束、调用方断线或查询等待预算耗尽，都不取消同步任务。
- 实例退出或能力关闭时，Secret service 负责停止并等待任务及子进程退出，再丢弃 session。不能只丢弃任务句柄而留下子进程。

### 4.2 查询等待预算

新增 `room.secret.syncWaitTimeoutMs`，单位毫秒，默认 `3000`。它控制 `list/get` 等待同步完成的最长时间；排队或复用在途任务也计入同一次等待预算，不因任务切换或重试重新获得完整预算。

预算到期只结束当前查询的等待，不终止同步、不把状态改为失败，也不发送“同步失败”事件。查询可以继续读取 `bw` 当前可用的本地密码库。

该预算不包括人工确认，也不是整个 Secret API 的响应时限。`bw` 本地读取仍需有独立的执行边界，不能假定离线读取始终立即完成。不得持有等待同步完成的锁阻塞本地查询；固定 CLI 版本的实际并发读写行为须在实现阶段验证。

`upsert/delete` 不通过查询等待预算降级：必须观察同步成功后，才能定位并执行远端写操作；同步失败或尚未完成时不得依据过期数据写入。

### 4.3 离线／同步未完成时的读取

| 同步结果 | `list/get` 行为 |
| --- | --- |
| 本次等待观察到同步成功 | 使用本地库读取，不附加过期标记。 |
| 明确的网络故障 | 本地库可用时允许读取，并附加过期标记。 |
| 查询等待预算耗尽，同步仍在进行 | 本地库可用时允许读取，并附加过期标记；同步继续。 |
| 认证失败、TLS 证书错误、解锁失败、解析或本地数据异常 | 返回对应错误，不回退。 |
| 无法可靠归类的同步失败 | 返回错误，不猜测为网络故障。 |

这里的网络故障指 Agentic 所在机器到 Vaultwarden 的 DNS 解析、连接或传输故障；不包括调用方到 Agentic 的断线。HTTP 429、5xx 不直接等同网络故障。

使用旧数据返回结果时，领域响应额外包含：

```json
{
  "stale": true,
  "lastSyncedAt": "2026-10-24T02:00:00Z"
}
```

- 仅在明确网络故障或同步等待预算耗尽而使用本地数据时附加这两个字段；正常结果不返回 `stale: false`。
- `lastSyncedAt` 直接读取 `bw` 记录的最后同步时间，不用本次触发时间或 Agentic 的当前时间代替。无法取得时间时不得伪造时间；空值及元数据读取失败的准确投影需在固定版本联调时落实。
- 不限制本地数据的最长年龄。远端在断网期间新增、修改或删除的内容可能尚未反映在本地库，过期标记不能保证数据仍然有效。
- 本地库仍须可读取、可解锁；数据异常直接报错，不从自建快照恢复。
- 过期读取的未命中错误也应携带相同的新鲜程度信息，不得把旧数据未命中报告成远端不存在。

## 5. `room.secret.list`

请求示例：

```json
{
  "filter": "github",
  "field": "site"
}
```

- `filter` 可选，执行大小写不敏感的包含过滤。
- `field` 可选，为 `site | account`，仅存在 `filter` 时生效。
- 提供 `filter` 但省略 `field` 时，同时匹配 `site/account`。过滤的大小写规则不改变 `account` 精确定位时区分大小写的规则。
- 省略 `filter` 时列出全部账号组，按使用权重排序。
- 不提供 cursor、页数、显式排序或调用方可调的权重参数。

响应示例：

```json
{
  "items": [
    {
      "site": "github.com",
      "account": "agent@example.invalid"
    }
  ]
}
```

列表项只返回 `site/account`，不返回 item ID、密码、seed、验证码、字段是否存在、权重或使用时间。过期结果按第 4 节附加元数据；过期空列表仅表示本地没有匹配项。

无过滤时依次按使用权重降序、最近成功使用时间降序、`site/account` 稳定排序。有过滤时先按完全匹配、前缀匹配、包含匹配区分匹配质量，再使用上述顺序。

完成过滤和公开结果裁剪后，若完整响应超过容量，返回 `secret_result_too_large`，提示增加或收紧过滤。不分页、不静默截断。具体容量与实际入口限制在实现阶段确定。

`list` 不请求确认。

## 6. 使用权重

权重由 Agentic 本地维护，不写入 Vaultwarden 字段，不进入可 Git 提交或同步的 Room 资料目录。

- 每次 `get` 成功生成响应中的密码或当前验证码后增加权重并记录时间；这不表示调用方已经成功登录目标服务。
- 权重随时间衰减，近期经常使用的条目优先；公式、半衰期属于实现选择，不对外暴露。
- 更新已有条目保留权重，新条目初始权重为零。
- 删除明确成功后清理权重；后续成功同步发现外部删除时清理孤立记录。不能根据失败同步或不完整本地结果推断远端删除。
- 记录按后端、账号与 item ID 关联，避免切换后端账号后继承无关权重。
- 权重损坏、缺失或写入失败不影响密钥读取及远端写入事实。排序退化为仍可用的时间信息及稳定名称顺序；全部记录丢失时，最近使用时间也可能不可用。

## 7. `room.secret.get`

请求示例：

```json
{
  "site": "github.com",
  "account": "agent@example.invalid",
  "type": "password"
}
```

`site/account/type` 必填；`type` 为 `password | totp`。

响应示例：

```json
{
  "value": "<PASSWORD_OR_CURRENT_CODE_PLACEHOLDER>"
}
```

- `password` 返回密码；`totp` 通过 `bw` 返回当前验证码，不返回 seed，也不重放之前生成的验证码。验证码不保证调用方稍后提交时仍未过期。
- 同步成功后账号组仍不存在，返回 `secret_not_found`；账号组存在但请求类型没有值，返回 `secret_value_missing`。
- 同步网络失败或等待预算耗尽而使用本地数据时，未命中返回 `secret_presence_unknown`，不能断言远端不存在。仅凭过期数据也不能确定远端缺少所请求字段。
- 重复账号组返回 `ambiguous_secret`。
- 成功返回后更新权重；附属权重保存失败不把成功读取改报为失败。

`getPolicy=confirm` 时，仅明确批准后才能向调用方披露值。拒绝、超时、确认不可用均不得披露，也不得增加权重；不继承普通命令在显式确认不可用时回落到 `allow` 的行为，不受全局命令 `policy.enabled` 绕过。

`getPolicy=allow` 不请求确认。响应丢失后必须重新调用并按当时策略授权；`confirm` 重新确认，`allow` 仍直接允许。不得从持久回执取回原来的密钥或验证码。

## 8. `room.secret.upsert`

请求示例：

```json
{
  "site": "github.com",
  "account": "agent@example.invalid",
  "password": "<PASSWORD_PLACEHOLDER>",
  "totp": "<BASE32_OR_OTPAUTH_TOTP_PLACEHOLDER>"
}
```

- `site/account` 必填；`password/totp` 至少提供一个。
- 同步成功后，身份不存在则创建，存在则更新；重复则返回 `ambiguous_secret`。
- 更新时省略字段保留原值。`bw edit` 是替换操作，适配器应读取并合并完整条目，保留其他已有内容，而不是只拼装四个字段后覆盖。
- 拒绝用空字符串或 `null` 清空字段。单字段清理由 Vaultwarden 客户端处理，第一阶段不增加清空语义。
- TOTP 输入须在写入前完成格式及受支持参数校验，不把 HOTP 等其他 URI 作为 TOTP 接受，不写入部分校验结果。
- 同一实例内协调“同步、定位、创建／更新”的并发，避免自身制造重复条目。外部客户端不受本地锁约束，不能承诺跨客户端的原子身份唯一性。

创建成功返回 `{"created": true}`，更新成功返回 `{"updated": true}`，不回显密钥、身份或底层 ID。

按 `upsertPolicy` 决定直接执行或严格确认。预览只展示 `site/account` 及涉及的字段类型，不展示实际内容。

读取、合并或写回失败不能返回成功；写请求发出后无法确定服务端结果，返回 `secret_write_outcome_unknown`，本次不自动重放。后续同步核对方式属于实现问题。

## 9. `room.secret.delete`

请求示例：

```json
{
  "site": "github.com",
  "account": "agent@example.invalid"
}
```

同步成功后精确定位一个账号组，将整个 Login item 移入 Vaultwarden 回收站，包括其中的密码和 TOTP。不使用永久删除，不提供恢复或按字段删除接口。

- 同步成功后仍不存在，返回 `secret_not_found`。
- 重复账号组返回 `ambiguous_secret`，不得批量删除。
- 服务端明确确认成功后返回 `{"deleted": true}`，并清理本地权重。
- 发出后连接中断等情况使结果无法确定时，返回 `secret_write_outcome_unknown`，不清理权重、不自动重放。

按 `deletePolicy` 决定直接执行或严格确认。预览展示 `site/account`，不展示密钥。

## 10. 配置与运行时准入

拟议配置示例；`enabled: true` 表示使用者主动启用，缺省仍为 `false`：

```json
{
  "room": {
    "secret": {
      "enabled": true,
      "serverUrl": "https://vault.example.invalid",
      "account": "agent@example.invalid",
      "clientId": "file:~/.agentic_gpt/secrets/bitwarden-client-id",
      "clientSecret": "file:~/.agentic_gpt/secrets/bitwarden-client-secret",
      "masterPassword": "file:~/.agentic_gpt/secrets/bitwarden-master-password",
      "getPolicy": "allow",
      "upsertPolicy": "confirm",
      "deletePolicy": "confirm",
      "syncWaitTimeoutMs": 3000
    }
  }
}
```

| 字段 | 缺省／约束 |
| --- | --- |
| `enabled` | `false`。 |
| `getPolicy` | `allow`；只允许 `allow | confirm`。 |
| `upsertPolicy` | `confirm`；只允许 `allow | confirm`。 |
| `deletePolicy` | `confirm`；只允许 `allow | confirm`。 |
| `syncWaitTimeoutMs` | `3000` 毫秒；仅限制查询等待，不取消同步。合法范围在实现阶段确定。 |
| 三个凭据字段 | 接入现有 `env:`／`file:` secret reference 校验和解析，不接受普通配置中的明文凭据。 |

生效条件同时包括 Room toolset 开启和 `room.secret.enabled=true`。当前仅有 `room` namespace；不为本功能新增嵌套 toolset 体系。关闭 Secret 子开关只关闭 Secret；关闭 Room toolset 关闭所有 Room 工具。除发现时隐藏外，已知工具名直接调用也必须在运行时拒绝。

配置需接入现有稀疏写盘、校验、配置 CLI、TUI 草稿／Review、字段隐藏和工具描述符。所有写者沿用配置变更锁和备份写入 helper。

第一阶段沿用当前 `room` 整节的重启生效边界，界面明确提示；不引入会话热切换或凭据热重建。所有面均须正确接入 Room profile／能力准入，不能只依据名称前缀分类。

## 11. 登录、解锁与本地状态

1. 使用 `BITWARDENCLI_APPDATA_DIR` 指向 Agentic 专属的私有持久目录，配置指定 `serverUrl`，不复用操作系统用户已有的个人 CLI 状态。
2. 使用独立账号的 Personal API Key 登录；不能误用组织 API Key 或 Secrets Manager access token。
3. 使用主密码解锁取得 session key；API Key 不能代替主密码解锁。
4. 核对实际服务端和账号身份，不将错误配置、登录或解锁失败报告为密钥不存在。
5. session 仅在 Agentic 及受控子进程内存中使用，不写入日志、事件、配置、回执或进程历史，不设置到整个进程树的全局环境。
6. 凭据通过受控子进程环境等合适方式传递；写入载荷使用 stdin，不能把密码、seed、session 或仅经 Base64 编码的密钥载荷放进 argv。
7. 未登录、已锁定或明确的会话失效，可在受控边界内重新登录／解锁一次并协调并发；不把未知写入结果当作可安全重试的认证错误。
8. 退出或关闭时回收任务和子进程，丢弃 session 并尽力 lock。实际配置重启生效期间，磁盘修改不等同运行中能力已立即关闭。

本地密码库由 `bw` 加密持久化；Agentic 只额外保存权重、触发时间等必要元数据。CLI 状态目录可能包含身份及认证状态，整个目录均应私有保护，不把它当作普通配置、日志或 Room 资料提交。

## 12. 同步事件与 agent.info

### 12.1 完成事件

每个实际启动的同步任务最终成功或失败时，向现有事件通知通道写入一条简短事件，级别均为最低的 `low`。查询复用任务不增加事件，查询预算耗尽不产生失败事件。

消息示例：“密钥同步成功”“密钥同步失败：网络不可用”。失败正文只使用归一化的安全原因，不包含原始 stdout/stderr、凭据、密码、seed、验证码或 session。沿用现有事件保留和展示规则，不把 event 等同于新增桌面推送渠道。

### 12.2 状态摘要

在实际持有 Secret service 的实例 `agent.info` 中增加以下拟议结构，并同步各实际消费者：

```json
{
  "secret": {
    "sync": {
      "status": "syncing",
      "lastTriggeredAt": "2026-10-24T02:00:00Z"
    }
  }
}
```

- `status` 为 `syncing | failed | succeeded`；未触发时为 `null`。
- `lastTriggeredAt` 为最近实际启动的同步任务的触发时间；未触发时为 `null`。复用任务和完成任务都不修改该触发时间。
- 成功、失败只由同步任务的实际终态决定，不能依据查询等待是否超时推断。
- `lastTriggeredAt` 不等于读取结果中的 `lastSyncedAt`：后者来自 `bw` 记录的最后同步时间。
- `agent.info` 仅观察现有状态，不为查询摘要主动触发同步或泄露密码库内容。

## 13. 敏感内容、回执与错误

### 13.1 不持久化与不重放

`upsert.password/totp` 和 `get.value` 不得进入 Agentic 的 Debug、Tracing、日志、审计正文、事件、reporting、Hub 数据库、传输账本、待重发命令或测试快照。

现有普通工具上报和 Hub 可靠命令路径会持久化请求／结果，Secret 接入时必须在真实记录边界处理，不能只依赖工具描述或末端日志替换。

- 在交付链路中允许向调用 Agent 返回明文，调用方自己的模型上下文或会话保存由调用方负责。
- Agentic 可以持久化脱敏状态、错误码及 `created/updated/deleted` 等安全回执；不能用含密钥的请求或结果支持重放。
- `get` 明文响应断线丢失后重新调用，按当前策略重新授权，不从账本恢复旧值。
- 写操作结果不确定时不自动重放；明确未发出或服务端明确拒绝时仍使用对应错误，不把所有连接问题都归入结果未知。
- 错误正文不含原始 `bw` JSON、stdout/stderr 或敏感值。输出格式无法识别时失败，不从未知文本猜测密钥。

### 13.2 领域错误

至少需要归一化：

| 错误 | 含义 |
| --- | --- |
| `secret_not_configured` | 能力尚无可用配置。关闭工具时仍遵守各入口的不可用／准入错误契约。 |
| `secret_authentication_failed` | 登录或认证失败。 |
| `secret_vault_locked` | 密码库无法解锁或会话不可用。 |
| `secret_not_found` | 成功同步后仍找不到目标账号组。 |
| `secret_presence_unknown` | 使用过期本地数据未命中，无法确定远端是否存在目标。 |
| `secret_value_missing` | 成功同步后账号组存在，但请求字段没有值。 |
| `ambiguous_secret` | 公开身份对应多个条目。 |
| `secret_invalid_totp` | TOTP 输入或现有值不受支持／无法生成验证码。 |
| `secret_provider_unavailable` | provider 当前不可用，且无法按读取规则返回本地数据。 |
| `secret_confirmation_rejected` | 确认被明确拒绝。 |
| `secret_confirmation_unavailable` | 确认超时、不可用等情况，未取得明确批准。 |
| `secret_write_outcome_unknown` | 写请求已发出，但服务端成败无法确定；本次不自动重放。 |
| `secret_result_too_large` | 列表过滤后的完整响应超过容量，需收紧过滤。 |

各入口继续使用现有错误与 `events` 等封套；上述示例仅表示领域数据，不引入一套脱离现有入口的通用响应格式。

## 14. 实现接入与验证边界

实现沿用现有所有者和消费者，具体入口见[开发指南](development/README.zh-CN.md)：

- 协议 DTO／HubCommand、Agent 本地 schema／dispatch、共享本地 service、运行时准入、Hub HTTP／OpenAPI 和 Apps MCP。
- [配置加载与验证](../crates/agentic-gpt/src/config/config.rs)、配置 CLI／TUI／Review／脱敏和[运行时启动](../crates/agentic-gpt/src/runtime/startup.rs)。
- [agent.info](../crates/agentic-gpt/src/runtime/agent_info.rs)、[事件通道](../crates/agentic-gpt/src/storage/event_store.rs)、工具 reporting、[Hub 回执](../crates/agentic-gpt-hub/src/storage/runs.rs)及[Agent 传输账本](../crates/agentic-gpt/src/storage/transport_ledger.rs)。

同步后台服务参考现有 `EmailBackground`，复用 Tokio 执行基础，不为本功能新增通用任务框架，也不直接套用会捕获并持久化输出的 `ManagedProcess`。

验收重点是实际消费者可见行为：多账号精确定位、字段合并、特殊字符保真、TOTP 当前码、重复项拒绝、确认失败关闭、子开关直接调用拒绝、同步任务复用、预算到期继续运行、过期未命中、写入结果未知、容量超限及全链路无敏感记录。

新增或修改测试前按仓库规则评估长期保留价值与测试档位，优先复用现有具名覆盖；不按验收清单机械生成固定内容或映射测试。涉及真实子进程的验证使用隔离 HOME、XDG 和临时状态目录，停止并等待全部派生进程后再清理；不读取真实个人密码库。

本次可行性审查仅核对仓库、官方资料及上游源码，未连接实际 Vaultwarden 部署，也未完成固定 CLI／服务端版本的端到端验证。后续构建、编译、测试优先使用 `~/.local/bin/remote-build` 针对当前项目执行。

## 15. 实现阶段待落实

以下属于实现和验证选择，不改变上述已确认的产品边界：

- 固定 `bw` 版本、获取和升级方式，与实际 Vaultwarden 版本联调；核验冷启动、离线解锁、同步期间本地读取及 CLI 原生锁行为。
- 可靠识别网络故障、认证失败、数据异常及写入结果未知；未知错误失败关闭，不靠笼统的非零退出码允许回退。
- 合法输入长度、同步等待配置范围、CLI 输出和列表完整响应容量。
- `lastSyncedAt` 在 `bw` 无记录或读取失败时的投影，以及本地库是否可用的判断。
- TOTP 的算法、位数、周期等支持范围，URI 写入规则，以及 Unicode 站点比较的确定性规则。
- 权重衰减函数、持久化格式与损坏后的退化排序。
- 任务退出回收、终态事件去重、状态更新和实际入口错误／事件封套的接入。
- 所有接口面不持久化敏感请求／结果的具体派发与回执方案，及写入结果不确定后的同步核对方式。

## 16. 外部依据

- [Bitwarden Password Manager CLI](https://bitwarden.com/help/cli/)：独立状态目录、自托管、同步、条目操作和 TOTP。
- [Personal API Key 登录](https://bitwarden.com/help/personal-api-key/)：API Key 与主密码解锁的区别。
- [Bitwarden 本地数据存储](https://bitwarden.com/help/data-storage/)及[离线使用](https://bitwarden.com/help/using-bitwarden-offline/)：本地加密和离线只读行为。
- [Vaultwarden 项目](https://github.com/dani-garcia/vaultwarden)及[维护者对 Secrets Manager 的说明](https://github.com/dani-garcia/vaultwarden/discussions/7457)。
- [bw 同步实现](https://github.com/bitwarden/clients/blob/main/libs/common/src/platform/sync/default-sync.service.ts)、[条目读取](https://github.com/bitwarden/clients/blob/main/apps/cli/src/commands/get.command.ts)和[列表实现](https://github.com/bitwarden/clients/blob/main/apps/cli/src/commands/list.command.ts)。

上游主分支源码仅作为本次调查依据，不等同第一阶段最终选定的兼容版本。
