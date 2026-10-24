阿里云官方配置：https://help.aliyun.com/zh/document_detail/36576.html；IMAP imap.qiye.aliyun.com:993，SMTP smtp.qiye.aliyun.com:465，均使用TLS并验证证书。
本机aarch64；通过用户~/.local/bin/remote-build release构建当前工作区。旧dist二进制早于本次收敛，不能用于最终验收。
当前执行环境没有/etc/codex/network-policy.json，也无environment_status工具；不推断其他环境就绪状态。
IMAP和SMTP均完成真实TLSv1.3认证；已自发自收一封唯一标记的MIME邮件，作为首次导入验收。服务商登录后CAPABILITY没有MOVE、有UIDPLUS；垃圾箱API应返回email_imap_move_not_supported。
