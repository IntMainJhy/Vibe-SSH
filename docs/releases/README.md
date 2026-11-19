# 发版流程

当前版本采用 `0.2.0-beta.N`，GitHub Releases 标为预发行。公开测试用于收集实际连接、安装和 Agent 使用反馈。稳定版证据记录在[正式版计划](../07-stable-release-plan.md)。

1. 同步 Cargo 工作区和锁文件、桌面 `package.json` 和锁文件、Tauri 配置、样例版本和官网。为版本准备本目录的发布说明、`CHANGELOG.md`、中英文安装文档，并核对内置 Skill 与应用协议。现有 CLI 协议为 v2。
2. 按[贡献指南](../../CONTRIBUTING.md#local-development)执行前端测试与构建、Rust 格式、Clippy、测试。SSH 验证使用临时回环服务或隔离 Debian。新出现的失败回归应在打标签前修复。
3. 构建 Apple 芯片和 Intel 两种应用，校验应用签名。用 `scripts/package-release.py --app <Vibe-SSH.app>` 生成 DMG 和 ZIP，脚本从应用二进制识别架构。隔离应用资料验证启动、Skill 与 CLI 版本、从上个版本升级后的配置和历史。记录物理 Intel 与 Rosetta 的差别。
4. 把准确对应安装包的提交和 `v<version>` 标签同步到 GitHub 和 AtomGit。GitHub 的发行工作流在两种 macOS runner 上验证和打包。成功后建立预发行，上传四个安装包和同一份 `SHA256SUMS.txt`。如需人工发布，从准确的标签构建，验证安装后再上传；保留同一个标签与构建来源。AtomGit 发行版上传相同的安装包与校验清单。
5. 验证两处公开发布页中的架构、文件、哈希和预发行状态。然后按[官网说明](../../site/README.md)更新 GitHub Pages，实际访问页面并核对下载和反馈入口。
