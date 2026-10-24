# Room 邮件使用指南

Room 邮件通过 IMAP 同步一个邮箱，通过 SMTP 发送纯文本邮件。六个 `room.email.*` API 使用 Agent 本地保存的邮件；调用列表不会临时触发收信，也不会将邮件标为已读。

Standalone 和 Local 需要启用 `room` 工具集。Hub Full 通过活动 Room Agent 路由这些操作，请求不带 `agentId`；Coordinator 不提供邮件操作。HTTP 路由为 `POST /v1/room/email/list`、`read`、`search`、`attachment`、`send` 和 `operate`。请求字段以 [OpenAPI](../openapi/hub.yaml) 和 [共享邮件请求类型](../crates/agentic-gpt-protocol/src/email.rs) 为准。

## 配置账户

先使用 `agentic-gpt config init` 创建配置，再通过全屏配置界面的邮件页填写账户，或将下面的 `email` 对象加入现有配置。示例中的服务器和邮箱均为占位符，必须替换；密码填写引用，不填写密码明文。

```json
{
  "email": {
    "enabled": true,
    "account": {
      "imapHost": "imap.example.invalid",
      "imapPort": 993,
      "imapTls": "implicit",
      "smtpHost": "smtp.example.invalid",
      "smtpPort": 465,
      "smtpTls": "implicit",
      "username": "user@example.invalid",
      "password": "env:AGENTIC_EMAIL_PASSWORD",
      "from": "user@example.invalid"
    },
    "syncIntervalSeconds": 60,
    "retentionDays": 90,
    "maxStorageBytes": 2147483648,
    "sendPolicy": "confirm",
    "trashPolicy": "confirm"
  }
}
```

启用邮件还需要启用 Room 工具集：

```bash
agentic-gpt config toolset enable room
```

运行 Agent 的环境中必须存在 `AGENTIC_EMAIL_PASSWORD`。也可以使用 `file:/absolute/path` 或 `file:~/…` 引用保存密码的文件。文件只保存密码或应用专用密码；不要把密码写进命令参数、日志或配置示例。此版本不提供 OAuth2 登录，也不创建或管理邮箱服务商的应用密码。

`implicit` 表示连接开始即使用 TLS；`starttls` 表示连接后强制升级为 TLS，升级失败就拒绝继续。两种方式均验证服务器证书，不支持明文认证或跳过证书验证。示例默认端口为 IMAP 993、SMTP 465；选择 STARTTLS 时，按服务商要求显式填写端口，例如 IMAP 143、SMTP 587。

| 配置键 | 默认值 | 范围或要求 |
| --- | --- | --- |
| `email.enabled` | `false` | `true` 或 `false`；启用时必须有完整账户。 |
| `email.account` | 无 | 单账户连接对象；可设为 `null` 清除，但必须先关闭邮件。 |
| `email.account.imapHost`、`smtpHost` | 无 | 服务器主机名；不带 URL 协议前缀、路径或用户名。 |
| `email.account.imapPort`、`smtpPort` | `993`、`465` | 1–65535。 |
| `email.account.imapTls`、`smtpTls` | `implicit` | `implicit` 或 `starttls`。 |
| `email.account.username` | 无 | 邮箱服务商要求的登录名。 |
| `email.account.password` | 无 | `env:NAME` 或 `file:PATH` 引用。 |
| `email.account.from` | 无 | 不含显示名的单个发件邮箱地址。 |
| `email.syncIntervalSeconds` | `60` 秒 | 10–86400 秒。 |
| `email.retentionDays` | `90` 天 | 1–36500 天。 |
| `email.maxStorageBytes` | `2147483648` 字节，即 2 GiB | 至少 1048576 字节，即 1 MiB。 |
| `email.sendPolicy` | `confirm` | `allow`、`confirm` 或 `deny`。 |
| `email.trashPolicy` | `confirm` | `allow`、`confirm` 或 `deny`。 |

通过 `agentic-gpt config keys --section email` 查询 CLI 支持的键。CLI 将整个账户作为一个 JSON 对象写入，避免将未填完的账户保存为有效配置；其余字段可单独设置：

```bash
agentic-gpt config set email.syncIntervalSeconds 120
agentic-gpt config set email.retentionDays 180
agentic-gpt config set email.maxStorageBytes 4294967296
agentic-gpt config set email.sendPolicy confirm
agentic-gpt config set email.trashPolicy deny
```

`email.account` 内任何连接配置变化都要求重启 Agent。`enabled`、同步间隔、保存天数、容量上限和两个策略由现有配置监听器应用，无需重启；新策略影响之后接纳的请求，已接纳请求保留当时的策略。关闭 Room 工具集或邮件开关会停止继续安排同步。无效配置不会替换运行中上一份有效配置。

## 收信、提醒与本地保存

Agent 运行期间后台同步 INBOX。首次启用时，先保存服务器当时已有邮件的分界，再导入历史邮件；这些历史邮件不会逐封产生新邮件提醒。正常重启沿用已保存进度，停机期间收到的新邮件仍产生提醒。首次导入中途重启也沿用第一次的分界。

同步同时更新邮件是否仍在 INBOX、是否已读及是否标星。其他客户端删除或移走邮件后，它不再出现在 `list` 的 INBOX 列表中；本地已经保存的副本仍可通过 `search` 和 `read` 查找，直到内容按保存天数或容量上限清理。容量按原始邮件与索引正文合计；达到保存天数或容量限制时，从较旧内容开始清理。清理删除原文和索引中的正文，保留元数据与短摘要；之后 `read` 和 `attachment` 返回 `email_content_expired`，不会把内容已清理误报为邮件从未存在。读取列表和正文均不会自动标为已读。

每封新邮件产生一条 `medium` 事件，只携带摘要和邮件引用。事件是提醒；标记事件已处理不会删除邮件，也不修改邮箱已读状态。列表和邮件本身应作为查看邮件的依据。

| 位置 | 内容 |
| --- | --- |
| `~/.agentic_gpt/state/email.sqlite3` | 邮件索引、同步进度、首次导入分界和发送状态。 |
| `~/.agentic_gpt/state/email/` | 邮件原文，包含正文及附件。 |
| `~/.agentic_gpt/cache/email/` | 可以重新生成的图片预览等缓存。 |
| `<workspace>/downloads/email/<mail-id>/` | 通过附件 API 显式导出的文件。 |

私有邮件状态不按 `agentId` 分目录，也不进入 Room 文档仓库的 Git 提交。保存天数和容量上限管理 Agent 内部邮件内容；已经导出到 workspace 的附件由用户管理，不随内部清理删除。

邮件 ID 由 Agent 生成。它与邮件头 `Message-ID` 或服务器内部编号不同，调用方应原样保存和传递。服务器改变邮件编号规则后，状态报告 `recoveryRequired`；旧 ID 仍可读取和搜索本地副本，但身份不确定的邮件不能继续执行远端状态操作；应先处理明确报告的恢复问题，不能根据相似标题猜测目标邮件。

## 恢复服务器邮件身份

`sync.state=recoveryRequired` 表示服务器更换了邮件编号规则，Agent 无法可靠确认旧记录对应哪封远端邮件。此时不会按标题猜测，也不会自动重新导入并替你决定是否提醒历史邮件。

先停止使用这份邮件状态的 Agent，再明确选择以下一种恢复方式。将 `PATH` 替换为实际配置文件路径：

```bash
agentic-gpt email --config PATH recover --notify-existing
```

该方式重新导入当前 INBOX，并为这些邮件产生提醒；或者：

```bash
agentic-gpt email --config PATH recover --skip-existing-notifications
```

该方式重新导入当前 INBOX，但不为恢复时已有邮件产生提醒。两个选项必须且只能选择一个；Agent 仍运行或邮件状态被其他实例占用时，恢复会被拒绝。命令需要连接 IMAP，失败时应先修复连接。

恢复保留旧本地 ID 和副本，解除它们与远端邮件的旧关联；后续重新导入生成新 ID。因此 `search` 可能同时找到历史副本和新导入记录，旧记录不能继续修改远端状态。恢复后启动 Agent，之后新收到的邮件按正常规则提醒。

## 使用六个 API

以下为工具参数示例，不是 shell 命令。HTTP 调用使用相同的 JSON 参数；MCP 的文本、结构化结果和图片内容按各入口的响应方式返回。六个 API 都复用现有响应中的 `events: {current, new}`，不使用单独的邮件响应封套；MCP 图片模式将图片放入原生图片内容块，同时保留文本和结构化结果中的 `events`。无法到达 Agent 的路由错误沿用现有规则，不附加事件面板。

### 列出 INBOX 邮件

调用 `room.email.list`：

```json
{
  "limit": 20,
  "filter": {
    "unread": true,
    "receivedAfter": "2026-10-20T00:00:00Z",
    "receivedBefore": "2026-10-25T00:00:00Z"
  }
}
```

不需要筛选时传 `{}`。`limit` 默认 20，范围 1–100。结果按接收时间从新到旧排序，只列当前 INBOX，不混入本地发送记录。日期使用带时区的 RFC 3339 时间，`receivedAfter` 包含该时刻，`receivedBefore` 不包含该时刻；接收时间来自 IMAP 服务器，而非邮件作者填写的 `Date` 头。

响应中的同步状态说明列表是否完整：`initialSyncComplete=false` 表示首次导入尚未完成；`state` 为 `syncing`、`ready`、`error` 或 `recoveryRequired`。有记录时还返回 `lastSuccessfulSyncAt` 和 `lastError`。初次同步未完成或同步失败时，不应把空列表理解为邮箱中没有邮件。

若响应给出 `nextCursor`，将它原样传回 `cursor` 继续读取。Cursor 是继续读取的位置标记，不需要解析；续页应保持相同筛选条件，可以调整 `limit`。没有总页数。

### 读取正文或图片

调用 `room.email.read`：

```json
{"id": "mail_返回的邮件ID", "view": "text"}
```

`view` 默认 `text`。有可用纯文本时返回规范化后的纯文本；只有 HTML 时转换为 Markdown，并在结果中标明实际格式。该转换用于阅读，不保证还原邮件的视觉布局。转换失败时保留原始 HTML，文本视图明确返回 `email_text_conversion_failed_use_image_view`，可改用图片视图；不会把转换失败当成空正文。正文每次最多返回 24 KiB，超过预算时用响应的 `nextCursor` 继续读取，并保持相同 `id` 和 `view`。

需要检查布局、表格或视觉内容时，改用：

```json
{"id": "mail_返回的邮件ID", "view": "image"}
```

`view` 有三种模式，续读时必须保持原模式：

| 模式 | 行为 |
| --- | --- |
| `text`（默认） | 读取纯文本或 HTML 转换后的 Markdown。 |
| `image` | 离线渲染图片分片；不请求任何远程资源。 |
| `imageWithRemote` | 渲染图片分片，并允许下载邮件引用的远程图片。只开放图片下载，不加载远程 CSS，不执行脚本或提交表单。 |

需要加载远程图片时，必须显式选择：

```json
{"id": "mail_返回的邮件ID", "view": "imageWithRemote"}
```

两种图片模式使用 `litehtml-rs` 渲染 HTML，不依赖项目的 Browser 功能，也无需安装 Chrome、Chromium 或 Python。构建时使用包内附带的 litehtml 源码，需要 C++17 编译工具链；运行设备需要适合邮件文字的字体，中文邮件应安装相应中文字体。字体缺失可能导致文字显示为方框，不应通过开启远程字体下载补救。

远程模式处理 `<img src>`、没有 `src` 时 `srcset` 的第一个候选，`body`、`table`、`tr`、`td`、`th` 上旧式 `background` 图片属性，以及渲染器支持的 style 属性和内联 `<style>` 图片。图片来源由渲染器在整封邮件排版、绘制时识别，包含后续分片的背景图片；不支持的 CSS 不会额外触发下载。所有来源合计使用同一份 16 张图片限额。

不会下载外链 CSS、`@import` 样式表、远程字体或 SVG。URL 对应内容必须是经过验证的 PNG、JPEG、WebP 或 GIF，不能用图片属性绕过格式限制；内嵌 data/CID 图片是否可用也受渲染器的格式和大小校验。

下载只允许公网 HTTP 80 和 HTTPS 443，重定向最多三次，每次重新检查目标地址；拒绝本机、内网、保留地址及带用户名或密码的 URL。每次最多处理 16 个不同图片来源，单图最多 4 MiB、下载合计最多 12 MiB；单资源处理最多 4 秒，下载阶段最多 12 秒，连同准备和缓存最多等待 15 秒。失败、被拒绝或超预算的图片保留占位，不暴露失败 URL 或凭据。

远程模式会将已验证的图片按原始来源保存在私有 `cache/email/remote-<内容标识>.json` 中，文件名中的标识按原始 HTML 内容计算；后续分片复用该文件，不会每翻一片重新联网。缓存保留 24 小时，总上限 64 MiB。旧版页面缓存可重新生成，首次读取会重建；更新前的图片 cursor 需从第一片重新读取。原始 HTML 上限 16 MiB；HTML 解析前限制标签起始符最多 100000 个，解析后限制节点最多 100000 个、深度最多 128 层。续读需要原缓存且图片内容保持一致，缓存失效或图片内容变化时返回 `email_render_cache_expired`，应去掉 cursor 从第一片重新读取；第一次读取也可复用仍有效的缓存。`blockedRemoteImages` 报告被阻断或无法加载的不同图片来源数，包含未被识别或未成功加载的远程 CSS 图片引用。

图片按纵向分片返回，不会执行邮件脚本、导航或提交表单；`imageWithRemote` 也不放开远程样式表或其他主动内容。`image` 仍可使用已解析的安全内嵌图片。邮件仅引用外部图片时，离线模式不会下载它们。

正文响应还列出附件的 `id`、`name`、`mimeType` 和 `size`；读取正文不会同时返回附件内容。规范化正文和附件清单在本地保存，text 续读不重新解析原始邮件或解码附件。旧版本地记录会自动补充附件清单，首次读取时解析一次；不会批量重新导入邮件或产生提醒。图片读取和附件读取只解析各自需要的内容。

### 搜索邮件或一封邮件的正文

在本地已同步内容中调用 `room.email.search`：

```json
{"query": "账单", "field": "all"}
```

`field` 可为 `all`、`subject`、`body`、`from`，默认 `all`。返回的 `items` 使用 `ref` 指向邮件 ID，`text` 为主题。搜索包含本地收到的邮件，以及本地记录的 `sent` 和 `unknown` 发送邮件；列表仍只显示 INBOX。

只搜索一封邮件正文时提供 `id`：

```json
{"id": "mail_返回的邮件ID", "query": "付款日期"}
```

此时任何 `field` 都按 `body` 处理。命中项的 `ref` 是可以交给 `read(view=text)` 的正文 cursor，`text` 是命中附近内容。搜索使用不区分大小写的字面子串匹配，不执行正则表达式；没有结果分页，最多返回 100 项且共用 24 KiB 内容预算。超出预算时返回 `truncated: true`，应缩小查询范围。

### 读取或导出附件

调用 `room.email.attachment`，使用正文响应中的附件 ID：

```json
{"id": "mail_返回的邮件ID", "attachmentId": "attachment_1"}
```

未设置 `download` 和 `path` 时，安全且大小合适的图片返回图片内容，小型 UTF-8 文本返回文本。PDF 和其他需要外部程序解析的文件，以及超过内联预算的附件，需先导出：

```json
{
  "id": "mail_返回的邮件ID",
  "attachmentId": "attachment_1",
  "download": true,
  "path": "downloads/email/report.pdf"
}
```

`path` 始终相对于当前 workspace；提供它就表示下载，即使省略 `download` 或传入 `false`。不传 `path` 时使用 `downloads/email/<mail-id>/<附件文件名>`。目标不能是绝对路径，不能通过 `..` 或符号链接逃出 workspace；自动创建父目录，拒绝覆盖已有文件。保存仍受 Agent 文件写入权限检查，成功只返回实际的 workspace 相对路径，可交给 `file.read` 等工具。

### 发送纯文本邮件

调用 `room.email.send`：

```json
{
  "to": ["recipient@example.invalid"],
  "subject": "会议安排",
  "text": "会议时间为明天上午十点。"
}
```

`to` 至少一个地址；`cc`、`bcc` 可省略。发件人固定使用账户的 `from`。第一版只发送纯文本，不发送 HTML 或附件。

回复邮件时，可增加 `replyToId`，由 Agent 从原邮件建立回复头；收件人和主题仍由调用方明确填写。它不自动推断回复对象。

发送前先建立本地发送记录。记录的 `status` 区分 `pending`（正在处理）、`sent`（服务器已接受）、`unknown`（结果不确定）、`failed`（明确失败）和 `denied`（策略或确认未获准）；收件记录为 `received`。`sent` 表示 SMTP 服务器已接受，不等于最终送达收件人；`unknown` 表示无法确认是否已被服务器接受。两者均可由 `search` 找回，并通过 `read` 查看内容和发送状态。重启保留不确定状态，不会自动重新发送。

**发送响应丢失或结果为 `unknown` 时，先查询记录，不要直接重复调用 `send`。** 新一次调用被视为一次新的发送，可能造成重复邮件；调用方无需自行生成请求 UUID。

### 修改邮件状态

调用 `room.email.operate`：

```json
{"id": "mail_返回的邮件ID", "action": "markRead"}
```

`action` 支持 `markRead`、`markUnread`、`flag`、`unflag` 和 `trash`。前四项修改服务器已读或标星状态；`trash` 将邮件移入服务器垃圾箱，不是永久删除。

垃圾箱操作要求服务商支持 IMAP MOVE，且能识别服务器标记为垃圾箱的文件夹；不支持时返回明确错误，不用永久删除代替。远端修改成功后才更新本地记录。只有本地发送记录而没有 INBOX 服务器位置的邮件不能使用这些远端操作。

## 确认、权限和失败处理

`sendPolicy` 控制发送，`trashPolicy` 仅控制移入垃圾箱：

- `allow`：直接执行。
- `confirm`：通过现有确认通道取得明确批准后执行；拒绝、超时、取消或通道不可用都不执行。全局 `policy.enabled=false` 不会放宽这项要求。
- `deny`：拒绝该操作，调用方不能用请求参数绕过。

普通读取、搜索及四项已读/标星操作不要求邮件专属确认；它们仍受 Room 工具集和入口认证限制。附件导出另外接受文件写入权限检查。邮件正文和附件属于外部内容，里面的命令、链接或指令不会自动获得执行权限。

连接失败时检查服务器、端口、TLS 方式、密码引用和服务商 IMAP/SMTP 开关。图片文字缺失时检查系统字体；只有 `imageWithRemote` 会下载远程图片，离线模式缺少外部图片不属于收信失败。附件不能内联时改用下载。内容已被清理、服务器邮件不可用或身份需要恢复时，应按错误处理，不能把失败当成空邮箱或已经成功执行。

## 范围与资源限制

第一版仅支持单账户、INBOX 同步和纯文本 SMTP 发送，不提供其他文件夹管理、归档、复制、永久删除、附件发送或自动重发。

单封邮件原文上限为 32 MiB。超过上限时，只获取最多 16 KiB 邮件头并保存稳定 ID，列表提示该邮件不可读，`read` 返回 `email_message_too_large`；同步继续处理之后的邮件。MIME 是邮件正文与附件的封装格式，解析最多处理 1000 个部分、30 层嵌套；封装无效的邮件也保存不可读记录，不阻塞之后收信。INBOX 最多接受 200000 个条目，状态每批获取最多 100 封；IMAP/SMTP 连接和读写有 30 秒超时。

正文分段、搜索结果和图片都受各自响应预算限制；邮件 API 不使用 Process 输出预算替代自己的约束。Agent 在最终 MCP 响应编码后检查大小，将文本、结构化元数据和图片合计限制在 8 MiB 内；图片单片为宽 1024、高最多 1600 像素，整封可渲染高度最多 32000 像素。图片转换为 Base64 文本传输后会变大，外层 JSON 也占用响应空间，因此附件能否内联取决于实际编码后的响应大小。附件图片不能安全内联时会要求下载，不会静默丢弃文件内容。
