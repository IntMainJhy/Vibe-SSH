# Vibe-SSH 方案总览

当前交付阶段为 macOS **公开测试版 0.2.0-beta.1**。从[发布说明](releases/0.2.0-beta.1.md)查看安装包、支持范围和已知限制，从[开发与实现状态](06-development-and-status.md)查阅分阶段验收。

本方案将 Vibe-SSH 定义为可由外部 Agent 高效操作的桌面 SSH 客户端。服务器由用户手动添加，操作在客户端内核中统一执行，客户端提供真实输出和可执行的接管动作。

“企业级项目”的要求落实为明确的模块边界、版本化契约、可追溯决策、故障语义、测试证据和发布门槛。企业多租户、集中管控和高可用服务器不属于首版范围。

## 阅读顺序

| 文档 | 回答的问题 |
| --- | --- |
| [产品与首版范围](01-product-and-scope.md) | 为谁解决什么问题，首版包含什么 |
| [系统架构](02-system-architecture.md) | 各进程、模块、数据和会话如何协作 |
| [外部 AI 协议](03-agent-protocol.md) | Agent 如何发现能力、执行、等待与恢复观察 |
| [安全与可靠性](04-security-and-reliability.md) | 凭据、授权、接管与故障如何处理 |
| [交付与验收](05-delivery-and-acceptance.md) | 如何分阶段开发并证明可用 |
| [发布流程](releases/README.md) | Beta 版本、构建、镜像和官网发布 |
| [开发与实现状态](06-development-and-status.md) | 当前代码能做什么，如何复现验证 |
| [正式版开发计划](07-stable-release-plan.md) | 传统 SSH、Agent 验收、性能与正式交付的开发顺序 |
| [Codex 静态站点部署对照](performance/2026-11-18-agent-deployment.md) | 独立部署、故障恢复和端到端计时 |
| [架构决策 0010](adr/0010-openssh-import-and-key-agent.md) | OpenSSH 预览导入与本地密钥代理认证 |
| [架构决策 0011](adr/0011-jump-host-and-local-forward.md) | 跳板机链路与个人会话固定、动态转发 |
| [架构决策 0012](adr/0012-folder-transfer.md) | 有界递归文件夹传输与取消语义 |
| [架构决策 0013](adr/0013-cli-batch.md) | 逐步校验同一租约的批量计划 |
| [架构决策 0009](adr/0009-agent-resource-reads.md) | Agent 的目录、文本与 Linux 资源读取 |
| [架构决策 0008](adr/0008-host-access-revocation.md) | 主机授权撤销、活动任务与旧租约的处理 |
| [架构决策 0001](adr/0001-runtime-and-reuse.md) | 为什么选 Rust/Tauri，如何复用开源组件 |
| [安装说明](installation.md) | 分发包、自动接入、签名与卸载边界 |
| [架构决策 0007](adr/0007-task-history.md) | 有界 SQLite 任务记录、输出尾部与重启语义 |
| [架构决策 0006](adr/0006-directory-diff-preview.md) | 只读清单、选择性哈希、忽略与预算 |
| [架构决策 0014](adr/0014-reviewed-directory-updates.md) | 桌面确认目录差异、版本检查、备份和分项回执 |
| [架构决策 0015](adr/0015-resumable-uploads.md) | 单文件上传恢复、临时文件校验及发布限制 |
| [架构决策 0005](adr/0005-native-acceptance-isolation.md) | 原生窗口与临时 Keychain 验收隔离 |
| [English installation](installation.en.md) | 英文安装、Agent 接入、升级与卸载 |
| [架构决策 0004](adr/0004-remote-text-editing.md) | 有界远端文本编辑、备份、版本与控制代次检查 |
| [架构决策 0003](adr/0003-desktop-files-and-onboarding.md) | 完整 SSH 工作台、自动 Skill 配置、SFTP 与监控 |
| [架构决策 0002](adr/0002-task-spaces-and-agent-neutrality.md) | 任务空间、每空间一条连接、Agent 中立、里程碑调整 |

## 已确定的决定

1. 用户像使用 FinalShell 一样手动填写服务器资料。
2. Vibe-SSH 是日常 SSH 客户端，外部 Agent 通过 Skill 与 CLI 操作，包含日常终端、文件和资源监控；不提供多 Agent 调度或管理界面。
3. Rust 内核统一管理 SSH 连接、任务、输出和控制权。
4. UI 使用 Tauri 2、React/TypeScript 和 xterm.js。
5. 优先复用协议库与终端组件，不整体 fork 一个完整 SSH 客户端。
6. 首版先验证 Linux/POSIX 远端；桌面目标为 macOS。
7. 开发使用本地临时 SSH 服务验证，不依赖真实生产凭据。
8. Agent 的每个目标是一个任务空间，拥有独立连接和控制权；用户自己的终端对 Agent 不可见。
9. Vibe-SSH 自有代码采用 [MIT 协议](../LICENSE)，第三方组件保留各自许可。

## 开源协作入口

[English README](../README.md)、[中文首页](../README.zh-CN.md)、[贡献指南](../CONTRIBUTING.md)、[安全策略](../SECURITY.md)、[支持说明](../SUPPORT.md)、[路线图](../ROADMAP.md)与[更新日志](../CHANGELOG.md)面向使用者和贡献者。仓库简介、Topics 与维护约定见[仓库维护说明](maintainers/repository.md)。

## 决策管理

影响模块边界、持久数据、权限语义或协议兼容性的变更，先补充架构决策记录。普通实现修正和 UI 微调无需新增决策文档。

需求标识使用 FR，质量要求使用 NFR，验收使用 AC。编号只服务于需求、实现和测试之间的映射，不用于制造额外流程。

## 当前实现边界

本目录记录完整首版设计。具体可用命令以 CLI 帮助和 `vibe-ssh skill` 为准；各里程碑功能在验收前均属于计划。已完成的切片与仍缺少的证据见开发与实现状态。系统架构文档中与 ADR 0002 不一致的描述以 ADR 0002 为准。
