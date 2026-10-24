以下为实现前的审查记录；实施结果和验收见 progress.md。

原设计第13节明确email-lib与拆分库为待选项；当前实际imap+lettre+mailparse+htmd。核心2272行由mod906/store645/network419/content302组成。现有规划没有保留email-lib与拆分库的比较依据，需要本次实证。

独立代码复核已定位可精简候选：核心重复MCP体积估算约25行；私有Request与protocol已有DTO重复约45–65行。需保留最终入口实际编码预算检查。
官方库复核进行中：email-lib0.27.0通用同步使用Maildir双侧缓存；SMTP封装可能含自动重试，与unknown不重发的要求冲突，正在核实是否可关闭。

email-lib核实补充：SMTP高层无retry关闭配置，但公开低层client可以绕开，因此不能说绝对不能用；绕开后需要自己拼装信封并处理结果。通用同步可配置不回写远端/不删除本地，但按Message-ID聚合，与原设计允许Message-ID缺失或重复冲突；不能直接替换以(account,UIDVALIDITY,UID)定位的本地持久记录。

运行接入1090精确拆分：283行请求DTO/HubArgs/LocalSchema，117行wire及路由映射，385行六组入口转发，共785行声明/转发；剩余305行包括104行图片/错误结果投影和201行确认/启动/恢复CLI/事件幂等/依赖。可收敛最明确为HubArgs复制公共请求并JSON来回转换（约110–125行），可用共享DTO可选schema derive；HTTP已有forward_room_command，MCP已有request_active_room，不应为减行数新增router宏。
图片拆为renderer369和remote839；后者DOM/CSS约326，下载地址166，磁盘缓存215，编排129。原项目无可直接安全复用的任意公网URL下载helper，现有skills/tunnel/browser下载各有不同约束；共享image解码已复用。需要核对库自带URLfetcher能否减少DOM/CSS自写扫描。

最终判断：
- 核心主体与已确认的本地索引/首次分界/恢复/提醒/发送未知态一致；不是重写协议栈。可收敛约70–100行重复公共请求声明和MCP预算估算。
- 接口1090行中785为声明和转发，104为结果投影，201为运行支持。HubArgs重复公共类型及JSON往返预计110–125行可去；Local单结果图片helper预计15–25行可共用。合计低成本精简估计约200–250行，尚未实际改动验证。
- 图片369行里render_document239–329约91行承担库排版/绘制/图片回填/裁片/PNG编码；其余是cursor、进程与资源限制。remote839中约325行自写HTML/CSS收集和改写、167行网络、215行缓存，其余编排。litehtml0.2.6官方发布源码提供url_fetcher与take_pending_images/load_image_data；可改由库发现图片来源，减少手写预处理，但自带预处理只找src，CSS背景仍需绘制回调，不是一行替换。跨重启24h磁盘缓存、多层锁是当前实现选择，不能都说成需求必需。取消后仍在运行的blocking磁盘任务需在删锁前保持并发保障。
- 官方库来源：email-lib0.27.0提交7b43fa6b4620eb6fdef87ee54426e3cac6d70410；litehtml0.2.6发布包提交1f7c8422d36d07e4e10c4e59b37814ee9e128645。Context7没有Rust包装库条目，实际核对官方源码。
- 静态审查额外发现：text每页经read_content重复读EML/解码所有附件/转换Markdown；首个空白plain会遮住后续有效plain；Hub read外部资源提示与Local不一致。未运行复现或改代码，不将这些当测试已验证缺陷。
