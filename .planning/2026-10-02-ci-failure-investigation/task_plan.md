# GitHub CI 失败检查

用户要求检查 GitHub CI 失败，并明确要求本地执行 CI 所需脚本验证。独立计划目录，不修改已完成事件系统的历史验收记录。

## 阶段一：证据与环境
Status: complete
读取失败 run/job/step、可用日志与 annotations；对照实际 workflow 和本地工具链。CI 失败是已知事实，不重跑远程 CI 确认。

## 阶段二：本地同等入口验证
Status: complete
执行 workflow 的 Rust 门禁及 live parity；用独立指定工具链核对 stable 版本差异。保持 warnings-as-errors，不放宽门禁，不改全局默认工具链。

## 阶段三：原因与结果
Status: complete
报告已证实原因、文件/诊断及验证范围；若不能获得确切 CI 诊断，明确区分本地复现与 CI 原始日志。

## 阶段四：修复弃用原子 API
Status: complete
用户已要求修复。仅把测试 helper 的条件原子峰值更新改为 fetch_max，保持 AcqRel；验证 Rust1.99.0 严格 Clippy、既有 MCP 测试和原子峰值冒烟，兼容1.98.1，不降低 CI 门禁。验证后提交本阶段源码变更。
