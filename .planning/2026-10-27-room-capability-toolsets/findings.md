# 已确认问题
- 一个Room namespace囊括七个子域，Email/Secret配置enabled重复，并且发现过滤规则不一致。
- RoomSecret配置藏在Room长页且整个section随room工具集禁用；Email配置可见但后台被room总开关阻断。
- Additional混放Browser/Skills/命令策略/配置管理；需要按领域分组。
- Bootstrap仅使用workspace/bootstrap，不需Room Git仓库。
- 变更触及Config/Serde、CLI/TUI、MCP descriptors/admission、运行时重配置、测试夹具及文档，API名称/Hub路由保持。
