![Vibe-SSH — AI 操作，随时接管](docs/assets/vibe-ssh-hero.png)

<h1 align="center">Vibe-SSH</h1>

<h3 align="center">为你和外部 AI 打造的 SSH 工作空间</h3>

<p align="center">
  <a href="LICENSE"><img src="docs/assets/license-mit.svg" alt="MIT 开源协议" width="115" height="28" /></a>
  <a href="https://github.com/IntMainJhy/Vibe-SSH/releases"><img src="docs/assets/release-downloads.svg" alt="下载发行版" width="184" height="28" /></a>
  <a href="https://github.com/IntMainJhy/Vibe-SSH/actions/workflows/rust.yml"><img src="docs/assets/ci-status.svg" alt="查看 CI 实时状态" width="153" height="28" /></a>
  <a href="docs/installation.md"><img src="docs/assets/platform-macos.svg" alt="macOS 桌面客户端" width="148" height="28" /></a>
</p>

<p align="center">
  开源的 macOS SSH/SFTP 客户端，提供 <strong>Skill 与 CLI 接入外部 Agent</strong>。<br />
  手动添加自己的服务器，通过 Skill 完成网站部署与故障排查。<br />
  <strong>命令与输出实时可见，任务空间随时接管。</strong>
</p>

<h3 align="center"><a href="https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-beta.1">下载 macOS 个人版 Beta</a></h3>

<p align="center">
  <a href="https://vibe-ssh.intmainjhy.cn/">产品官网</a> &nbsp;·&nbsp;
  <a href="#可以做什么">功能概览</a> &nbsp;·&nbsp;
  <a href="docs/installation.md">安装说明</a> &nbsp;·&nbsp;
  <a href="skills/vibe-ssh/guide.md">Agent 指南</a> &nbsp;·&nbsp;
  <a href="docs/README.md">技术文档</a> &nbsp;·&nbsp;
  <a href="ROADMAP.md">路线图</a> &nbsp;·&nbsp;
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

<p align="center">
  <a href="README.md" lang="en">English</a> · <strong>简体中文</strong>
  &nbsp;|&nbsp;
  <a href="https://github.com/IntMainJhy/Vibe-SSH">GitHub 主仓库</a> · <a href="https://gitcode.com/InMainJhy/Vibe-SSH">AtomGit</a>
</p>

<p align="center">你的日常终端与 Agent 任务空间使用独立 SSH 连接，可以并行工作。</p>

> **个人版 Beta**。 提供 Apple 芯片与 Intel Mac 安装包。Linux 是远端服务器目标，目前没有 Windows/Linux 桌面包。应用使用临时签名，尚未完成 Developer ID 签名与 Apple 公证。能力与验证范围见下文。

![Vibe-SSH 工作台：服务器导航、会话与 Agent 活动入口](docs/assets/workspace.png)

*截图来自实际应用界面的开发预览，使用示例服务器、地址和任务数据。*

## 产品版本

| 版本 | 发布状态 | 功能 |
| --- | --- | --- |
| **个人版 Beta** | 已发布，免费下载，全部源码采用 MIT 许可 | SSH、SFTP、端口转发、监控、Agent Skill 和 CLI、批量部署、历史记录、日志和人工接管。 |
| **企业版 Pro** | 付费版本，开发中 | 为重复交付设计的版本化发布模板和 Markdown 交付报告。 |

两个版本的服务器凭据都留在本机。本仓库维护完整的个人版源码。详见[企业版 Pro 开发文档](docs/08-enterprise-pro.md)。

## 可以做什么

| 能力 | 使用方式 |
| --- | --- |
| 日常 SSH | 手动填写服务器信息、核对指纹，使用多标签终端、搜索、外观切换与字号设置。 |
| 外部 AI 操作 | 逐台开启 Agent 接入，Agent 读取 Skill 后通过版本匹配的 CLI 操作。 |
| 观看执行过程 | 打开 Agent 空间，查看指令、任务状态和终端输出。 |
| 人工接管 | 接管后内核拒绝空间里的新 Agent 写操作；归还生成新租约，旧租约保持失效。 |
| 文件传输与编辑 | 浏览 SFTP 根目录和主目录，续传文件和文件夹，编辑有大小限制的 UTF-8 文本，带版本检查和备份。 |
| 连接与转发 | 预览导入 OpenSSH 配置，使用密钥代理、跳板机、网络代理和个人会话端口转发。 |
| 目录比较与历史 | 核对目录差异，再应用选中的新增或替换。历史可查看传输、部署步骤与归档日志。 |
| Linux 资源监控 | 查看 CPU、内存、交换空间、负载、运行时间和根分区用量，监控页可见时采样。 |
| 网站部署 | 使用静态站点/Nginx 与 Docker Compose 参考流程，包含验证和失败处理。 |

Vibe-SSH 把你已有的 AI Agent 接入 SSH 工作流。客户端本身不运行模型，也不替 Agent 作出自主部署决策。

## 安装与首次连接

1. 在 [Releases](https://github.com/IntMainJhy/Vibe-SSH/releases) 选择安装包：Apple 芯片选 `macos-arm64`，Intel 选 `macos-x86_64`。每次发布附 DMG、ZIP 和 `SHA256SUMS.txt`。
2. 把 `Vibe-SSH.app` 移入“应用程序”后启动。校验和与 macOS 单次允许打开的步骤见[安装说明](docs/installation.md)。
3. 手动输入服务器名称、地址、端口、用户名、密码或私钥，通过可信渠道核对指纹后保存。
4. 打开你自己的终端。需要 AI 操作时，再为这台主机开启 Agent 接入。

应用启动及升级时会同步兼容 Agent 的 Skill 链接，并创建 `~/.local/bin/vibe-ssh` 命令链接；无关的已有同名内容会保留。详细路径、移动应用和卸载行为见安装说明。

## 让外部 Agent 使用

保持 Vibe-SSH 运行，在 Agent 对话里说明服务器和具体目标，例如：

> 通过 Vibe-SSH 把这个项目构建后的站点部署到我的博客服务器。先检查现有配置，保留当前可用版本，部署后验证网站地址。

Agent 首先读取与已安装应用匹配的说明：

```sh
vibe-ssh skill
# PATH 没有包含命令目录时：
"$HOME/.local/bin/vibe-ssh" skill
```

说明包含授权主机列表、每个目标的任务空间、执行指令、增量输出、上传与交接。能读取 Skill 并执行本机命令行的兼容 Agent 可以使用这些接口；每种 Agent 的独立部署验证仍需记录具体证据。

当前开发版的静态站点 Skill 支持用一次 `batch` 调用完成多个命令和上传，桌面端逐步显示任务与输出。步骤失败、结果未知或用户接管后，后续步骤停止提交。 [完整部署对照](docs/performance/2026-11-18-agent-deployment.md)

<details>
<summary>命令行调用示例</summary>

下方主机 ID 与租约来自前一步 JSON 返回值，日常使用时由 Agent 管理。

```sh
vibe-ssh hosts
vibe-ssh space open <host-id> --name "检查部署环境" --actor "Codex"
vibe-ssh run --lease <lease> -- 'uname -a'
```

长任务返回游标，通过 `job wait` 继续等待。等待到期不代表任务失败。操作前阅读[完整指南](skills/vibe-ssh/guide.md)。

</details>

部署参考流程：

```sh
vibe-ssh skill static-site   # 构建后的静态站点，通过 Nginx 提供服务
vibe-ssh skill compose       # 已有 Docker Compose 的项目
```

## 观看、接管与归还

Agent 的指令、终端和控制权归属于任务空间。你可以在自己的终端里照常工作，同时观察 Agent 空间。

接管后，内核拒绝这个空间里的后续 Agent 指令、输入、上传、中断与关闭操作。已运行的命令继续执行，直到完成或由你明确中断。Agent 遇到密码或需要人作决定时，也可以主动交接。

归还会产生新租约，Agent 必须读取最新状态后继续。你的个人终端不向 Agent 暴露；添加主机、信任指纹、开启接入和人工接管不属于 Agent 套接字接口。

![Vibe-SSH 会话视图：任务输出与人工接管入口](docs/assets/session.png)

*会话截图使用应用自带的开发示例数据。*

## 当前状态与边界

macOS [公开测试版](https://github.com/IntMainJhy/Vibe-SSH/releases/tag/v0.2.0-beta.1) 为 **0.2.0-beta.1**。客户端包含个人终端、隔离的 Agent 任务空间、SFTP 续传、Linux 监控，以及版本匹配的 Skill 和 CLI。

桌面界面当前使用简体中文，CLI 指南与社区文档提供英文入口。

已有工程记录覆盖临时 OpenSSH 服务和隔离 Debian 13 虚拟机上的执行、上传、中断、接管、密码交接与两种部署流程。这些证据有明确范围，不代表每种 Agent 都已独立完成部署，也不代表已普遍验证公网服务器。

Beta 保留任务摘要与输出尾部。新任务每路另保存最多 64 MiB 的原始输出，归档总配额为 512 MiB。历史会标出归档不完整的记录，个人历史仅在桌面端可见。详见 [ADR 0007](docs/adr/0007-task-history.md)。

原生验收覆盖主机录入和指纹核对、中文终端、SFTP 传输、文本保存、接管归还和退出保护。加密私钥口令在隔离的 macOS 钥匙串中通过重启连接验证。

各功能的原生和回环验收范围见开发记录。稳定版仍需干净 macOS 系统和物理 Intel 设备验收、Developer ID 签名与 Apple 公证。

[独立 Agent 验收](docs/acceptance/2026-11-18-install-and-agent.md)中，Codex 使用已安装的 Skill 完成静态站点部署、Nginx 配置故障恢复和人工交接。Claude Code 已发现 Skill，配置的模型网关未能连接，部署待复验。

详见[开发与验证记录](docs/06-development-and-status.md)、[路线图](ROADMAP.md)和[更新日志](CHANGELOG.md)。

## 技术架构

```mermaid
flowchart LR
    Agent[Codex / Claude Code / 外部 Agent] --> Skill[版本匹配的 Skill]
    Skill --> CLI[vibe-ssh CLI]
    CLI --> Socket[当前用户专属 Unix 套接字]
    Socket --> Core[桌面应用内的 Rust 内核]
    UI[Tauri + React + xterm.js] --> Core
    Core --> AgentSpace[Agent 空间 / 独立 SSH 连接]
    Core --> Personal[个人终端 / 独立 SSH 连接]
    AgentSpace --> Server[你的服务器]
    Personal --> Server
```

Rust/Tokio 和 [russh](https://github.com/Eugeny/russh) 负责 SSH，Tauri 2、React/TypeScript 和 [xterm.js](https://github.com/xtermjs/xterm.js) 构成界面。连接只在同一任务空间内复用。Agent 使用协议 v2，人工管理操作留在应用进程内部。

决策与故障语义见[任务空间 ADR](docs/adr/0002-task-spaces-and-agent-neutrality.md)和[协议设计](docs/03-agent-protocol.md)。

## 从源码构建

在 macOS 上安装 Rust **1.89 或更新版本**、Node.js **24**、Xcode Command Line Tools，并满足 [Tauri 的前置要求](https://v2.tauri.app/start/prerequisites/)。

```sh
git clone https://github.com/IntMainJhy/Vibe-SSH.git
cd Vibe-SSH

cd apps/desktop
npm ci
npm test
npm run build
cd ../..

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

cd apps/desktop
npx tauri build
```

桌面 crate 在编译时嵌入前端产物，因此先构建前端，再检查 Rust 工作区。原生端到端测试使用 macOS 的 `/usr/sbin/sshd` 和临时目录、临时密钥。

隔离 Linux 测试虚拟机和部署样例见[贡献指南](CONTRIBUTING.md)。浏览器开发预览只使用示例数据，不连接服务器。

## 常见问题

**需要另外配置模型吗？**

Vibe-SSH 通过 Skill 和 CLI 接入外部 Agent，继续使用已有的 Agent 环境。

**可以单纯当 SSH 工具使用吗？**

可以。终端、文件和监控由你直接使用，主机是否开放给 Agent 单独设置。

**服务器需要安装 Vibe-SSH 服务吗？**

客户端通过 SSH 连接；操作仍需要正常的远端工具，例如上传使用的 `tar`，以及部署需要的 Nginx 或 Docker。

**一定比传统 SSH 工具快吗？**

设计重点是减少重复配置，让 Agent 直接获取输出与状态。[回环基准](docs/performance/2026-11-17-loopback.json)有明确范围，不能推导公网或完整部署的一般提速结论。

**支持 Windows 或 Linux 桌面吗？**

当前安装包只面向 macOS，Linux 用作远端服务器目标。其他桌面平台见[路线图](ROADMAP.md)。

## 社区与文档

- [报告问题](https://github.com/IntMainJhy/Vibe-SSH/issues/new/choose)，提供版本和复现步骤。
- [提问或分享流程](https://github.com/IntMainJhy/Vibe-SSH/discussions)。
- 阅读[贡献指南](CONTRIBUTING.md)、[行为准则](CODE_OF_CONDUCT.md)与[支持说明](SUPPORT.md)。
- 安全问题按[安全策略](SECURITY.md)私下报告。
- 技术细节见[方案总览](docs/README.md)、[界面规范](DESIGN.md)与[Agent Skill](skills/vibe-ssh/SKILL.md)。

## 开源协议

Vibe-SSH 采用 [MIT License](LICENSE)。第三方组件保留各自许可证，发行包单独维护[第三方声明](third_party/licenses/README.md)。
