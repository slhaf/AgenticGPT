# 发现
用户bin remote-build与仓库scripts/remote-build.sh不同，必须用前者。前者rsync到远端cache，在远端--locked执行check/build/test，release交叉编译aarch64并取回dist/remote。test隔离HOME，但新smoke还需XDG/runtime/TMP隔离与进程清理。

## 图片方案待用户选择
用户明确图片视图不得依赖项目Browser功能，并要求先检索可行方案、由用户选择。暂停renderer实现，未添加任何Chrome依赖或渲染代码。
- litehtml-rs官方README有email/pixbuf feature，CPU直接RGBA，email预处理支持内嵌图与表格/旧式邮件属性，远程资源默认关闭。候选最贴合项目；Rust绑定较新，样本与性能未测。
- Blitz提供独立HTML/CSS与CPU图像路径，当前官方主仓与about状态不同，table-collapse等表格实现有限制，需样本验证。
- WeasyPrint独立HTML/CSS排版成熟，依赖Python/Pango，当前主输出PDF，需额外PDF转PNG，集成链更长。
- Takumi现成HTML转换会丢style/head/link，不宜直接用于原始邮件，不能只凭支持HTML就推荐。
其他实现已落盘，但尚未远程编译；remote-build sync远端步骤尚未返回，原因未核实；未终止任何远端任务。Cargo.lock通过cargo update --workspace只增加新依赖、保留原有版本。

### Email core work in progress (2026-10-24)

- Added core owner/content/store/network and 3 tier-⑤ tests, plus imap/imap-proto/lettre/mailparse/htmd/webpki-roots dependencies. No Rust compilation, test execution, real mailbox or SMTP smoke has run. Cargo.lock still needs remote dependency resolution. Treat all new core code as unverified.
- Existing confirmation_content collapses automatic_reason to 100 characters. Email sends supply To/Subject and a body preview, but desktop reason truncation can omit required information. A bounded email-specific display path through existing confirmation providers remains required. Do not weaken strict Confirm behavior.
- Body storage cleanup currently runs after a full mailbox sync. Must enforce retention/capacity while importing (without scanning the full index for every message), otherwise a large first import can exceed configured capacity transiently.
- First connection errors are now persisted with accounts.validity=0 as an uninitialized sentinel; successful SELECT persists its true baseline before import. Need real SQL/reopen verification.
- UIDVALIDITY changes preserve old records and pause remote operations. A user-invoked recovery mechanism/diagnostic procedure has not yet been implemented.
- SMTP pending records are persisted before confirmation/network and become unknown after reopen. A pre-network worker error currently returns a generic error without its new ID, although the record remains searchable; assess send result handling during compile/review.
- TLS uses existing rustls with WebPKI trust and explicit ring provider, with a tiny bounded STARTTLS pre-login handshake to impose TCP/IO deadlines; all mailbox commands use imap library. Need real TLS/STARTTLS execution. Parent is researching a standalone HTML mail renderer separately; do not assume project Browser availability.
