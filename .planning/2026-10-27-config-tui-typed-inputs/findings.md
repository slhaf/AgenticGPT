# 发现
- 五个枚举：HubTransport、Email IMAP/SMTP TLS、sendPolicy、trashPolicy 前置页文本而 Review 选择。
- numeric_field 仅五项，遗漏 HTTP 端口、邮件数字、事件TTL、Secret等待及Catalog整数；数字组件固定5字符宽不适合容量。
- allowHosts 在两个入口均需手写JSON，应用语义包含null、["*"]、自定义非空列表。
- Email序列化错误统一投影到EmailEnabled，格式帮助缺失。
- 引用应使用来源+路径/变量名，保持引用值；真正密码字段不误作引用。
- 独立审查指出引用字段必须保留“未设置/清空”，已补Unset和C清空。
- 结构化编辑器Apply失败必须回滚前置草稿并保留编辑器内部输入，避免Esc仍遗留非法值；已统一。
- 列表导航按可见模式和focus索引处理，避免Tab选中行与被修改行错位、Apply误触隐藏列表。
