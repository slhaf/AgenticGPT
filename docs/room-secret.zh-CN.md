# Room 密钥使用指南

Room Secret 通过 Agent 本机的 Bitwarden Password Manager CLI（`bw`）访问独立 Vaultwarden 账号，提供账号组列表、密码／当前 TOTP 读取、创建／更新和移入回收站。它不提供网页登录、密码生成或永久删除，也不把密钥保存到 Room 文档仓库。

本页适用于包含 `room.secret.*` 的构建，说明当前代码的配置与调用方式。[设计文档](room-secret-api-design.md)记录决策和实现依据；已验证的 CLI／服务端组合和范围见文末。共享请求类型见 [secret.rs](../crates/agentic-gpt-protocol/src/secret.rs)，HTTP schema 见 [OpenAPI](../openapi/hub.yaml)。

## 准备与启用

1. 在运行 Agent 的机器准备 `bw`，并使 Agent 的 `PATH` 能找到它。已联调版本为 [`bw` **2026.10.0**](https://github.com/bitwarden/clients/releases/tag/cli-v2026.10.0)；适配器不按版本号拒绝调用，也不自动下载 CLI。仅在命令不可用、执行失败或bw 命令输出缺少当前操作必需的数据时返回对应错误；额外字段不影响使用。Agentic 不直接检查 Vaultwarden 的 HTTP 响应字段。
2. 准备 Vaultwarden 服务端，已联调版本为 **1.37.4**。与上述 CLI 配合时，**1.35.1** 的 Personal API 登录响应缺少 CLI 要求的 `userDecryptionOptions`，无法完成初始化。服务端也不按版本号设门禁；其他组合未验证，不据此声明最低兼容版本。准备专用账号的 Personal API client ID、client secret 和主密码。API Key 用于登录，主密码用于解锁；不要填组织 API Key 或 Secrets Manager access token。
3. 用运行 Agent 的环境或私有文件提供三个凭据。`env:NAME` 指向 Agent 进程可读的环境变量；`file:PATH` 指向只保存对应凭据的文件，支持绝对路径或 `~` 展开。文件应由运行账号私有保护，不放进 workspace、日志或 Git。普通配置不接受凭据明文。
4. 在已有配置中启用 Room 工具集，再填写连接设置，最后启用 Secret：

```bash
agentic-gpt config toolset enable room
agentic-gpt config set room.secret.serverUrl https://vault.example.invalid
agentic-gpt config set room.secret.account agent@example.invalid
agentic-gpt config set room.secret.clientId env:AGENTIC_BW_CLIENT_ID
agentic-gpt config set room.secret.clientSecret env:AGENTIC_BW_CLIENT_SECRET
agentic-gpt config set room.secret.masterPassword env:AGENTIC_BW_MASTER_PASSWORD
agentic-gpt config set room.secret.enabled true
```

服务地址和账号是不可用的示意值，须替换。三个环境变量须由服务管理器或当前运行环境注入，命令只填写引用。若使用文件，将引用改为 `file:/absolute/path`，例如 `file:~/.agentic_gpt/secrets/bitwarden-client-id`。当前服务地址接受 HTTP(S)，不接受 URL 中的用户名、密码、查询参数或片段。

也可在 `agentic-gpt config init` 的 Room 页填写上述字段，再在 Review 编辑和提交。配置显示、Review、JSON 预览和 Debug 隐藏服务地址、账号与凭据引用。完整默认值和长度要求见[Room 密钥配置](configuration.zh-CN.md#room-密钥配置)。

`room.secret.getPolicy`、`upsertPolicy`、`deletePolicy` 与 `syncWaitTimeoutMs` 支持热更新，配置重载后用于新请求。已接纳的请求保留原策略和等待预算；调整预算不会取消正在运行的同步。启停、服务地址、后端账号和凭据引用变更通过局部服务协调：先排空请求，再停止旧 CLI／同步、释放 session 与账户锁，之后允许新配置激活 owner。排空繁忙会保留旧实例并报告应用失败；保存成功不保证远程鉴权可用，也不会自动重放密钥写入。实际生效状态见[配置生效边界](configuration.zh-CN.md#热加载与重启边界)。`room` 工具集使用既有实时准入：关闭它会拒绝后续全部 Room 操作。Secret 子开关只控制四项密钥操作；关闭时不会影响其他 Room 工具，也不能通过直接调用已知工具名绕过。

Agent 本地 MCP 在 Secret 关闭时隐藏四个工具。Hub Full MCP 沿用固定工具列表，未启用时实际调用返回错误。Standalone／Local 从本地 MCP 入口调用；Hub Full 路由到活动 Room Agent，HTTP 使用 `POST /v1/room/secret/list`、`get`、`upsert`、`delete`，请求均不含 `agentId`。Coordinator 不提供这些操作。

## 账号组与调用

一个 Login item 的 `name` 对应 `site`，`login.username` 对应 `account`。输入先去除首尾空白；站点按 Unicode 小写比较，账号区分大小写。两者都必须非空、不含控制字符，且各不超过 1024 字节。`site` 是名称，不会自动把完整 URL、路径、子域名或公共后缀合并成同一站点。重复身份返回 `ambiguous_secret`，不猜测目标。

| 工具 | 请求与结果 |
| --- | --- |
| `room.secret.list` | `{}` 列全部账号组；可提供 `filter`，以及仅在过滤时有用的 `field=site/account`。返回项仅有 `site/account`，没有底层 ID、密码、seed、权重或字段存在标记。 |
| `room.secret.get` | 必填 `site/account/type`；`type=password` 返回密码，`type=totp` 生成当前验证码；成功返回 `value`。验证码可能在调用方提交前过期。 |
| `room.secret.upsert` | 必填 `site/account`，至少提供非空 `password/totp` 之一。不存在则创建，存在则合并更新，省略字段保留原值；成功只返回 `created:true` 或 `updated:true`。 |
| `room.secret.delete` | 必填 `site/account`；将整个 Login item 连同密码和 TOTP 移入回收站；成功只返回 `deleted:true`。恢复和单字段清理由 Vaultwarden 客户端处理。 |

例如，读取指定账号的当前验证码：

```json
{
  "site": "github.com",
  "account": "agent@example.invalid",
  "type": "totp"
}
```

`list.filter` 最多 1024 字节，执行不区分大小写的包含匹配；未指定 `field` 时匹配两列。无过滤时按衰减使用权重、最近成功使用时间及稳定名称排序；有过滤时先比较完全／前缀／包含匹配质量。成功 `get` 才增加使用权重；这不代表调用方已成功登录。列表不分页、不静默截断，结果超过 512 KiB 时返回 `secret_result_too_large`，需收紧过滤。

`upsert.password/totp` 各最多 64 KiB；空字符串和显式 `null` 均拒绝，不能用它们清空字段。TOTP 接受 Base32 seed 或 `otpauth://totp/…` URI；当前支持 SHA1／SHA256／SHA512、6 或 8 位验证码、30 秒周期。HOTP、重复或未知 URI 参数及不受支持的设置返回 `secret_invalid_totp`。读取 TOTP 不返回 seed。

账号组以 `bw list items` 返回的可解密条目为准。沿用 bw 对异常条目的容错行为：条目损坏或密钥不匹配等解密异常可能导致条目被省略且命令成功退出。这属于异常条目的容错，并非正常读写步骤，也不是导入专用分支。Agentic 不读取加密缓存来核对完整性，也不另存快照。因此成功同步后的 `secret_not_found` 只表示可读列表未命中；upsert 可能为被省略的条目创建同名账号组，重复检查也只能覆盖可见条目。成功同步后的权重清理同样依据该列表，不能作为远端已删除的证明。相关原生行为见[固定版解密结果处理](https://github.com/bitwarden/clients/blob/cli-v2026.10.0/libs/common/src/vault/services/cipher.service.ts)。

## 确认与同步

默认 `getPolicy=allow`，`upsertPolicy=confirm`，`deletePolicy=confirm`；三者仅接受 `allow` 或 `confirm`。`list` 不请求确认。`confirm` 必须取得明确批准：拒绝、超时、确认不可用都不会披露值或执行写入，`policy.enabled=false` 也不会放宽此要求。预览只展示 `site/account` 与涉及的字段类型。

每次调用先触发同步；同一实例已有同步任务运行时复用它，不重复启动。`list/get` 的 `syncWaitTimeoutMs` 默认 3000 毫秒，范围 0–30000，0 表示不等待。预算不包括人工确认，也不是整个 API 的响应时限；本地 CLI 读取另有 20 秒执行边界。预算耗尽后，同步任务继续运行。

| 观察到的同步结果 | 查询行为 |
| --- | --- |
| 同步成功 | 使用本地密码库，不增加过期字段。 |
| 明确网络故障，或查询等待预算耗尽 | 本地库仍可读取与解锁时返回旧数据，并附加 `stale:true` 和 `lastSyncedAt`。 |
| 认证、TLS、解锁、解析、数据异常或无法可靠归类的错误 | 返回错误，不根据这些错误回退到旧数据。 |

`lastSyncedAt` 来自 `bw status.lastSync`，没有同步记录时为 `null`；不能把 Agent 的触发时间当作最后成功同步时间。无法读取有效同步元数据时返回错误。旧数据不设最长年龄限制，可能尚未反映远端新增、修改或删除；旧的可读列表未命中返回 `secret_presence_unknown`，不能据此判定远端不存在。成功同步后的可读列表未命中为 `secret_not_found`，组存在但所请求字段缺失为 `secret_value_missing`。

`upsert/delete` 必须观察同步成功，不能按查询等待预算降级写入。同一实例协调同步、定位和修改；其他客户端仍可能并发修改，不承诺跨客户端的原子身份唯一性。写请求已发出而无法确定远端结果时返回 `secret_write_outcome_unknown`，不自动重放；应先通过 Vaultwarden 客户端或后续同步核对，避免盲目重发。

每个实际启动的同步任务在完成时产生一条 `low` 事件；请求复用任务或等待预算耗尽不会另外产生失败事件。`agent.info.secret.sync` 提供 `status`（未触发为 `null`，否则为 `syncing/failed/succeeded`）与 `lastTriggeredAt`。查询状态不触发同步；`lastTriggeredAt` 与密码库的 `lastSyncedAt` 含义不同。

## 本地状态与敏感内容

CLI 状态位于 `~/.agentic_gpt/state/secret/<后端账号摘要>/`，由 `BITWARDENCLI_APPDATA_DIR` 指向私有目录；不复用个人 `bw` 状态，不进入 Room 资料仓库。密码库由 `bw` 加密持久化，Agentic 另存使用权重。整个目录还含身份和认证状态，应按私有凭据资料保护。

密码、TOTP seed、验证码和 session 不进入 Agentic 的日志、Debug、普通工具 reporting、Hub 持久运行回执或 Agent 重放账本。四项操作不使用普通可靠命令的持久重放路径，run 查询不能恢复旧密码或验证码。凭据和 session 通过受控子进程环境传递，写入载荷通过 stdin 传递；不把这些内容放入 argv。

`get` 响应丢失后需重新调用，并按当时策略重新授权；`confirm` 会重新确认。Agentic 会向调用方返回明文值，调用方自己的模型上下文和会话保存由调用方管理。退出时回收同步任务和 CLI 子进程，丢弃内存 session 并尽力锁定密码库。

2026-10-24 已用隔离账号联调 `bw 2026.10.0` 与 Vaultwarden `1.37.4`：Personal API 登录、解锁、stdin 创建及完整条目合并、特殊字符保真、当前 TOTP、原生同步时间、同步期间本地读取、断网后锁定再解锁并读取缓存、删除进入回收站均已通过。测试使用一次性服务端和私有状态目录，不使用个人密码库。 同时通过实际 Agent 本地 MCP 验证了四项操作、当前 TOTP、区分账号大小写、省略字段保留、等待预算耗尽后后台同步继续成功、离线过期读取及未命中、离线写入拒绝、Agent 重启后离线解锁读取；被扫描的 Agent 状态和日志未发现测试密码、seed 或验证码。

网络故障分类取决于 CLI 实际输出。已验证单地址拒绝连接可识别并回退；localhost 双地址失败可能只留下不含具体原因的聚合错误，此时按无法可靠归类的失败报错，不强行降级为过期读取。
